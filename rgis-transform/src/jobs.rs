use std::sync::Arc;
use geo::MapCoords;
use geo_projected::CastTo;
use geodesy::prelude::Context;

pub struct ReprojectRasterExtentJob {
    pub extent: geo::Rect<f64>,
    /// Raster size in pixels, used to decide how fine the projected grid
    /// needs to be.
    pub width_px: u32,
    pub height_px: u32,
    pub layer_id: rgis_primitives::LayerId,
    pub source_crs: rgis_primitives::Crs,
    pub target_crs: rgis_primitives::Crs,
    pub geodesy_ctx: rgis_crs::GeodesyContext,
}

pub struct ReprojectRasterExtentJobOutcome {
    pub projected_grid: rgis_layers::ProjectedRasterGrid,
    pub layer_id: rgis_primitives::LayerId,
    pub target_crs: rgis_primitives::Crs,
}

impl bevy_jobs::Job for ReprojectRasterExtentJob {
    type Outcome = Result<ReprojectRasterExtentJobOutcome, rgis_crs::TransformError>;

    fn name(&self) -> String {
        "Projecting raster extent".to_string()
    }

    async fn perform(self, _progress_sender: bevy_jobs::Context) -> Self::Outcome {
        let geodesy_ctx = self.geodesy_ctx.read().unwrap();
        let projected_grid = project_raster_grid(
            &*geodesy_ctx,
            self.extent,
            [self.width_px, self.height_px],
            &self.source_crs,
            &self.target_crs,
        );
        Ok(ReprojectRasterExtentJobOutcome {
            projected_grid,
            layer_id: self.layer_id,
            target_crs: self.target_crs,
        })
    }
}

/// Initial grid cells per side.
const MIN_GRID: u32 = 32;
/// Max grid cells per side.
const MAX_GRID: u32 = 2048;
/// Max total grid cells, to bound vertex count and projection work.
const MAX_GRID_CELLS: u32 = 1 << 18;
/// Within a grid cell the texture is mapped affinely, while the true
/// projection is curved. Keep refining the grid until that discrepancy,
/// measured at cell edge midpoints, is below this many texels.
pub(crate) const MAX_GRID_ERROR_TEXELS: f64 = 0.25;

