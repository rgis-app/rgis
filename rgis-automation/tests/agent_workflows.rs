//! Realistic workflows run headlessly: no OS window, no GPU, no renderer or
//! UI plugins. Each test drives the app with the same commands `--script`
//! uses and asserts on [`AppState`], not pixels.
//!
//! What this can't cover (meshes, egui windows, screenshots) is checked by
//! the Playwright suite in `www/tests` and by real `--script` runs; see
//! `screenshot_needs_the_renderer` for how that boundary shows up here.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowResolution};
use rgis_automation::state::{Bbox, LayerKind, LayerState};
use rgis_automation::{AppState, ScriptRunner};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
const GEOTIFF_TEST_DATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../geotiff-test-data");
const MAX_UPDATES: usize = 200_000;

/// An rgis app with every non-rendering plugin.
struct Harness {
    app: App,
}

impl Harness {
    fn new() -> Self {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        // Just the `Window` component (no winit, no OS window), so the camera
        // systems have a viewport size to work with.
        app.world_mut().spawn((
            Window {
                resolution: WindowResolution::new(1280, 720),
                ..default()
            },
            PrimaryWindow,
        ));
        // Resources normally provided by the input and UI plugins.
        app.init_resource::<ButtonInput<KeyCode>>();
        app.insert_resource(rgis_units::SidePanelWidth(0.));
        app.insert_resource(rgis_units::TopPanelHeight(0.));
        app.insert_resource(rgis_units::BottomPanelHeight(0.));
        app.add_plugins((
            rgis_events::RgisEventsPlugin,
            rgis_ui_messages::Plugin,
            bevy_jobs::Plugin,
            rgis_crs::Plugin::default(),
            rgis_layers::Plugin,
            rgis_file_loader::Plugin,
            rgis_transform::Plugin,
            rgis_camera::Plugin::default(),
            rgis_automation::Plugin,
        ));
        rgis_primitives::RgisSet::configure(&mut app);
        Harness { app }
    }

