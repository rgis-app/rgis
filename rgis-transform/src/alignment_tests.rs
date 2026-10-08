//! Exhaustive raster/vector registration tests.
//!
//! For each scenario, a dense set of lon/lat sample points goes through the
//! vector reprojection pipeline, and a synthetic raster goes through the
//! raster grid pipeline. Each projected sample is then located inside the
//! raster mesh exactly the way the GPU would (same triangles, barycentric UV
//! interpolation), the interpolated UV is mapped back to a ground position,
//! and that is compared against the sample itself. Any disagreement means a
//! vector drawn at that location would not sit on top of the raster texel
//! for the same location.

use geo::{Distance, Haversine};
use geo_projected::CastTo;
use geodesy::prelude::Context;

use crate::jobs::{
    clamp_to_area_of_use, project_feature, project_raster_grid, MAX_GRID_ERROR_TEXELS,
};

fn crs(ctx: &mut geodesy::ctx::Minimal, code: u16) -> rgis_primitives::Crs {
    rgis_primitives::Crs {
        epsg_code: Some(code),
        proj_string: None,
        op_handle: rgis_crs::epsg_code_to_geodesy_op_handle(ctx, code)
            .unwrap_or_else(|e| panic!("EPSG:{code}: {e}")),
    }
}

fn rect(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> geo::Rect<f64> {
    geo::Rect::new(
        geo::coord! { x: min_x, y: min_y },
        geo::coord! { x: max_x, y: max_y },
    )
}

/// Point-in-mesh lookup for a [`rgis_layers::ProjectedRasterGrid`], using
/// the same triangles the renderer draws.
struct MeshLocator<'a> {
    grid: &'a rgis_layers::ProjectedRasterGrid,
    triangles: Vec<[usize; 3]>,
    /// Triangle indices, bucketed on a `BUCKETS × BUCKETS` lattice over the
    /// grid's extent.
    buckets: Vec<Vec<usize>>,
}

const BUCKETS: usize = 256;

impl<'a> MeshLocator<'a> {
    fn new(grid: &'a rgis_layers::ProjectedRasterGrid) -> Self {
        let triangles: Vec<_> = grid.triangles().collect();
        let mut locator = MeshLocator { grid, triangles, buckets: vec![Vec::new(); BUCKETS * BUCKETS] };
        for t in 0..locator.triangles.len() {
            let (min, max) = locator.bbox(t);
            let (c0, r0) = locator.bucket(min);
            let (c1, r1) = locator.bucket(max);
            for r in r0..=r1 {
                for c in c0..=c1 {
                    locator.buckets[r * BUCKETS + c].push(t);
                }
            }
        }
        locator
    }

    fn pos(&self, i: usize) -> geo::Coord<f64> {
        let [x, y] = self.grid.positions[i];
        geo::coord! { x: f64::from(x), y: f64::from(y) }
    }

    fn bbox(&self, t: usize) -> (geo::Coord<f64>, geo::Coord<f64>) {
        let [a, b, c] = self.triangles[t].map(|i| self.pos(i));
        (
            geo::coord! { x: a.x.min(b.x).min(c.x), y: a.y.min(b.y).min(c.y) },
            geo::coord! { x: a.x.max(b.x).max(c.x), y: a.y.max(b.y).max(c.y) },
        )
    }

    fn bucket(&self, p: geo::Coord<f64>) -> (usize, usize) {
        let e = self.grid.extent;
        let f = |v: f64, lo: f64, span: f64| {
            (((v - lo) / span * BUCKETS as f64).floor().max(0.0) as usize).min(BUCKETS - 1)
        };
        (f(p.x, e.min().x, e.width()), f(p.y, e.min().y, e.height()))
    }

