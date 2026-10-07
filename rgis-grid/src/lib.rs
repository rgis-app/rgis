use bevy::prelude::*;

pub struct Plugin;

#[derive(Resource)]
struct GridFont(Handle<Font>);

impl bevy::app::Plugin for Plugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (spawn_grid, load_grid_font));
        app.add_systems(PostUpdate, update_grid);
        app.add_systems(Update, update_grid_labels);
    }
}

fn load_grid_font(mut commands: Commands, mut fonts: ResMut<Assets<Font>>) {
    let font_data = include_bytes!("../../rgis/assets/fonts/RobotoCondensed-VariableFont_wght.ttf");
    let font = fonts.add(Font::try_from_bytes(font_data.to_vec()).expect("Failed to load grid font"));
    commands.insert_resource(GridFont(font));
}

// ── Degree-friendly intervals ────────────────────────────────────────────────

/// Degree-friendly intervals sorted largest → smallest.
/// Whole degrees, then arc-minutes, then arc-seconds.
const DEGREE_INTERVALS: &[f32] = &[
    180.0,
    90.0,
    45.0,
    30.0,
    15.0,
    10.0,
    5.0,
    2.0,
    1.0,
    30.0 / 60.0,
    15.0 / 60.0,
    10.0 / 60.0,
    5.0 / 60.0,
    2.0 / 60.0,
    1.0 / 60.0,
    30.0 / 3600.0,
    15.0 / 3600.0,
    10.0 / 3600.0,
    5.0 / 3600.0,
    2.0 / 3600.0,
    1.0 / 3600.0,
];

/// Pick the smallest degree-friendly interval that keeps at least
/// `min_screen_px` pixels between consecutive grid lines.
fn nice_degree_interval(degrees_per_pixel: f32, min_screen_px: f32) -> f32 {
    let min_degrees = degrees_per_pixel * min_screen_px;
    let mut result = DEGREE_INTERVALS[0];
    for &interval in DEGREE_INTERVALS {
        if interval >= min_degrees {
            result = interval;
        } else {
            break;
        }
    }
    result
}

// ── Generic 1-2-5 interval (fallback for unknown CRS) ───────────────────────

fn nice_interval(camera_scale: f32, min_screen_px: f32) -> f32 {
    let min_spacing = camera_scale * min_screen_px;
    let mag = 10f32.powf(min_spacing.log10().floor());
    let normalized = min_spacing / mag;
    if normalized <= 1.0 {
        mag
    } else if normalized <= 2.0 {
        2.0 * mag
    } else if normalized <= 5.0 {
        5.0 * mag
    } else {
        10.0 * mag
    }
}

// ── Web Mercator helpers (EPSG:3857) ─────────────────────────────────────────

const EARTH_RADIUS: f64 = 6_378_137.0;

fn lon_to_x(lon_deg: f64) -> f32 {
    (lon_deg.to_radians() * EARTH_RADIUS) as f32
}

fn lat_to_y(lat_deg: f64) -> f32 {
    let lat_rad = lat_deg.to_radians();
    (EARTH_RADIUS * (std::f64::consts::FRAC_PI_4 + lat_rad / 2.0).tan().ln()) as f32
}

fn x_to_lon(x: f32) -> f64 {
    (x as f64 / EARTH_RADIUS).to_degrees()
}

fn y_to_lat(y: f32) -> f64 {
    (2.0 * (y as f64 / EARTH_RADIUS).exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees()
}

// ── CRS classification ──────────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum CrsKind {
    /// Coordinates are in degrees (e.g. EPSG:4326).
    Geographic,
    /// EPSG:3857 – coordinates in metres, but grid in degrees.
    WebMercator,
    /// Unknown projection – fall back to generic 1-2-5 intervals.
    Other,
}