/// Project a raster's footprint into `target_crs` as a textured grid.
///
/// The raster's extent (in `source_crs`) is sampled on a lattice and every
/// sample is projected. Each vertex's UV is derived from the source
/// coordinate it was sampled at, so the texture stays registered with the
/// projected positions even when only part of the raster is sampled (e.g.
/// clamped to the target CRS's area of use). The lattice is refined, per
/// axis, until affine interpolation within each cell is accurate to
/// [`MAX_GRID_ERROR_TEXELS`].
pub(crate) fn project_raster_grid<C: Context>(
    ctx: &C,
    raster_extent: geo::Rect<f64>,
    raster_size_px: [u32; 2],
    source_crs: &rgis_primitives::Crs,
    target_crs: &rgis_primitives::Crs,
) -> rgis_layers::ProjectedRasterGrid {
    let mut sample_extent = raster_extent;
    let area = target_crs.area_of_use();

    // Clamp geographic source coordinates to the target CRS's area of
    // use. Many projections have singularities outside their valid area
    // (e.g. Mercator Y → ∞ at the poles), so clamping prevents extreme
    // values from stretching the projected extent. This matches the
    // clamping applied to vector layers.
    if source_crs.is_geographic() {
        if let Some(area) = area {
            let (min, max) = (raster_extent.min(), raster_extent.max());
            let clamped_min_x = min.x.max(area.lon_west);
            let clamped_min_y = min.y.max(area.lat_south);
            let clamped_max_x = max.x.min(area.lon_east);
            let clamped_max_y = max.y.min(area.lat_north);
            // Only apply clamping if the extent still has positive area.
            // An inverted extent (min > max) means the source is entirely
            // outside the target CRS's valid area — e.g. projected meter
            // coordinates mistakenly treated as degrees — so skip clamping.
            if clamped_min_x <= clamped_max_x && clamped_min_y <= clamped_max_y {
                sample_extent = geo::Rect::new(
                    geo::coord! { x: clamped_min_x, y: clamped_min_y },
                    geo::coord! { x: clamped_max_x, y: clamped_max_y },
                );
            }
        }
    }

    // A projected source can't be clamped exactly (its edges don't follow
    // lines of longitude/latitude), and vector layers in a projected CRS
    // aren't clamped either. But don't spend the grid on parts of the raster
    // far outside the target's area of use, where the projection may fold
    // over or blow up: sample only the part that covers a generously
    // expanded area of use.
    if !source_crs.is_geographic() {
        if let Some(area) = area {
            let area = area.expanded(AREA_OF_USE_MARGIN);
            if let Some(bbox) = area_of_use_in_crs(ctx, source_crs, &area) {
                if let Some(clipped) = intersect(sample_extent, bbox) {
                    sample_extent = clipped;
                }
            }
        }
    }

    let sampler = GridSampler {
        transformer: rgis_crs::CrsTransformer::new(ctx, source_crs, target_crs),
        raster_extent,
        raster_size_px,
        sample_extent,
    };

    let (mut cols, mut rows) = (MIN_GRID, MIN_GRID);
    loop {
        let grid = sampler.sample(cols, rows);
        let [err_x, err_y, err_diagonal] = sampler.max_error_texels(&grid);
        let can_refine_x = cols < MAX_GRID && cols * 2 * rows <= MAX_GRID_CELLS;
        let can_refine_y = rows < MAX_GRID && cols * rows * 2 <= MAX_GRID_CELLS;
        // Diagonal error usually comes from curvature along one axis, and
        // refining that axis fixes both. Only if neither axis needs refining
        // is it a cross-axis effect that calls for refining both.
        let diagonal_only = err_x <= MAX_GRID_ERROR_TEXELS
            && err_y <= MAX_GRID_ERROR_TEXELS
            && err_diagonal > MAX_GRID_ERROR_TEXELS;
        let mut refine_x = (err_x > MAX_GRID_ERROR_TEXELS || diagonal_only) && can_refine_x;
        let mut refine_y = (err_y > MAX_GRID_ERROR_TEXELS || diagonal_only) && can_refine_y;
        if refine_x && refine_y && cols * rows * 4 > MAX_GRID_CELLS {
            // Only room to refine one axis; pick the worse one.
            refine_x = err_x >= err_y;
            refine_y = !refine_x;
        }
        if !refine_x && !refine_y {
            return grid;
        }
        if refine_x {
            cols *= 2;
        }
        if refine_y {
            rows *= 2;
        }
    }
}

struct GridSampler<'a, C: Context> {
    transformer: rgis_crs::CrsTransformer<'a, C>,
    raster_extent: geo::Rect<f64>,
    raster_size_px: [u32; 2],
    sample_extent: geo::Rect<f64>,
}