    /// The UV the rendered raster mesh would show at `p`, and whether `p`
    /// is actually inside the mesh. If it isn't, the UV is extrapolated from
    /// the nearest triangle (within a few bucket cells), so a caller can
    /// check that `p` is merely just past the mesh's edge. The mesh's edges
    /// are chords of the raster's (possibly curved) projected boundary, so
    /// points on the boundary can land slightly outside.
    fn uv_at(&self, p: geo::Coord<f64>) -> Option<([f64; 2], bool)> {
        const EPS: f64 = 1e-6;
        let (bc, br) = self.bucket(p);
        // (min barycentric weight, uv) of the best candidate so far.
        let mut best: Option<(f64, [f64; 2])> = None;
        for r in br.saturating_sub(1)..=(br + 1).min(BUCKETS - 1) {
            for c in bc.saturating_sub(1)..=(bc + 1).min(BUCKETS - 1) {
                for &t in &self.buckets[r * BUCKETS + c] {
                    let [ia, ib, ic] = self.triangles[t];
                    let [a, b, c] = [ia, ib, ic].map(|i| self.pos(i));
                    let det = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
                    if det.abs() < f64::MIN_POSITIVE {
                        continue;
                    }
                    let wa = ((b.y - c.y) * (p.x - c.x) + (c.x - b.x) * (p.y - c.y)) / det;
                    let wb = ((c.y - a.y) * (p.x - c.x) + (a.x - c.x) * (p.y - c.y)) / det;
                    let wc = 1.0 - wa - wb;
                    let min_w = wa.min(wb).min(wc);
                    if best.is_some_and(|(w, _)| w >= min_w) {
                        continue;
                    }
                    let uv = |i: usize, k: usize| f64::from(self.grid.uvs[i][k]);
                    let uv = [
                        wa * uv(ia, 0) + wb * uv(ib, 0) + wc * uv(ic, 0),
                        wa * uv(ia, 1) + wb * uv(ib, 1) + wc * uv(ic, 1),
                    ];
                    if min_w >= -EPS {
                        return Some((uv, true));
                    }
                    best = Some((min_w, uv));
                }
            }
        }
        best.map(|(_, uv)| (uv, false))
    }
}

struct Scenario {
    name: &'static str,
    raster_crs: u16,
    /// Raster extent in `raster_crs`.
    raster_extent: geo::Rect<f64>,
    raster_size_px: [u32; 2],
    /// Lon/lat region to sample, stepped by `step` degrees.
    samples: geo::Rect<f64>,
    step: f64,
}

#[derive(Default)]
struct Report {
    grid: (u32, u32),
    checked: usize,
    /// Samples just outside the mesh, but within tolerance of its edge.
    outside: usize,
    missing: Vec<geo::Coord<f64>>,
    max_err_texels: f64,
    max_err_m: f64,
    worst: Option<geo::Coord<f64>>,
}