fn classify_crs(target_crs: &rgis_crs::TargetCrs) -> CrsKind {
    if let Some(code) = target_crs.0.epsg_code {
        if code == 4326 {
            return CrsKind::Geographic;
        }
        if code == 3857 {
            return CrsKind::WebMercator;
        }
        // Look up the proj4 string for other EPSG codes.
        if let Some(def) = crs_definitions::from_code(code) {
            if def.proj4.contains("longlat") {
                return CrsKind::Geographic;
            }
        }
    }
    // Check the raw proj string if available.
    if let Some(ref proj) = target_crs.0.proj_string {
        if proj.contains("longlat") {
            return CrsKind::Geographic;
        }
    }
    CrsKind::Other
}

// ── Label formatting ────────────────────────────────────────────────────────

/// Format a degree value with N/S or E/W suffix.
fn format_degree(value: f64, is_latitude: bool) -> String {
    let suffix = if is_latitude {
        if value >= 0.0 { "N" } else { "S" }
    } else {
        if value >= 0.0 { "E" } else { "W" }
    };
    let abs = value.abs();
    let deg = abs.floor() as u32;
    let rem = (abs - deg as f64) * 60.0;
    let min = rem.floor() as u32;
    let sec = (rem - min as f64) * 60.0;

    if deg == 0 && min == 0 && sec.abs() > 0.01 {
        format!("{sec:.0}\u{2033} {suffix}")
    } else if sec.abs() > 0.01 {
        format!("{deg}\u{00b0}{min}\u{2032}{sec:.0}\u{2033} {suffix}")
    } else if min > 0 {
        format!("{deg}\u{00b0}{min}\u{2032} {suffix}")
    } else {
        format!("{deg}\u{00b0} {suffix}")
    }
}

/// Format a generic projected coordinate value.
fn format_value(value: f32) -> String {
    if value.abs() >= 1_000_000.0 {
        format!("{:.0}", value)
    } else if value.abs() >= 1.0 {
        format!("{:.1}", value)
    } else {
        format!("{:.4}", value)
    }
}

// ── Line ranges ─────────────────────────────────────────────────────────────

/// The coordinates grid lines can be drawn at, as `(min, max)`.
const LONGITUDES: (f64, f64) = (-180.0, 180.0);
const LATITUDES: (f64, f64) = (-90.0, 90.0);
const MERCATOR_LATITUDES: (f64, f64) = (-85.051_129, 85.051_129);

/// Backstop for grids without a domain: far more lines than fit in a window
/// at `MIN_LINE_SPACING_PX` apart.
const MAX_LINES_PER_AXIS: f64 = 1000.0;

/// Indices `i` of the grid lines at `i * interval` that cover `min..=max`,
/// limited to lines inside `domain`.
///
/// Degree intervals stop growing at 180°, so without a domain, zooming far
/// out would mean a line every 180° across an unbounded view. If there would
/// still be more than `MAX_LINES_PER_AXIS` lines, there are none.
fn line_indices(
    min: f64,
    max: f64,
    interval: f64,
    domain: Option<(f64, f64)>,
) -> impl Iterator<Item = i64> {
    let mut first = (min / interval).floor();
    let mut last = (max / interval).ceil();
    if let Some((domain_min, domain_max)) = domain {
        first = first.max((domain_min / interval).ceil());
        last = last.min((domain_max / interval).floor());
    }
    let drawable = first.is_finite() && last.is_finite() && last - first <= MAX_LINES_PER_AXIS;
    drawable
        .then_some(first as i64..=last as i64)
        .into_iter()
        .flatten()
}

// ── Constants ────────────────────────────────────────────────────────────────

const MIN_LINE_SPACING_PX: f32 = 80.0;
const GRID_Z: f32 = -0.01;
const LABEL_FONT_SIZE: f32 = 11.0;
const LABEL_MARGIN_PX: f32 = 8.0;

// ── Grid colour ──────────────────────────────────────────────────────────────