impl<C: Context> GridSampler<'_, C> {
    /// Source-CRS coordinate at fractional grid position `(col, row)`.
    fn source_coord(&self, col: f64, row: f64, cols: u32, rows: u32) -> geo::Coord<f64> {
        let e = self.sample_extent;
        geo::coord! {
            x: e.min().x + col / f64::from(cols) * e.width(),
            y: e.min().y + row / f64::from(rows) * e.height(),
        }
    }

    fn project(&self, src: geo::Coord<f64>) -> Option<geo::Coord<f64>> {
        let out = self.transformer.transform_coord(src).ok()?;
        (out.x.is_finite() && out.y.is_finite()).then_some(out)
    }

    fn sample(&self, cols: u32, rows: u32) -> rgis_layers::ProjectedRasterGrid {
        let num_verts = ((cols + 1) * (rows + 1)) as usize;
        let mut positions = Vec::with_capacity(num_verts);
        let mut uvs = Vec::with_capacity(num_verts);
        let mut valid = Vec::with_capacity(num_verts);

        let r = self.raster_extent;
        for row in 0..=rows {
            for col in 0..=cols {
                let src = self.source_coord(f64::from(col), f64::from(row), cols, rows);
                // Texture row 0 is the top (max y) of the raster.
                uvs.push([
                    ((src.x - r.min().x) / r.width()) as f32,
                    ((r.max().y - src.y) / r.height()) as f32,
                ]);
                match self.project(src) {
                    Some(c) => {
                        positions.push([c.x as f32, c.y as f32]);
                        valid.push(true);
                    }
                    None => {
                        positions.push([0.0, 0.0]);
                        valid.push(false);
                    }
                }
            }
        }

        filter_outliers(&positions, &mut valid);

        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for (&[x, y], _) in positions.iter().zip(&valid).filter(|(_, v)| **v) {
            min_x = min_x.min(f64::from(x));
            min_y = min_y.min(f64::from(y));
            max_x = max_x.max(f64::from(x));
            max_y = max_y.max(f64::from(y));
        }

        rgis_layers::ProjectedRasterGrid {
            cols,
            rows,
            positions,
            uvs,
            valid,
            extent: geo::Rect::new(
                geo::coord! { x: min_x, y: min_y },
                geo::coord! { x: max_x, y: max_y },
            ),
        }
    }

    /// The largest distance, in texels, between where the mesh draws the
    /// midpoint of a grid edge and where that midpoint actually projects to,
    /// for horizontal edges, vertical edges, and cell diagonals (the shared
    /// edge of each cell's two triangles), respectively.
    fn max_error_texels(&self, grid: &rgis_layers::ProjectedRasterGrid) -> [f64; 3] {
        let (cols, rows) = (grid.cols, grid.rows);
        let stride = cols as usize + 1;
        let idx = |col: u32, row: u32| row as usize * stride + col as usize;
        let [w_px, h_px] = self.raster_size_px.map(f64::from);
        let pos = |i: usize| grid.positions[i].map(f64::from);
        let texel = |i: usize| {
            let [u, v] = grid.uvs[i].map(f64::from);
            [u * w_px, v * h_px]
        };

        let mut err = [0.0f64; 3];
        for row in 0..rows {
            for col in 0..cols {
                let corners @ [tl, tr, bl, br] = [
                    idx(col, row),
                    idx(col + 1, row),
                    idx(col, row + 1),
                    idx(col + 1, row + 1),
                ];
                if !corners.iter().all(|&i| grid.valid[i]) {
                    continue;
                }
                // The cell's target → texel mapping, linearised at `tl`, to
                // convert position errors to texels. Cells can be very
                // anisotropic (e.g. near a pole), so a single scale factor
                // won't do.
                let [p0, px, py] = [pos(tl), pos(tr), pos(bl)];
                let [t0, tx, ty] = [texel(tl), texel(tr), texel(bl)];
                let e1 = [px[0] - p0[0], px[1] - p0[1]];
                let e2 = [py[0] - p0[0], py[1] - p0[1]];
                let det = e1[0] * e2[1] - e1[1] * e2[0];
                if det.abs() < f64::MIN_POSITIVE {
                    continue;
                }
                let t1 = [tx[0] - t0[0], tx[1] - t0[1]];
                let t2 = [ty[0] - t0[0], ty[1] - t0[1]];
                let to_texels = |d: [f64; 2]| {
                    let a = (d[0] * e2[1] - d[1] * e2[0]) / det;
                    let b = (e1[0] * d[1] - e1[1] * d[0]) / det;
                    (a * t1[0] + b * t2[0]).hypot(a * t1[1] + b * t2[1])
                };

                let edge_error = |a: usize, b: usize, mid_col: f64, mid_row: f64| -> f64 {
                    let Some(actual) = self.project(self.source_coord(mid_col, mid_row, cols, rows))
                    else {
                        return 0.0;
                    };
                    let [ax, ay] = pos(a);
                    let [bx, by] = pos(b);
                    to_texels([actual.x - (ax + bx) / 2.0, actual.y - (ay + by) / 2.0])
                };

                let (c, r) = (f64::from(col), f64::from(row));
                // Horizontal edges: top, plus bottom on the last row.
                err[0] = err[0].max(edge_error(tl, tr, c + 0.5, r));
                if row + 1 == rows {
                    err[0] = err[0].max(edge_error(bl, br, c + 0.5, r + 1.0));
                }
                // Vertical edges: left, plus right on the last column.
                err[1] = err[1].max(edge_error(tl, bl, c, r + 0.5));
                if col + 1 == cols {
                    err[1] = err[1].max(edge_error(tr, br, c + 1.0, r + 0.5));
                }
                // The diagonal shared by the cell's two triangles
                // (top-right to bottom-left; see `triangles()`).
                err[2] = err[2].max(edge_error(tr, bl, c + 0.5, r + 0.5));
            }
        }
        err
    }
}