fn run(
    ctx: &mut geodesy::ctx::Minimal,
    scenario: &Scenario,
    target_code: u16,
) -> Report {
    let wgs84 = crs(ctx, 4326);
    let raster_crs = crs(ctx, scenario.raster_crs);
    let target = crs(ctx, target_code);
    let area = target.area_of_use();

    let to_raster = rgis_crs::CrsTransformer::new(&*ctx, &wgs84, &raster_crs);
    let from_raster = rgis_crs::CrsTransformer::new(&*ctx, &raster_crs, &wgs84);

    // Lon/lat samples that lie inside both the raster and the target CRS's
    // area of use (outside it, both pipelines clamp, by design).
    let mut samples = Vec::new();
    let s = scenario.samples;
    let nx = ((s.width() / scenario.step).round() as usize).max(1);
    let ny = ((s.height() / scenario.step).round() as usize).max(1);
    for j in 0..=ny {
        for i in 0..=nx {
            let lon = s.min().x + s.width() * i as f64 / nx as f64;
            let lat = s.min().y + s.height() * j as f64 / ny as f64;
            if let Some(a) = area {
                if lon < a.lon_west || lon > a.lon_east || lat < a.lat_south || lat > a.lat_north {
                    continue;
                }
            }
            let Ok(r) = to_raster.transform_coord(geo::coord! { x: lon, y: lat }) else {
                continue;
            };
            let e = scenario.raster_extent;
            if r.x < e.min().x || r.x > e.max().x || r.y < e.min().y || r.y > e.max().y {
                continue;
            }
            samples.push(geo::coord! { x: lon, y: lat });
        }
    }
    if samples.is_empty() {
        return Report::default();
    }

    // Vector pipeline: a 4326 layer containing every sample point.
    let multi_point: geo::MultiPoint<f64> = samples.iter().map(|&c| geo::Point(c)).collect();
    let fc = geo_features::FeatureCollection::from_geometry(multi_point.into());
    let mut fc: geo_features::FeatureCollection<geo_projected::UnprojectedScalar> =
        geo_projected::WrapTo::wrap(fc);
    clamp_to_area_of_use(&mut fc, &wgs84, &target);
    let mut fc = fc.cast::<geo_projected::Projected>();
    let transformer = rgis_crs::CrsTransformer::new(&*ctx, &wgs84, &target);
    for feature in &mut fc.features {
        project_feature(&transformer, feature).unwrap();
    }
    let Some(geo::Geometry::MultiPoint(projected)) = &fc.features[0].geometry else {
        panic!("expected multipoint");
    };

    // Raster pipeline.
    let grid = project_raster_grid(
        &*ctx,
        scenario.raster_extent,
        scenario.raster_size_px,
        &raster_crs,
        &target,
    );

    let locator = MeshLocator::new(&grid);
    let e = scenario.raster_extent;
    let [w_px, h_px] = scenario.raster_size_px.map(f64::from);
    let mut report = Report { grid: (grid.cols, grid.rows), ..Default::default() };
    for (sample, p) in samples.iter().zip(projected.iter()) {
        // Vector meshes are built from f32 positions, like the raster mesh.
        let p = geo::coord! { x: f64::from(p.x().0 as f32), y: f64::from(p.y().0 as f32) };
        let Some(([u, v], inside)) = locator.uv_at(p) else {
            report.missing.push(*sample);
            continue;
        };
        let expected = to_raster.transform_coord(*sample).unwrap();
        let expected_u = (expected.x - e.min().x) / e.width();
        let expected_v = (e.max().y - expected.y) / e.height();
        let err_texels = ((u - expected_u) * w_px).hypot((v - expected_v) * h_px);

        let shown = geo::coord! {
            x: e.min().x + u * e.width(),
            y: e.max().y - v * e.height(),
        };
        let shown = from_raster.transform_coord(shown).unwrap();
        let err_m = Haversine.distance(geo::Point(*sample), geo::Point(shown));

        if !inside {
            if err_texels > MAX_ERR_TEXELS {
                report.missing.push(*sample);
                continue;
            }
            report.outside += 1;
        }
        report.checked += 1;
        report.max_err_m = report.max_err_m.max(err_m);
        if err_texels > report.max_err_texels {
            report.max_err_texels = err_texels;
            report.worst = Some(*sample);
        }
    }
    report
}

const WEB_MERCATOR_HALF: f64 = 20_037_508.342_789_244;

fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "world 4326 raster, global samples",
            raster_crs: 4326,
            raster_extent: rect(-180.0, -90.0, 180.0, 90.0),
            raster_size_px: [21600, 10800],
            samples: rect(-180.0, -90.0, 180.0, 90.0),
            step: 2.5,
        },
        Scenario {
            name: "world 4326 raster, Great Lakes samples",
            raster_crs: 4326,
            raster_extent: rect(-180.0, -90.0, 180.0, 90.0),
            raster_size_px: [21600, 10800],
            samples: rect(-93.0, 41.0, -75.0, 49.0),
            step: 0.25,
        },
        Scenario {
            name: "Great Lakes 4326 raster",
            raster_crs: 4326,
            raster_extent: rect(-93.0, 40.0, -75.0, 50.0),
            raster_size_px: [1800, 1000],
            samples: rect(-93.0, 40.0, -75.0, 50.0),
            step: 0.25,
        },
        Scenario {
            name: "world 3857 raster",
            raster_crs: 3857,
            raster_extent: rect(
                -WEB_MERCATOR_HALF,
                -WEB_MERCATOR_HALF,
                WEB_MERCATOR_HALF,
                WEB_MERCATOR_HALF,
            ),
            raster_size_px: [8192, 8192],
            samples: rect(-180.0, -85.0, 180.0, 85.0),
            step: 2.5,
        },
        Scenario {
            name: "Lake Ontario UTM 18N raster",
            raster_crs: 32618,
            raster_extent: rect(150_000.0, 4_750_000.0, 450_000.0, 4_950_000.0),
            raster_size_px: [3000, 2000],
            samples: rect(-80.0, 42.8, -75.5, 44.8),
            step: 0.1,
        },
    ]
}