fn grid_color(clear_color: &ClearColor) -> Color {
    let bg = clear_color.0.to_srgba();
    let luminance = 0.299 * bg.red + 0.587 * bg.green + 0.114 * bg.blue;
    if luminance < 0.5 {
        Color::srgba(1.0, 1.0, 1.0, 0.08)
    } else {
        Color::srgba(0.0, 0.0, 0.0, 0.12)
    }
}

fn label_color(clear_color: &ClearColor) -> Color {
    let bg = clear_color.0.to_srgba();
    let luminance = 0.299 * bg.red + 0.587 * bg.green + 0.114 * bg.blue;
    if luminance < 0.5 {
        Color::srgba(1.0, 1.0, 1.0, 0.35)
    } else {
        Color::srgba(0.0, 0.0, 0.0, 0.45)
    }
}

// ── Bevy components / state ──────────────────────────────────────────────────

#[derive(Component)]
struct Grid;

#[derive(Component)]
struct GridLabel;

#[derive(Default)]
struct LastCameraState {
    translation: Vec3,
    scale: Vec3,
    window_size: Vec2,
}

// ── Viewport info shared between grid mesh and label systems ─────────────────

struct ViewportInfo {
    camera_scale: f32,
    win_w: f32,
    win_h: f32,
    world_left: f32,
    world_right: f32,
    world_bottom: f32,
    world_top: f32,
}

fn get_viewport_info(
    camera_query: &Query<&Transform, With<Camera>>,
    windows: &Query<&Window, With<bevy::window::PrimaryWindow>>,
) -> Option<ViewportInfo> {
    let Ok(transform) = camera_query.single() else {
        return None;
    };
    let Ok(window) = windows.single() else {
        return None;
    };
    let camera_scale = transform.scale.x;
    let win_w = window.width();
    let win_h = window.height();
    let half_w = win_w * camera_scale * 0.5;
    let half_h = win_h * camera_scale * 0.5;
    let cam_x = transform.translation.x;
    let cam_y = transform.translation.y;
    Some(ViewportInfo {
        camera_scale,
        win_w,
        win_h,
        world_left: cam_x - half_w,
        world_right: cam_x + half_w,
        world_bottom: cam_y - half_h,
        world_top: cam_y + half_h,
    })
}

// ── Systems ──────────────────────────────────────────────────────────────────

fn spawn_grid(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let mesh = meshes.add(Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    ));
    let material = materials.add(Color::srgba(0.0, 0.0, 0.0, 0.12));
    commands.spawn((
        Mesh2d(mesh),
        MeshMaterial2d(material),
        Transform::from_xyz(0.0, 0.0, GRID_Z),
        bevy::picking::Pickable::IGNORE,
        Grid,
    ));
}