/// Fraction of a target CRS's area of use, on each side, by which a
/// projected-source raster may extend past it and still be drawn.
const AREA_OF_USE_MARGIN: f64 = 0.25;

/// Bounding box, in `crs`, of the given area of use.
fn area_of_use_in_crs<C: Context>(
    ctx: &C,
    crs: &rgis_primitives::Crs,
    area: &rgis_primitives::AreaOfUse,
) -> Option<geo::Rect<f64>> {
    const N: u32 = 32;
    let mut min = geo::coord! { x: f64::INFINITY, y: f64::INFINITY };
    let mut max = geo::coord! { x: f64::NEG_INFINITY, y: f64::NEG_INFINITY };
    for j in 0..=N {
        for i in 0..=N {
            let lon_lat = geo::coord! {
                x: area.lon_west + (area.lon_east - area.lon_west) * f64::from(i) / f64::from(N),
                y: area.lat_south + (area.lat_north - area.lat_south) * f64::from(j) / f64::from(N),
            };
            let Ok(c) = rgis_crs::lon_lat_to_crs(ctx, crs, lon_lat) else {
                continue;
            };
            if c.x.is_finite() && c.y.is_finite() {
                min = geo::coord! { x: min.x.min(c.x), y: min.y.min(c.y) };
                max = geo::coord! { x: max.x.max(c.x), y: max.y.max(c.y) };
            }
        }
    }
    if min.x > max.x || min.y > max.y {
        return None;
    }
    // Pad by one lattice step: the area's true extent in `crs` may bulge
    // between lattice points.
    let pad_x = (max.x - min.x) / f64::from(N);
    let pad_y = (max.y - min.y) / f64::from(N);
    Some(geo::Rect::new(
        geo::coord! { x: min.x - pad_x, y: min.y - pad_y },
        geo::coord! { x: max.x + pad_x, y: max.y + pad_y },
    ))
}

fn intersect(a: geo::Rect<f64>, b: geo::Rect<f64>) -> Option<geo::Rect<f64>> {
    let min = geo::coord! { x: a.min().x.max(b.min().x), y: a.min().y.max(b.min().y) };
    let max = geo::coord! { x: a.max().x.min(b.max().x), y: a.max().y.min(b.max().y) };
    (min.x < max.x && min.y < max.y).then(|| geo::Rect::new(min, max))
}

/// Invalidate outlier positions using IQR-based detection.
/// This handles near-polar Mercator vertices and other projection
/// singularities that produce extreme but finite values.
fn filter_outliers(positions: &[[f32; 2]], valid: &mut [bool]) {
    let mut valid_xs: Vec<f64> = Vec::new();
    let mut valid_ys: Vec<f64> = Vec::new();
    for (&[x, y], _) in positions.iter().zip(valid.iter()).filter(|(_, v)| **v) {
        valid_xs.push(f64::from(x));
        valid_ys.push(f64::from(y));
    }
    if valid_xs.len() < 4 {
        return;
    }
    valid_xs.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
    valid_ys.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());

    let n = valid_xs.len();
    let q1_x = valid_xs[n / 4];
    let q3_x = valid_xs[3 * n / 4];
    let iqr_x = q3_x - q1_x;
    let q1_y = valid_ys[n / 4];
    let q3_y = valid_ys[3 * n / 4];
    let iqr_y = q3_y - q1_y;

    let lo_x = q1_x - 3.0 * iqr_x;
    let hi_x = q3_x + 3.0 * iqr_x;
    let lo_y = q1_y - 3.0 * iqr_y;
    let hi_y = q3_y + 3.0 * iqr_y;

    for (&[x, y], is_valid) in positions.iter().zip(valid.iter_mut()) {
        let (x, y) = (f64::from(x), f64::from(y));
        if *is_valid && (x < lo_x || x > hi_x || y < lo_y || y > hi_y) {
            *is_valid = false;
        }
    }
}