const TARGETS: &[u16] = &[4326, 3857, 3395, 32618, 3978, 3035];

/// Allowed raster/vector disagreement, in raster pixels. The grid is
/// refined to `MAX_GRID_ERROR_TEXELS` at edge midpoints; allow some slack
/// for the error elsewhere in a cell.
const MAX_ERR_TEXELS: f64 = 2.0 * MAX_GRID_ERROR_TEXELS;

#[test]
fn raster_and_vector_stay_registered() {
    let mut ctx = geodesy::ctx::Minimal::new();
    let mut failures = Vec::new();
    for scenario in scenarios() {
        for &target in TARGETS {
            let r = run(&mut ctx, &scenario, target);
            let line = format!(
                "{:<40} -> EPSG:{target:<5} grid {:>4}x{:<4}: checked {:>5} ({:>3} at edge), missing {:>4}, \
                 max err {:>6.3} px / {:>8.1} m at {:?}",
                scenario.name,
                r.grid.0,
                r.grid.1,
                r.checked,
                r.outside,
                r.missing.len(),
                r.max_err_texels,
                r.max_err_m,
                r.worst.map(|c| (c.x, c.y)),
            );
            println!("{line}");
            if !r.missing.is_empty() {
                println!(
                    "    missing: {:?}",
                    r.missing.iter().take(12).map(|c| (c.x, c.y)).collect::<Vec<_>>()
                );
            }
            if r.max_err_texels > MAX_ERR_TEXELS || !r.missing.is_empty() {
                failures.push(line);
            }
        }
    }
    assert!(failures.is_empty(), "misregistered:\n{}", failures.join("\n"));
}

/// Small, high-resolution rasters (drone/satellite imagery with cm-scale
/// pixels) must not drive grid refinement to its cap: over a few tens of
/// metres no projection curves measurably. Measuring the grid's error with
/// f32 positions once made them refine to 2^18 cells, hanging the web app.
#[test]
fn small_high_res_rasters_keep_a_coarse_grid() {
    let mut ctx = geodesy::ctx::Minimal::new();
    // Parameters of fixtures in geotiff-test-data/real_data.
    let cases = [
        // hot-oam/68077a72c46a9912474701ef.tif
        (32613, rect(246_690.034, 4_309_980.042, 246_739.982, 4_310_030.042), [999, 1000]),
        // vantor/maxar_opendata_yellowstone_visual.tif
        (32612, rect(529_843.75, 5_035_117.188, 529_882.812, 5_035_156.25), [128, 128]),
        // umbra/sydney_airport_GEC.tif (approximate, unrotated)
        (4326, rect(150.7539, -33.8896, 150.7564, -33.8884), [512, 512]),
    ];
    for (raster_code, extent, size) in cases {
        let raster_crs = crs(&mut ctx, raster_code);
        for &target in TARGETS {
            let target_crs = crs(&mut ctx, target);
            let grid = project_raster_grid(&ctx, extent, size, &raster_crs, &target_crs);
            assert!(
                grid.cols * grid.rows <= 64 * 64,
                "EPSG:{raster_code} -> EPSG:{target}: grid {}x{}",
                grid.cols,
                grid.rows,
            );
        }
    }
}