fn update_grid(
    grid_query: Query<(&Mesh2d, &MeshMaterial2d<ColorMaterial>), With<Grid>>,
    camera_query: Query<&Transform, With<Camera>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    clear_color: Res<ClearColor>,
    target_crs: Option<Res<rgis_crs::TargetCrs>>,
    mut last_state: Local<LastCameraState>,
) {
    let Ok(transform) = camera_query.single() else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };
    let Ok((mesh_handle, mat_handle)) = grid_query.single() else {
        return;
    };

    let window_size = Vec2::new(window.width(), window.height());
    if transform.translation == last_state.translation
        && transform.scale == last_state.scale
        && window_size == last_state.window_size
    {
        return;
    }

    last_state.translation = transform.translation;
    last_state.scale = transform.scale;
    last_state.window_size = window_size;

    if let Some(mat) = materials.get_mut(&mat_handle.0) {
        mat.color = grid_color(&clear_color);
    }

    let Some(vp) = get_viewport_info(&camera_query, &windows) else {
        return;
    };

    let thickness = vp.camera_scale;
    let height = vp.world_top - vp.world_bottom;
    let center_y = (vp.world_top + vp.world_bottom) * 0.5;
    let width = vp.world_right - vp.world_left;
    let center_x = (vp.world_right + vp.world_left) * 0.5;

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let crs_kind = target_crs
        .as_ref()
        .map(|crs| classify_crs(crs))
        .unwrap_or(CrsKind::Other);

    match crs_kind {
        CrsKind::Geographic => {
            let deg_per_px_x = (vp.world_right - vp.world_left) / vp.win_w;
            let deg_per_px_y = (vp.world_top - vp.world_bottom) / vp.win_h;

            let lon_interval = nice_degree_interval(deg_per_px_x, MIN_LINE_SPACING_PX);
            let lat_interval = nice_degree_interval(deg_per_px_y, MIN_LINE_SPACING_PX);

            for i in line_indices(
                f64::from(vp.world_left),
                f64::from(vp.world_right),
                f64::from(lon_interval),
                Some(LONGITUDES),
            ) {
                let x = i as f32 * lon_interval;
                add_rect(&mut positions, &mut indices, x, center_y, thickness, height);
            }

            for i in line_indices(
                f64::from(vp.world_bottom),
                f64::from(vp.world_top),
                f64::from(lat_interval),
                Some(LATITUDES),
            ) {
                let y = i as f32 * lat_interval;
                add_rect(&mut positions, &mut indices, center_x, y, width, thickness);
            }
        }

        CrsKind::WebMercator => {
            let lon_left = x_to_lon(vp.world_left);
            let lon_right = x_to_lon(vp.world_right);
            let lat_bottom = y_to_lat(vp.world_bottom);
            let lat_top = y_to_lat(vp.world_top);

            let deg_per_px_lon = (lon_right - lon_left) as f32 / vp.win_w;

            // Use the same degree interval for both axes. Mercator's non-linear
            // y-axis makes per-pixel latitude density meaningless, and geographic
            // grids conventionally use uniform degree spacing.
            let interval = nice_degree_interval(deg_per_px_lon, MIN_LINE_SPACING_PX);
            let lon_interval = interval;
            let lat_interval = interval;

            for i in line_indices(
                lon_left,
                lon_right,
                f64::from(lon_interval),
                Some(LONGITUDES),
            ) {
                let lon = i as f64 * lon_interval as f64;
                if lon.abs() > 180.0 {
                    continue;
                }
                let x = lon_to_x(lon);
                add_rect(&mut positions, &mut indices, x, center_y, thickness, height);
            }

            for i in line_indices(
                lat_bottom,
                lat_top,
                f64::from(lat_interval),
                Some(MERCATOR_LATITUDES),
            ) {
                let lat = i as f64 * lat_interval as f64;
                if lat.abs() > 85.051_129 {
                    continue;
                }
                let y = lat_to_y(lat);
                add_rect(&mut positions, &mut indices, center_x, y, width, thickness);
            }
        }

        CrsKind::Other => {
            let interval = nice_interval(vp.camera_scale, MIN_LINE_SPACING_PX);

            for i in line_indices(
                f64::from(vp.world_left),
                f64::from(vp.world_right),
                f64::from(interval),
                None,
            ) {
                let x = i as f32 * interval;
                add_rect(&mut positions, &mut indices, x, center_y, thickness, height);
            }

            for i in line_indices(
                f64::from(vp.world_bottom),
                f64::from(vp.world_top),
                f64::from(interval),
                None,
            ) {
                let y = i as f32 * interval;
                add_rect(&mut positions, &mut indices, center_x, y, width, thickness);
            }
        }
    }

    if let Some(mesh) = meshes.get_mut(&mesh_handle.0) {
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_indices(bevy::mesh::Indices::U32(indices));
    }
}

// ── Grid label rendering (world-space Text2d) ──────────────────────────────

const LABEL_Z: f32 = -0.005;

/// Collected label data: world position + text + anchor.
struct LabelSpec {
    world_x: f32,
    world_y: f32,
    text: String,
    anchor: bevy::sprite::Anchor,
}