pub struct ReprojectGeometryJob {
    pub feature_collection: Arc<geo_features::FeatureCollection<geo_projected::UnprojectedScalar>>,
    pub layer_id: rgis_primitives::LayerId,
    pub source_crs: rgis_primitives::Crs,
    pub target_crs: rgis_primitives::Crs,
    pub geodesy_ctx: rgis_crs::GeodesyContext,
}

pub struct ReprojectGeometryJobOutcome {
    pub feature_collection: geo_features::FeatureCollection<geo_projected::ProjectedScalar>,
    pub layer_id: rgis_primitives::LayerId,
    pub target_crs: rgis_primitives::Crs,
}

impl bevy_jobs::Job for ReprojectGeometryJob {
    type Outcome = Result<ReprojectGeometryJobOutcome, rgis_crs::TransformError>;

    fn name(&self) -> String {
        "Projecting layer".to_string()
    }

    async fn perform(self, progress_sender: bevy_jobs::Context) -> Self::Outcome {
        let mut feature_collection = Arc::unwrap_or_clone(self.feature_collection);
        let total = feature_collection.features.len();

        clamp_to_area_of_use(&mut feature_collection, &self.source_crs, &self.target_crs);

        let mut feature_collection = feature_collection.cast::<geo_projected::Projected>();

        for (i, feature) in feature_collection.features.iter_mut().enumerate() {
            let _ = progress_sender.send_progress((100 * i / total) as u8).await;

            let geodesy_ctx = self.geodesy_ctx.read().unwrap();
            let transformer =
                rgis_crs::CrsTransformer::new(&*geodesy_ctx, &self.source_crs, &self.target_crs);
            project_feature(&transformer, feature)?;
        }

        feature_collection.recalculate_bounding_rect();

        Ok(ReprojectGeometryJobOutcome {
            feature_collection,
            layer_id: self.layer_id,
            target_crs: self.target_crs,
        })
    }
}

/// If the source CRS is geographic, clamp coordinates to the target CRS's
/// area of use before projecting. This prevents singularities (e.g. Mercator
/// Y → ∞ at the poles) from producing extreme values.
pub(crate) fn clamp_to_area_of_use(
    feature_collection: &mut geo_features::FeatureCollection<geo_projected::UnprojectedScalar>,
    source_crs: &rgis_primitives::Crs,
    target_crs: &rgis_primitives::Crs,
) {
    if !source_crs.is_geographic() {
        return;
    }
    let Some(area) = target_crs.area_of_use() else {
        return;
    };
    // Only clamp if the bounding rect is within the area of use.
    // If all coordinates fall outside (e.g. projected meter coords
    // mistakenly tagged as geographic), clamping would collapse the
    // geometry to a degenerate line or point.
    let bbox = feature_collection.bounding_rect();
    let within_area = bbox.map_or(true, |r| {
        r.min().x.0 <= area.lon_east
            && r.max().x.0 >= area.lon_west
            && r.min().y.0 <= area.lat_north
            && r.max().y.0 >= area.lat_south
    });
    if !within_area {
        return;
    }
    for feature in &mut feature_collection.features {
        if let Some(ref mut geom) = feature.geometry {
            *geom = geom.map_coords(|coord| geo::Coord {
                x: geo_projected::UnprojectedScalar::new(
                    coord.x.0.clamp(area.lon_west, area.lon_east),
                ),
                y: geo_projected::UnprojectedScalar::new(
                    coord.y.0.clamp(area.lat_south, area.lat_north),
                ),
            });
        }
    }
}

pub(crate) fn project_feature<C: Context>(
    transformer: &rgis_crs::CrsTransformer<'_, C>,
    feature: &mut geo_features::Feature<geo_projected::ProjectedScalar>,
) -> Result<(), rgis_crs::TransformError> {
    if let Some(ref mut geometry) = feature.geometry {
        transformer.transform_geometry(geometry)?;
    }
    feature.recalculate_bounding_rect();
    Ok(())
}