    /// Run a script (JSON) to completion. Returns the error of the step that
    /// failed, if any.
    fn run(&mut self, script: &str) -> Result<(), String> {
        let steps = rgis_automation::parse_script(script)?;
        let errors_before = self.state().command_errors.len();
        self.app
            .world_mut()
            .resource_mut::<ScriptRunner>()
            .push(steps);
        for _ in 0..MAX_UPDATES {
            self.app.update();
            if self.app.world().resource::<ScriptRunner>().is_finished() {
                let state = self.state();
                return match state.command_errors.get(errors_before..) {
                    Some([.., error]) => Err(error.clone()),
                    _ => Ok(()),
                };
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("script didn't finish after {MAX_UPDATES} updates");
    }

    fn state(&mut self) -> AppState {
        rgis_automation::app_state(self.app.world_mut())
    }

    fn layer(&mut self, name: &str) -> LayerState {
        let state = self.state();
        state
            .layers
            .iter()
            .find(|layer| layer.name == name)
            .cloned()
            .unwrap_or_else(|| panic!("no layer named {name:?} in {:#?}", state.layers))
    }

    fn layer_names(&mut self) -> Vec<String> {
        self.state()
            .layers
            .into_iter()
            .map(|layer| layer.name)
            .collect()
    }
}

fn fixture(name: &str) -> String {
    format!("{FIXTURES}/{name}")
}

fn load(name: &str) -> String {
    format!(
        r#"[{{"cmd": "load_file", "path": "{}"}}, {{"cmd": "wait_idle"}}]"#,
        fixture(name)
    )
}

#[track_caller]
fn assert_bbox_near(actual: Option<Bbox>, expected: [f64; 4], tolerance: f64) {
    let actual = actual.expect("expected a bbox");
    let actual_values = [actual.min_x, actual.min_y, actual.max_x, actual.max_y];
    for (a, e) in actual_values.iter().zip(expected) {
        assert!(
            (a - e).abs() <= tolerance,
            "bbox {actual:?} isn't within {tolerance} of {expected:?}"
        );
    }
}

// Extents of the fixtures as rgis projects them into EPSG:3857.
//
// KNOWN ISSUE: EPSG:3857 is spherical Mercator, but rgis currently computes
// y on the GRS80 ellipsoid. `geodesy::authoring::parse_proj` only understands
// `+a` together with `+rf`, so the `+a=6378137 +b=6378137` in 3857's PROJ
// definition is dropped. x is unaffected. The true Web Mercator y values are
// noted next to each constant; switch to them once that's fixed.
const SHAPES_3857: [f64; 4] = [
    -1_113_194.907_932_735_7,
    -553_583.846_797_552_7, // true 3857: -557_305.257_274_576_9
    3_339_584.723_798_207,
    3_482_189.085_304_026_5, // true 3857: 3_503_549.843_504_374
];
/// `antimeridian.geojson`'s box runs along ±10° between 170° and -170°. The
/// fix splits it where those edges cross 180° along a great circle, which
/// bulges poleward to this latitude.
const ANTIMERIDIAN_CROSSING_LAT: f64 = 10.151_081_711_048_134;
const ANTIMERIDIAN_3857: [f64; 4] = [
    -20_037_508.342_789_244,
    -1_128_446.008_460_946_6, // true 3857: -1_135_971.755_199_03
    20_037_508.342_789_244,
    1_128_446.008_460_946_6, // true 3857: 1_135_971.755_199_03
];

/// Workflow 1: loading a GeoJSON file creates a layer with the right
/// features and extent.
#[test]
fn load_geojson() {
    let mut h = Harness::new();
    h.run(&load("shapes.geojson")).unwrap();

    let state = h.state();
    assert!(state.idle);
    assert_eq!(state.jobs_in_flight, 0);
    assert_eq!(state.target_crs.as_ref().unwrap().epsg, Some(3857));
    assert_eq!(state.layers.len(), 1);

    let layer = &state.layers[0];
    assert_eq!(layer.name, "shapes.geojson");
    assert_eq!(layer.index, 0);
    assert_eq!(layer.kind, LayerKind::Vector);
    assert!(layer.visible);
    assert_eq!(layer.crs.epsg, Some(4326));
    assert_eq!(layer.feature_count, Some(3));
    assert_eq!(layer.geometry_types, ["Polygon"]);
    assert_bbox_near(layer.bbox, [-10., -5., 30., 30.], 1e-9);
    assert!(layer.projected);
    assert_bbox_near(layer.projected_bbox, SHAPES_3857, 1.);
    // No renderer in this harness, so "rendered" is unknown rather than false.
    assert_eq!(layer.rendered, None);
    assert!(layer.fill_color.is_some());
    assert!(state.messages.is_empty(), "{:?}", state.messages);
}

/// Workflow 2: changing the CRS 4326 → 3857 reprojects the layer.
#[test]
fn change_crs_reprojects_layers() {
    let mut h = Harness::new();
    h.run(&load("shapes.geojson")).unwrap();

    h.run(r#"[{"cmd": "change_crs", "epsg": 4326}, {"cmd": "wait_idle"}]"#)
        .unwrap();
    let state = h.state();
    let target = state.target_crs.unwrap();
    assert_eq!(target.epsg, Some(4326));
    assert_eq!(target.name.as_deref(), Some("WGS 84"));
    assert!(target.geographic);
    assert_bbox_near(state.layers[0].projected_bbox, [-10., -5., 30., 30.], 1e-6);

    h.run(r#"[{"cmd": "change_crs", "epsg": 3857}, {"cmd": "wait_idle"}]"#)
        .unwrap();
    let state = h.state();
    let target = state.target_crs.unwrap();
    assert_eq!(target.epsg, Some(3857));
    assert_eq!(target.name.as_deref(), Some("WGS 84 / Pseudo-Mercator"));
    assert!(!target.geographic);
    assert_bbox_near(state.layers[0].projected_bbox, SHAPES_3857, 1.);
    // The source extent never changes.
    assert_bbox_near(state.layers[0].bbox, [-10., -5., 30., 30.], 1e-9);
}

/// A CRS given as a PROJ string instead of an EPSG code.
#[test]
fn change_crs_with_proj_string() {
    let mut h = Harness::new();
    h.run(&load("shapes.geojson")).unwrap();
    h.run(
        r#"[{"cmd": "change_crs", "proj": "+proj=merc +a=6378137 +b=6378137"},
            {"cmd": "wait_idle"}]"#,
    )
    .unwrap();

    let state = h.state();
    let target = state.target_crs.unwrap();
    assert_eq!(target.epsg, None);
    assert_eq!(
        target.proj.as_deref(),
        Some("+proj=merc +a=6378137 +b=6378137")
    );
    assert_bbox_near(state.layers[0].projected_bbox, SHAPES_3857, 1.);
}

/// Workflow 3: a polygon crossing the antimeridian is split, and stays valid and
/// narrow after reprojection instead of wrapping around the world.
#[test]
fn antimeridian_geometry_survives_reprojection() {
    use geo::{BoundingRect, Validation};

    let mut h = Harness::new();
    h.run(&load("antimeridian.geojson")).unwrap();

    let layer = h.layer("antimeridian.geojson");
    assert_eq!(layer.geometry_types, ["MultiPolygon"]);
    let lat = ANTIMERIDIAN_CROSSING_LAT;
    assert_bbox_near(layer.bbox, [-180., -lat, 180., lat], 1e-9);
    assert_bbox_near(layer.projected_bbox, ANTIMERIDIAN_3857, 1.);

    // The state only has the overall extent, so check the parts directly.
    let world = h.app.world_mut();
    let mut layers = world.query::<&rgis_layers::LayerData>();
    let data = layers.single(world).unwrap();
    let projected = data.projected_feature_collection().unwrap();
    let geometry = projected.features[0].geometry.as_ref().unwrap();
    let geo::Geometry::MultiPolygon(parts) = geometry else {
        panic!("expected a MultiPolygon, got {geometry:?}");
    };
    assert_eq!(parts.0.len(), 2);
    for part in &parts.0 {
        let part: geo::Polygon<f64> =
            geo::MapCoords::map_coords(part, |c| geo::Coord { x: c.x.0, y: c.y.0 });
        assert!(part.is_valid(), "invalid part: {part:?}");
        // Each half spans 10° of longitude (~1,113 km). Without the split, a
        // part would run the long way around (~37,800 km).
        let width = part.bounding_rect().unwrap().width();
        assert!(width < 1_200_000., "part is {width} m wide: {part:?}");
    }
}

/// Workflow 4: toggle, reorder, duplicate, rename, recolor, and delete layers.
#[test]
fn manage_layers() {
    let mut h = Harness::new();
    h.run(&load("shapes.geojson")).unwrap();
    h.run(&load("lines.geojson")).unwrap();
    assert_eq!(h.layer_names(), ["shapes.geojson", "lines.geojson"]);

    // Lines have no fill.
    assert_eq!(h.layer("lines.geojson").fill_color, None);

    h.run(r#"[{"cmd": "toggle_visibility", "layer": "shapes.geojson"}]"#)
        .unwrap();
    assert!(!h.layer("shapes.geojson").visible);
    // With `visible`, the toggle is idempotent.
    h.run(
        r#"[{"cmd": "toggle_visibility", "layer": 0, "visible": true},
              {"cmd": "toggle_visibility", "layer": 0, "visible": true}]"#,
    )
    .unwrap();
    assert!(h.layer("shapes.geojson").visible);

    h.run(r#"[{"cmd": "move_layer", "layer": "shapes.geojson", "direction": "up"}]"#)
        .unwrap();
    assert_eq!(h.layer_names(), ["lines.geojson", "shapes.geojson"]);
    assert_eq!(h.layer("shapes.geojson").index, 1);
    // Moving the top layer up is a no-op.
    h.run(r#"[{"cmd": "move_layer", "layer": -1, "direction": "up"}]"#)
        .unwrap();
    assert_eq!(h.layer_names(), ["lines.geojson", "shapes.geojson"]);

    h.run(r#"[{"cmd": "duplicate_layer", "layer": "lines.geojson"}, {"cmd": "wait_idle"}]"#)
        .unwrap();
    assert_eq!(
        h.layer_names(),
        ["lines.geojson", "shapes.geojson", "Copy of lines.geojson"]
    );
    let copy = h.layer("Copy of lines.geojson");
    assert_eq!(copy.feature_count, Some(1));
    assert!(copy.projected);

    let error = h
        .run(
            r##"[{"cmd": "rename_layer", "layer": -1, "name": "Copied path"},
                 {"cmd": "set_stroke_color", "layer": "Copied path", "color": "#ff0000"},
                 {"cmd": "set_fill_color", "layer": {"id": 999}, "color": "#00ff00"}]"##,
        )
        .unwrap_err();
    assert!(error.contains("no layer matches id 999"), "{error}");
    // The steps before the failing one ran.
    assert_eq!(h.layer("Copied path").stroke_color, "#ff0000ff");

    let shapes_id = h.layer("shapes.geojson").id;
    h.run(&format!(
        r#"[{{"cmd": "set_fill_color", "layer": {{"id": {shapes_id}}}, "color": [0, 0, 1, 0.5]}},
            {{"cmd": "set_point_size", "layer": "shapes.geojson", "size": 9}}]"#
    ))
    .unwrap();
    let shapes = h.layer("shapes.geojson");
    assert_eq!(shapes.fill_color.as_deref(), Some("#0000ff80"));
    assert_eq!(shapes.point_size, 9.);

    h.run(r#"[{"cmd": "delete_layer", "layer": "shapes.geojson"}]"#)
        .unwrap();
    assert_eq!(h.layer_names(), ["lines.geojson", "Copied path"]);
    assert_eq!(h.layer("Copied path").index, 1);
}

/// Layer references that don't match produce errors that list the layers,
/// and stop the rest of the script.
#[test]
fn bad_layer_reference_stops_the_script() {
    let mut h = Harness::new();
    h.run(&load("shapes.geojson")).unwrap();

    let error = h
        .run(&format!(
            r#"[{{"cmd": "toggle_visibility", "layer": "nope"}},
                {{"cmd": "load_file", "path": "{}"}}]"#,
            fixture("lines.geojson")
        ))
        .unwrap_err();
    assert_eq!(
        error,
        "step 3 (toggle_visibility): no layer matches \"nope\"; \
         layers from bottom to top: [0: \"shapes.geojson\"]"
    );
    let state = h.state();
    assert_eq!(
        state.layers.len(),
        1,
        "the load after the error must not run"
    );
    assert_eq!(state.script.last_error.as_deref(), Some(error.as_str()));
    assert_eq!(state.command_errors, [error]);

    let error = h
        .run(r#"[{"cmd": "delete_layer", "layer": 5}]"#)
        .unwrap_err();
    assert!(error.contains("no layer matches index 5"), "{error}");
}

/// Workflow 5: loading a GeoTIFF creates a raster layer with the file's
/// extent and CRS.
#[test]
fn load_geotiff() {
    let path = format!(
        "{GEOTIFF_TEST_DATA}/rasterio_generated/fixtures/uint8_rgb_deflate_block64_cog.tif"
    );
    assert!(
        std::path::Path::new(&path).exists(),
        "{path} is missing; run `git submodule update --init`"
    );
    let mut h = Harness::new();
    h.run(&format!(
        r#"[{{"cmd": "load_file", "path": "{path}"}}, {{"cmd": "wait_idle"}}]"#
    ))
    .unwrap();

    let layer = h.layer("uint8_rgb_deflate_block64_cog.tif");
    assert_eq!(layer.kind, LayerKind::Raster);
    assert_eq!(layer.feature_count, None);
    assert_eq!(layer.crs.epsg, Some(4326));
    let raster = layer.raster.unwrap();
    assert_eq!((raster.width, raster.height), (128, 128));
    assert_eq!(raster.format, "rgba");
    assert_bbox_near(layer.bbox, [0., -1.28, 1.28, 0.], 1e-9);
    assert!(layer.projected);
    // See SHAPES_3857 about y. True 3857 min_y: -142_500.802_048_179_5.
    assert_bbox_near(
        layer.projected_bbox,
        [0., -141_547.005_159_885_65, 142_488.948_215_390_18, 0.],
        1.,
    );
    // Rasters can't be duplicated.
    let error = h
        .run(r#"[{"cmd": "duplicate_layer", "layer": 0}]"#)
        .unwrap_err();
    assert!(
        error.contains("only vector layers can be duplicated"),
        "{error}"
    );
}

/// Workflow 6: select a feature, then deselect it.
#[test]
fn select_and_deselect_feature() {
    let mut h = Harness::new();
    h.run(&load("shapes.geojson")).unwrap();

    h.run(r#"[{"cmd": "select_feature", "layer": "shapes.geojson", "feature": 1}]"#)
        .unwrap();
    let selected = h.state().selected_feature.expect("a selected feature");
    assert_eq!(selected.layer_name.as_deref(), Some("shapes.geojson"));
    assert_eq!(selected.layer_index, Some(0));
    assert_eq!(selected.feature_index, Some(1));
    assert!(
        selected
            .properties
            .contains(&("name".to_string(), "Triangle".to_string())),
        "{:?}",
        selected.properties
    );

    let error = h
        .run(r#"[{"cmd": "select_feature", "layer": 0, "feature": 3}]"#)
        .unwrap_err();
    assert!(
        error.contains("has 3 feature(s); index 3 is out of range"),
        "{error}"
    );
    assert!(
        h.state().selected_feature.is_some(),
        "a failed select keeps the selection"
    );

    h.run(r#"[{"cmd": "deselect"}]"#).unwrap();
    assert_eq!(h.state().selected_feature, None);

    // Deleting the selected feature's layer also clears the selection.
    h.run(
        r#"[{"cmd": "select_feature", "layer": 0, "feature": 0},
              {"cmd": "delete_layer", "layer": 0}]"#,
    )
    .unwrap();
    assert_eq!(h.state().selected_feature, None);
}

/// Workflow 7: malformed files and unknown CRSs are reported in the state; nothing
/// panics.
#[test]
fn errors_are_reported_in_the_state() {
    let mut h = Harness::new();

    // The load is accepted, then fails in the background.
    h.run(&load("malformed.geojson")).unwrap();
    let state = h.state();
    assert!(state.layers.is_empty());
    assert_eq!(state.messages.len(), 1, "{:?}", state.messages);
    assert!(
        state.messages[0].starts_with("Error loading file:"),
        "{:?}",
        state.messages
    );

    let error = h
        .run(r#"[{"cmd": "change_crs", "epsg": 9999}]"#)
        .unwrap_err();
    assert_eq!(error, "step 3 (change_crs): Unknown EPSG code: 9999");
    let error = h
        .run(r#"[{"cmd": "change_crs", "proj": "+proj=bogus"}]"#)
        .unwrap_err();
    assert!(
        error.contains("invalid PROJ string \"+proj=bogus\""),
        "{error}"
    );
    let error = h
        .run(&format!(
            r#"[{{"cmd": "load_file", "path": "{}", "crs": 9999}}]"#,
            fixture("shapes.geojson")
        ))
        .unwrap_err();
    assert!(error.contains("Unknown EPSG code: 9999"), "{error}");
    let error = h
        .run(r#"[{"cmd": "load_file", "path": "/no/such/file.geojson"}]"#)
        .unwrap_err();
    assert!(
        error.contains("couldn't read \"/no/such/file.geojson\""),
        "{error}"
    );
    let error = h
        .run(r#"[{"cmd": "load_file", "path": "notes.txt"}]"#)
        .unwrap_err();
    assert!(error.contains("can't tell the format"), "{error}");

    let state = h.state();
    assert_eq!(
        state.target_crs.unwrap().epsg,
        Some(3857),
        "the CRS is unchanged"
    );
    assert_eq!(state.command_errors.len(), 5, "{:#?}", state.command_errors);

    // Bad scripts are rejected before anything runs.
    assert!(rgis_automation::parse_script(r#"[{"cmd": "explode"}]"#).is_err());
}

/// The camera follows center/zoom/pan commands.
#[test]
fn camera_commands() {
    let mut h = Harness::new();
    h.run(&load("shapes.geojson")).unwrap();

    h.run(r#"[{"cmd": "center_on_layer", "layer": 0}, {"cmd": "wait_idle"}]"#)
        .unwrap();
    let state = h.state();
    let camera = state.camera.unwrap();
    assert!(!camera.animating);
    assert_eq!(
        (camera.viewport.width, camera.viewport.height),
        (1280., 720.)
    );
    let extent = state.layers[0].projected_bbox.unwrap();
    let [min_x, min_y, max_x, max_y] = [extent.min_x, extent.min_y, extent.max_x, extent.max_y];
    assert!((camera.center.x - (min_x + max_x) / 2.).abs() < 1.);
    assert!((camera.center.y - (min_y + max_y) / 2.).abs() < 1.);
    let visible = camera.visible_bbox.unwrap();
    assert!(visible.min_x <= min_x + 1. && visible.max_x >= max_x - 1.);
    assert!(visible.min_y <= min_y + 1. && visible.max_y >= max_y - 1.);

    h.run(r#"[{"cmd": "zoom", "factor": 2}]"#).unwrap();
    let zoomed = h.state().camera.unwrap();
    assert!((zoomed.scale - camera.scale / 2.).abs() < camera.scale * 1e-4);

    h.run(r#"[{"cmd": "pan", "dx": 100, "dy": -50}]"#).unwrap();
    let panned = h.state().camera.unwrap();
    let scale = f64::from(zoomed.scale);
    assert!((panned.center.x - (zoomed.center.x + 100. * scale)).abs() < 1.);
    assert!((panned.center.y - (zoomed.center.y - 50. * scale)).abs() < 1.);
}

/// Regression test for #284: a single point has no extent to fit, which used
/// to set the camera scale to zero (hanging the scale bar, and leaving zoom
/// stuck at zero).
#[test]
fn center_on_a_single_point() {
    let mut h = Harness::new();
    let point = serde_json::json!({
        "type": "Feature",
        "properties": {},
        "geometry": {"type": "Point", "coordinates": [10, 20]},
    });
    let script = serde_json::json!([
        {"cmd": "load_text", "text": point.to_string(), "format": "geojson"},
        {"cmd": "wait_idle"},
    ]);
    h.run(&script.to_string()).unwrap();
    let initial_scale = h.state().camera.unwrap().scale;

    h.run(r#"[{"cmd": "center_on_layer", "layer": 0}, {"cmd": "wait_idle"}]"#)
        .unwrap();
    let state = h.state();
    let camera = state.camera.unwrap();
    assert_eq!(camera.scale, initial_scale);
    let point = state.layers[0].projected_bbox.unwrap();
    assert!((camera.center.x - point.min_x).abs() < 1.);
    assert!((camera.center.y - point.min_y).abs() < 1.);

    h.run(r#"[{"cmd": "zoom", "factor": 2}]"#).unwrap();
    assert_eq!(h.state().camera.unwrap().scale, initial_scale / 2.);
}

/// A camera flight in progress when the CRS changes is cancelled. Otherwise
/// it carries on to its target in the old CRS: here, a Web Mercator scale of
/// thousands of metres per pixel that would then be read as degrees per pixel.
#[test]
fn crs_change_cancels_camera_flight() {
    let mut h = Harness::new();
    h.run(&load("shapes.geojson")).unwrap();

    h.run(
        r#"[
            {"cmd": "center_on_layer", "layer": 0},
            {"cmd": "change_crs", "epsg": 4326},
            {"cmd": "wait_idle"}
        ]"#,
    )
    .unwrap();
    let camera = h.state().camera.unwrap();
    assert!(!camera.animating);
    assert!(camera.scale < 1., "{} degrees per pixel", camera.scale);
}

/// Batch runs (`--dump-state` etc.): on failure, the state is still written
/// and the app exits with code 1.
#[test]
fn batch_run_failure_dumps_state_and_exits_nonzero() {
    let dump = std::env::temp_dir().join(format!(
        "rgis-automation-test-{}-failure.json",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&dump);

    let mut h = Harness::new();
    let steps = rgis_automation::parse_script(&format!(
        r#"[{{"cmd": "load_file", "path": "{}"}},
            {{"cmd": "wait_idle"}},
            {{"cmd": "change_crs", "epsg": 9999}},
            {{"cmd": "dump_state", "path": "{}"}}]"#,
        fixture("shapes.geojson"),
        dump.display()
    ))
    .unwrap();
    h.app
        .insert_resource(ScriptRunner::new(steps).exit_when_done(Some(dump.clone())));

    let mut exit = None;
    for _ in 0..MAX_UPDATES {
        h.app.update();
        exit = h.app.should_exit();
        if exit.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(exit, Some(AppExit::from_code(1)));

    let dumped: AppState = serde_json::from_str(&std::fs::read_to_string(&dump).unwrap()).unwrap();
    let _ = std::fs::remove_file(&dump);
    assert_eq!(dumped.layers.len(), 1);
    assert_eq!(
        dumped.script.last_error.as_deref(),
        Some("step 3 (change_crs): Unknown EPSG code: 9999")
    );
}

/// Screenshots need the renderer, which this harness doesn't have. The step
/// fails with a clear error instead of hanging; real screenshots are covered
/// by running `rgis --script ... --screenshot out.png`.
#[test]
fn screenshot_needs_the_renderer() {
    let mut h = Harness::new();
    let error = h
        .run(r#"[{"cmd": "screenshot", "path": "out.png"}]"#)
        .unwrap_err();
    assert_eq!(
        error,
        "step 1 (screenshot): screenshots need the renderer, which isn't running"
    );
}