fn update_grid_labels(
    mut commands: Commands,
    label_query: Query<Entity, With<GridLabel>>,
    camera_query: Query<&Transform, With<Camera>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    clear_color: Res<ClearColor>,
    target_crs: Option<Res<rgis_crs::TargetCrs>>,
    grid_font: Option<Res<GridFont>>,
    side_panel_width: Res<rgis_units::SidePanelWidth>,
    bottom_panel_height: Res<rgis_units::BottomPanelHeight>,
    mut last_state: Local<LastCameraState>,
) {
    // Wait for the font to be available before spawning labels.
    let Some(ref font_res) = grid_font else {
        return;
    };

    let Ok(transform) = camera_query.single() else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };

    let window_size = Vec2::new(window.width(), window.height());
    if transform.translation == last_state.translation
        && transform.scale == last_state.scale
        && window_size == last_state.window_size
        && !label_query.is_empty()
    {
        return;
    }

    last_state.translation = transform.translation;
    last_state.scale = transform.scale;
    last_state.window_size = window_size;

    // Despawn all previous labels.
    for entity in label_query.iter() {
        commands.entity(entity).despawn();
    }

    let Some(vp) = get_viewport_info(&camera_query, &windows) else {
        return;
    };

    let color = label_color(&clear_color);
    let text_scale = vp.camera_scale;

    // Compute world positions for label edges (offset from viewport edges).
    let bottom_margin_world = (bottom_panel_height.0 + LABEL_MARGIN_PX + 16.0) * vp.camera_scale;
    let left_margin_world = (side_panel_width.0 + LABEL_MARGIN_PX + 4.0) * vp.camera_scale;
    let label_world_y = vp.world_bottom + bottom_margin_world;
    let label_world_x = vp.world_left + left_margin_world;

    let crs_kind = target_crs
        .as_ref()
        .map(|crs| classify_crs(crs))
        .unwrap_or(CrsKind::Other);

    let mut labels: Vec<LabelSpec> = Vec::new();

    match crs_kind {
        CrsKind::Geographic => {
            let deg_per_px_x = (vp.world_right - vp.world_left) / vp.win_w;
            let deg_per_px_y = (vp.world_top - vp.world_bottom) / vp.win_h;
            let lon_interval = nice_degree_interval(deg_per_px_x, MIN_LINE_SPACING_PX);
            let lat_interval = nice_degree_interval(deg_per_px_y, MIN_LINE_SPACING_PX);

            for i in line_indices(
                f64::from(vp.world_left),
                f64::from(vp.world_right),
                f64::from(lon_interval),
                Some(LONGITUDES),
            ) {
                let x = i as f32 * lon_interval;
                labels.push(LabelSpec {
                    world_x: x,
                    world_y: label_world_y,
                    text: format_degree(x as f64, false),
                    anchor: bevy::sprite::Anchor::BOTTOM_CENTER,
                });
            }

            for i in line_indices(
                f64::from(vp.world_bottom),
                f64::from(vp.world_top),
                f64::from(lat_interval),
                Some(LATITUDES),
            ) {
                let y = i as f32 * lat_interval;
                labels.push(LabelSpec {
                    world_x: label_world_x,
                    world_y: y,
                    text: format_degree(y as f64, true),
                    anchor: bevy::sprite::Anchor::CENTER_LEFT,
                });
            }
        }

        CrsKind::WebMercator => {
            let lon_left = x_to_lon(vp.world_left);
            let lon_right = x_to_lon(vp.world_right);
            let lat_bottom = y_to_lat(vp.world_bottom);
            let lat_top = y_to_lat(vp.world_top);

            let deg_per_px_lon = (lon_right - lon_left) as f32 / vp.win_w;

            let interval = nice_degree_interval(deg_per_px_lon, MIN_LINE_SPACING_PX);
            let lon_interval = interval;
            let lat_interval = interval;

            for i in line_indices(
                lon_left,
                lon_right,
                f64::from(lon_interval),
                Some(LONGITUDES),
            ) {
                let lon = i as f64 * lon_interval as f64;
                if lon.abs() > 180.0 {
                    continue;
                }
                let x = lon_to_x(lon);
                labels.push(LabelSpec {
                    world_x: x,
                    world_y: label_world_y,
                    text: format_degree(lon, false),
                    anchor: bevy::sprite::Anchor::BOTTOM_CENTER,
                });
            }

            for i in line_indices(
                lat_bottom,
                lat_top,
                f64::from(lat_interval),
                Some(MERCATOR_LATITUDES),
            ) {
                let lat = i as f64 * lat_interval as f64;
                if lat.abs() > 85.051_129 {
                    continue;
                }
                let y = lat_to_y(lat);
                labels.push(LabelSpec {
                    world_x: label_world_x,
                    world_y: y,
                    text: format_degree(lat, true),
                    anchor: bevy::sprite::Anchor::CENTER_LEFT,
                });
            }
        }

        CrsKind::Other => {
            let interval = nice_interval(vp.camera_scale, MIN_LINE_SPACING_PX);

            for i in line_indices(
                f64::from(vp.world_left),
                f64::from(vp.world_right),
                f64::from(interval),
                None,
            ) {
                let x = i as f32 * interval;
                labels.push(LabelSpec {
                    world_x: x,
                    world_y: label_world_y,
                    text: format_value(x),
                    anchor: bevy::sprite::Anchor::BOTTOM_CENTER,
                });
            }

            for i in line_indices(
                f64::from(vp.world_bottom),
                f64::from(vp.world_top),
                f64::from(interval),
                None,
            ) {
                let y = i as f32 * interval;
                labels.push(LabelSpec {
                    world_x: label_world_x,
                    world_y: y,
                    text: format_value(y),
                    anchor: bevy::sprite::Anchor::CENTER_LEFT,
                });
            }
        }
    }

    // Spawn Text2d entities for each label.
    for label in labels {
        commands.spawn((
            Text2d::new(label.text),
            TextFont {
                font: font_res.0.clone(),
                font_size: LABEL_FONT_SIZE,
                ..default()
            },
            TextColor(color),
            label.anchor,
            Transform {
                translation: Vec3::new(label.world_x, label.world_y, LABEL_Z),
                scale: Vec3::new(text_scale, text_scale, 1.0),
                ..default()
            },
            bevy::picking::Pickable::IGNORE,
            GridLabel,
        ));
    }
}

fn add_rect(
    positions: &mut Vec<[f32; 3]>,
    indices: &mut Vec<u32>,
    cx: f32,
    cy: f32,
    w: f32,
    h: f32,
) {
    let base = positions.len() as u32;
    let hw = w * 0.5;
    let hh = h * 0.5;
    positions.push([cx - hw, cy - hh, 0.0]);
    positions.push([cx + hw, cy - hh, 0.0]);
    positions.push([cx + hw, cy + hh, 0.0]);
    positions.push([cx - hw, cy + hh, 0.0]);
    indices.push(base);
    indices.push(base + 1);
    indices.push(base + 2);
    indices.push(base);
    indices.push(base + 2);
    indices.push(base + 3);
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── nice_degree_interval ────────────────────────────────────────────

    #[test]
    fn degree_interval_zoomed_out_picks_large_interval() {
        // ~0.28 deg/px (full world across 1280px)
        let interval = nice_degree_interval(360.0 / 1280.0, 80.0);
        assert!(interval >= 15.0, "expected >=15°, got {interval}");
    }

    #[test]
    fn degree_interval_zoomed_in_picks_small_interval() {
        // ~0.001 deg/px (city-level zoom)
        let interval = nice_degree_interval(0.001, 80.0);
        assert!(interval <= 1.0, "expected <=1°, got {interval}");
    }

    #[test]
    fn degree_interval_returns_value_from_list() {
        let interval = nice_degree_interval(0.05, 80.0);
        assert!(
            DEGREE_INTERVALS.contains(&interval),
            "interval {interval} not in DEGREE_INTERVALS"
        );
    }

    #[test]
    fn degree_interval_never_below_smallest() {
        let smallest = *DEGREE_INTERVALS.last().unwrap();
        let interval = nice_degree_interval(0.0000001, 80.0);
        assert!(interval >= smallest);
    }

    // ── nice_interval (1-2-5 generic) ───────────────────────────────────

    #[test]
    fn nice_interval_picks_round_values() {
        let interval = nice_interval(1.0, 80.0);
        // 80 units min spacing → should pick 100
        assert_eq!(interval, 100.0);
    }

    #[test]
    fn nice_interval_scales_with_camera() {
        let a = nice_interval(1.0, 80.0);
        let b = nice_interval(10.0, 80.0);
        assert!(b > a, "larger camera scale should give larger interval");
    }

    // ── Mercator round-trip ─────────────────────────────────────────────

    #[test]
    fn lon_x_round_trip() {
        for lon in [-180.0, -90.0, 0.0, 45.0, 180.0] {
            let x = lon_to_x(lon);
            let back = x_to_lon(x);
            assert!((back - lon).abs() < 1e-4, "lon {lon} -> x {x} -> {back}");
        }
    }

    #[test]
    fn lat_y_round_trip() {
        for lat in [-85.0, -45.0, 0.0, 45.0, 85.0] {
            let y = lat_to_y(lat);
            let back = y_to_lat(y);
            assert!((back - lat).abs() < 1e-6, "lat {lat} -> y {y} -> {back}");
        }
    }

    #[test]
    fn equator_maps_to_near_zero() {
        assert!(lat_to_y(0.0).abs() < 1e-6);
        assert!(lon_to_x(0.0).abs() < 1e-6);
    }

    #[test]
    fn mercator_y_increases_with_latitude() {
        assert!(lat_to_y(45.0) > lat_to_y(0.0));
        assert!(lat_to_y(0.0) > lat_to_y(-45.0));
    }

    // ── format_degree ───────────────────────────────────────────────────

    #[test]
    fn format_degree_zero_latitude() {
        let s = format_degree(0.0, true);
        assert_eq!(s, "0\u{00b0} N");
    }

    #[test]
    fn format_degree_zero_longitude() {
        let s = format_degree(0.0, false);
        assert_eq!(s, "0\u{00b0} E");
    }

    #[test]
    fn format_degree_negative_latitude() {
        let s = format_degree(-45.0, true);
        assert!(s.ends_with("S"), "expected S suffix, got {s}");
        assert!(s.contains("45"), "expected 45 in {s}");
    }

    #[test]
    fn format_degree_with_minutes() {
        let s = format_degree(45.5, true);
        assert!(s.contains("30\u{2032}"), "expected 30′ in {s}");
    }

    #[test]
    fn format_degree_west_longitude() {
        let s = format_degree(-90.0, false);
        assert!(s.ends_with("W"), "expected W suffix, got {s}");
    }

    // ── format_value ────────────────────────────────────────────────────

    #[test]
    fn format_value_large() {
        assert_eq!(format_value(1_500_000.0), "1500000");
    }

    #[test]
    fn format_value_medium() {
        assert_eq!(format_value(123.4), "123.4");
    }

    #[test]
    fn format_value_small() {
        assert_eq!(format_value(0.0012), "0.0012");
    }

    // ── line_indices ────────────────────────────────────────────────────

    #[test]
    fn line_indices_cover_the_view() {
        let indices: Vec<_> = line_indices(-250.0, 250.0, 100.0, None).collect();
        assert_eq!(indices, [-3, -2, -1, 0, 1, 2, 3]);
    }

    #[test]
    fn line_indices_stay_inside_the_domain() {
        let indices: Vec<_> = line_indices(-1e9, 1e9, 45.0, Some(LATITUDES)).collect();
        assert_eq!(indices, [-2, -1, 0, 1, 2]);
        // A view entirely past the antimeridian.
        assert_eq!(
            line_indices(200.0, 300.0, 10.0, Some(LONGITUDES)).count(),
            0
        );
    }

    #[test]
    fn line_indices_give_up_on_too_many_lines() {
        assert_eq!(line_indices(-1e30, 1e30, 1.0, None).count(), 0);
        assert_eq!(
            line_indices(f64::NEG_INFINITY, f64::INFINITY, 180.0, None).count(),
            0
        );
    }

    // ── Grid systems ────────────────────────────────────────────────────

    /// Run the grid systems once with the camera at `scale` target-CRS units
    /// per pixel, and return how many grid lines and labels they made.
    fn grid_at_scale(epsg: u16, scale: f32) -> (usize, usize) {
        let mut geodesy_ctx = geodesy::ctx::Minimal::default();
        let op_handle = rgis_crs::epsg_code_to_geodesy_op_handle(&mut geodesy_ctx, epsg)
            .unwrap_or_else(|e| panic!("EPSG:{epsg}: {e}"));

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<ColorMaterial>()
            .init_resource::<ClearColor>()
            .insert_resource(rgis_crs::TargetCrs(rgis_primitives::Crs {
                epsg_code: Some(epsg),
                proj_string: None,
                op_handle,
            }))
            .insert_resource(GridFont(Handle::default()))
            .insert_resource(rgis_units::SidePanelWidth(0.0))
            .insert_resource(rgis_units::BottomPanelHeight(0.0))
            .add_systems(Startup, spawn_grid)
            .add_systems(Update, (update_grid, update_grid_labels));
        app.world_mut().spawn((
            Window {
                resolution: bevy::window::WindowResolution::new(1280, 720),
                ..default()
            },
            bevy::window::PrimaryWindow,
        ));
        app.world_mut().spawn((
            Camera::default(),
            Transform::from_scale(Vec3::new(scale, scale, 1.0)),
        ));
        app.update();

        let world = app.world_mut();
        let labels = world
            .query_filtered::<(), With<GridLabel>>()
            .iter(world)
            .count();
        let mesh = world
            .query_filtered::<&Mesh2d, With<Grid>>()
            .iter(world)
            .next()
            .map(|mesh| mesh.0.clone())
            .unwrap_or_else(|| panic!("no grid mesh"));
        let vertices = world
            .resource::<Assets<Mesh>>()
            .get(&mesh)
            .and_then(|mesh| mesh.attribute(Mesh::ATTRIBUTE_POSITION))
            .map_or(0, |positions| positions.len());
        // Each line is a rectangle with four vertices.
        (vertices / 4, labels)
    }

    /// EPSG codes for a geographic CRS, Web Mercator, and a projected CRS
    /// (UTM zone 33N), which each take a different path through the grid.
    const CRS_KINDS: [u16; 3] = [4326, 3857, 32633];

    #[test]
    fn grid_at_normal_zoom() {
        for (epsg, scale) in CRS_KINDS.into_iter().zip([0.1, 10_000.0, 1_000.0]) {
            let (lines, labels) = grid_at_scale(epsg, scale);
            assert!(lines >= 10, "EPSG:{epsg}: only {lines} lines");
            assert_eq!(lines, labels, "EPSG:{epsg}");
        }
    }

    // Regression test for #283: zooming far out used to generate a line and a
    // label every 180° across the whole view, which could be billions.
    #[test]
    fn grid_when_zoomed_far_out() {
        for epsg in CRS_KINDS {
            let (lines, labels) = grid_at_scale(epsg, 1e30);
            // About one window's worth of lines, at most.
            assert!(
                lines <= 50 && labels <= 50,
                "EPSG:{epsg}: {lines} lines and {labels} labels"
            );
        }
    }
}
