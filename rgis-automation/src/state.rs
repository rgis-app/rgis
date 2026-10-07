//! A JSON-serializable snapshot of everything an agent needs to check what
//! the app is doing, without looking at pixels.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Bumped whenever a field changes meaning or is removed.
pub const STATE_VERSION: u32 = 1;

/// How many recent warnings/errors from the log buffer to include.
const MAX_LOG_ENTRIES: usize = 50;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppState {
    pub version: u32,
    /// Frames rendered so far.
    pub frame: u32,
    /// True when nothing is in flight: no background jobs, camera flights,
    /// fades, or pending script steps, for a few consecutive frames.
    pub idle: bool,
    pub jobs_in_flight: usize,
    /// Names of the running background jobs, e.g. "Projecting layer".
    pub jobs: Vec<String>,
    /// `None` only before the first frame has run.
    pub target_crs: Option<CrsState>,
    /// `None` when there is no camera or no primary window.
    pub camera: Option<CameraState>,
    /// Layers in draw order: `layers[0]` is the bottom layer. The side panel
    /// lists them in the opposite order.
    pub layers: Vec<LayerState>,
    pub selected_feature: Option<SelectedFeatureState>,
    /// Titles of the egui windows on screen, alphabetically.
    pub open_windows: Vec<String>,
    /// Text shown in the message window, oldest first (typically errors such
    /// as "Error loading file: ...").
    pub messages: Vec<String>,
    /// Errors from script or dispatch commands, oldest first.
    pub command_errors: Vec<String>,
    /// Recent WARN and ERROR log entries, oldest first.
    pub logs: Vec<LogEntryState>,
    pub script: ScriptState,
    pub animations_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrsState {
    pub epsg: Option<u16>,
    pub proj: Option<String>,
    /// e.g. "WGS 84 / Pseudo-Mercator", when known.
    pub name: Option<String>,
    pub geographic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bbox {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Bbox {
    fn new(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Option<Bbox> {
        // JSON has no infinity/NaN; report "no extent" instead.
        [min_x, min_y, max_x, max_y]
            .iter()
            .all(|v| v.is_finite())
            .then_some(Bbox {
                min_x,
                min_y,
                max_x,
                max_y,
            })
    }

    fn from_rect<T: geo::CoordFloat>(rect: geo::Rect<T>) -> Option<Bbox> {
        Bbox::new(
            rect.min().x.to_f64()?,
            rect.min().y.to_f64()?,
            rect.max().x.to_f64()?,
            rect.max().y.to_f64()?,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraState {
    /// Center of the window, in target-CRS (projected) coordinates.
    pub center: Point,
    /// Target-CRS units per logical pixel. Smaller is more zoomed in.
    pub scale: f32,
    /// Window size in logical pixels.
    pub viewport: Size,
    /// The target-CRS area covered by the whole window (including the area
    /// under the side and top/bottom panels).
    pub visible_bbox: Option<Bbox>,
    /// True while the camera is flying to a new position.
    pub animating: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayerKind {
    Vector,
    Raster,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerState {
    /// Position in draw order (0 = bottom). Accepted as a layer reference.
    pub index: usize,
    /// Internal id; accepted as a layer reference via `{"id": n}`.
    pub id: u16,
    pub name: String,
    pub kind: LayerKind,
    pub visible: bool,
    /// The CRS of the layer's source data.
    pub crs: CrsState,
    /// Vector layers only.
    pub feature_count: Option<usize>,
    /// Vector layers only, e.g. ["MultiPolygon", "Polygon"].
    pub geometry_types: Vec<String>,
    /// Extent in the layer's source CRS.
    pub bbox: Option<Bbox>,
    /// Extent in the current target CRS; `None` until reprojection finishes.
    pub projected_bbox: Option<Bbox>,
    /// Whether the layer has been reprojected into the current target CRS.
    pub projected: bool,
    /// Whether meshes/sprites have been spawned for the layer. `None` when
    /// the renderer isn't running (e.g. in headless tests).
    pub rendered: Option<bool>,
    /// How many render entities are drawing the layer (e.g. one per vector
    /// mesh batch plus one per point sprite). Should not change when the CRS
    /// changes; growing means stale geometry is still on screen. `None` when
    /// the renderer isn't running.
    pub render_entities: Option<usize>,
    /// sRGB `#rrggbbaa`; `None` for layers without a fill (lines, rasters).
    pub fill_color: Option<String>,
    /// sRGB `#rrggbbaa`.
    pub stroke_color: String,
    pub point_size: f32,
    /// Raster layers only.
    pub raster: Option<RasterState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RasterState {
    pub width: u32,
    pub height: u32,
    /// "grayscale" or "rgba".
    pub format: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectedFeatureState {
    pub layer_id: u16,
    pub layer_index: Option<usize>,
    pub layer_name: Option<String>,
    /// 0-based position of the feature in the layer's source file; the value
    /// to pass to `select_feature`.
    pub feature_index: Option<usize>,
    /// Internal feature id.
    pub feature_id: u64,
    /// The feature's attributes, as shown in the properties window.
    pub properties: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEntryState {
    pub level: String,
    pub target: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScriptState {
    /// Steps queued but not finished (including the one in progress).
    pub pending_steps: usize,
    /// The step currently waiting (e.g. "wait_idle"), if any.
    pub current_step: Option<String>,
    pub completed_steps: usize,
    /// Set when a step failed; the rest of that script was skipped.
    pub last_error: Option<String>,
}

/// Build an [`AppState`] from the world.
///
/// Works with any subset of the rgis plugins: missing pieces (no renderer, no
/// UI, no window) show up as `None` or empty lists.
pub fn app_state(world: &mut World) -> AppState {
    let jobs: Vec<String> = world
        .query::<&bevy_jobs::InProgressJob>()
        .iter(world)
        .map(|job| job.name.clone())
        .collect();

    let layers = layer_states(world);
    let camera = camera_state(world);
    let selected_feature = selected_feature_state(world, &layers);

    let open_windows = world
        .get_resource::<rgis_ui::OpenWindows>()
        .map(|windows| windows.titles().map(String::from).collect())
        .unwrap_or_default();

    let (messages, command_errors) = world
        .get_resource::<crate::AutomationLog>()
        .map(|log| {
            (
                log.messages.iter().cloned().collect(),
                log.command_errors.iter().cloned().collect(),
            )
        })
        .unwrap_or_default();

    let script = world
        .get_resource::<crate::runner::ScriptRunner>()
        .map(crate::runner::ScriptRunner::state)
        .unwrap_or_default();

    let idle = world
        .get_resource::<crate::IdleTracker>()
        .is_some_and(crate::IdleTracker::is_idle)
        && script.pending_steps == 0;

    AppState {
        version: STATE_VERSION,
        frame: world
            .get_resource::<bevy::diagnostic::FrameCount>()
            .map_or(0, |frame| frame.0),
        idle,
        jobs_in_flight: jobs.len(),
        jobs,
        target_crs: world
            .get_resource::<rgis_crs::TargetCrs>()
            .map(|target| crs_state(&target.0)),
        camera,
        layers,
        selected_feature,
        open_windows,
        messages,
        command_errors,
        logs: log_entries(world),
        script,
        animations_enabled: rgis_renderer::ANIMATIONS_ENABLED
            .load(std::sync::atomic::Ordering::Relaxed),
    }
}

pub(crate) fn crs_state(crs: &rgis_primitives::Crs) -> CrsState {
    CrsState {
        epsg: crs.epsg_code,
        proj: crs.proj_string.clone(),
        name: crs.epsg_code.and_then(crs_name),
        geographic: crs.is_geographic(),
    }
}

/// The CRS name from the EPSG registry's WKT, e.g. `PROJCRS["WGS 84 / ..."`.
fn crs_name(epsg: u16) -> Option<String> {
    let wkt = crs_definitions::from_code(epsg)?.wkt;
    let (_, after) = wkt.split_once("[\"")?;
    let (name, _) = after.split_once('"')?;
    Some(name.to_string())
}

fn color_hex(color: Color) -> String {
    let [r, g, b, a] = color.to_srgba().to_u8_array();
    format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
}

fn layer_states(world: &World) -> Vec<LayerState> {
    let Some(order) = world.get_resource::<rgis_layers::LayerOrder>() else {
        return vec![];
    };
    let render_index = world.get_resource::<rgis_renderer::RenderEntityIndex>();

    order
        .iter_bottom_to_top()
        .enumerate()
        .filter_map(|(index, entity)| {
            let entity = world.get_entity(entity).ok()?;
            let id = *entity.get::<rgis_primitives::LayerId>()?;
            let data = entity.get::<rgis_layers::LayerData>()?;
            let color = entity.get::<rgis_layers::LayerColor>()?;

            let (kind, feature_count, geometry_types, bbox, projected_bbox, raster) = match data {
                rgis_layers::LayerData::Vector {
                    unprojected_feature_collection,
                    projected_feature_collection,
                    geom_type,
                } => (
                    LayerKind::Vector,
                    Some(unprojected_feature_collection.features.len()),
                    geom_type
                        .to_string()
                        .split(", ")
                        .filter(|name| *name != "(Empty)")
                        .map(String::from)
                        .collect(),
                    unprojected_feature_collection
                        .bounding_rect
                        .and_then(Bbox::from_rect),
                    projected_feature_collection
                        .as_ref()
                        .and_then(|fc| fc.bounding_rect)
                        .and_then(Bbox::from_rect),
                    None,
                ),
                rgis_layers::LayerData::Raster {
                    raster,
                    projected_grid,
                } => (
                    LayerKind::Raster,
                    None,
                    vec![],
                    Bbox::from_rect(raster.extent),
                    projected_grid
                        .as_ref()
                        .and_then(|grid| Bbox::from_rect(grid.extent)),
                    Some(RasterState {
                        width: raster.width,
                        height: raster.height,
                        format: match raster.format {
                            geo_raster::RasterFormat::R8 => "grayscale".into(),
                            geo_raster::RasterFormat::Rgba8 => "rgba".into(),
                        },
                    }),
                ),
            };

            // The index also contains the layer entity itself (it carries a
            // `LayerId`); anything else is a render entity.
            let render_entities = render_index.map(|render_index| {
                render_index
                    .get(id)
                    .iter()
                    .filter(|render_entity| {
                        world
                            .get::<rgis_layers::LayerMarker>(**render_entity)
                            .is_none()
                    })
                    .count()
            });

            Some(LayerState {
                index,
                id: id.get(),
                name: entity
                    .get::<rgis_layers::LayerName>()
                    .map(|name| name.0.clone())
                    .unwrap_or_default(),
                kind,
                visible: entity
                    .get::<rgis_layers::LayerVisible>()
                    .is_some_and(|visible| visible.0),
                crs: entity
                    .get::<rgis_layers::LayerCrs>()
                    .map(|crs| crs_state(&crs.0))?,
                feature_count,
                geometry_types,
                bbox,
                projected: data.is_active(),
                projected_bbox,
                rendered: render_entities.map(|count| count > 0),
                render_entities,
                fill_color: color.fill.map(color_hex),
                stroke_color: color_hex(color.stroke),
                point_size: entity
                    .get::<rgis_layers::LayerPointSize>()
                    .map_or(0., |size| size.0),
                raster,
            })
        })
        .collect()
}

fn camera_state(world: &mut World) -> Option<CameraState> {
    let (camera_entity, transform) = world
        .query_filtered::<(Entity, &Transform), With<Camera>>()
        .iter(world)
        .next()
        .map(|(entity, transform)| (entity, *transform))?;
    let window = world
        .query_filtered::<&Window, With<bevy::window::PrimaryWindow>>()
        .iter(world)
        .next()?;
    let viewport = Size {
        width: window.width(),
        height: window.height(),
    };

    let center = Point {
        x: f64::from(transform.translation.x),
        y: f64::from(transform.translation.y),
    };
    let scale = transform.scale.x;
    let half_width = f64::from(viewport.width * scale) / 2.;
    let half_height = f64::from(viewport.height * scale) / 2.;

    Some(CameraState {
        center,
        scale,
        viewport,
        visible_bbox: Bbox::new(
            center.x - half_width,
            center.y - half_height,
            center.x + half_width,
            center.y + half_height,
        ),
        animating: world
            .get::<rgis_camera::fly_to::CameraFlyTo>(camera_entity)
            .is_some(),
    })
}

fn selected_feature_state(world: &World, layers: &[LayerState]) -> Option<SelectedFeatureState> {
    let (layer_id, feature_id) = world.get_resource::<rgis_layers::SelectedFeature>()?.0?;
    let layer = layers.iter().find(|layer| layer.id == layer_id.get());

    let feature_collection = world
        .get_resource::<rgis_layers::LayerIdToEntity>()
        .and_then(|id_map| id_map.get(layer_id))
        .and_then(|entity| world.get::<rgis_layers::LayerData>(entity))
        .and_then(rgis_layers::LayerData::unprojected_feature_collection);
    let feature_index = feature_collection.and_then(|fc| {
        fc.features
            .iter()
            .position(|feature| feature.id == feature_id)
    });
    let properties = feature_collection
        .zip(feature_index)
        .and_then(|(fc, index)| {
            fc.properties
                .as_ref()
                .map(|properties| geo_features::properties_for_row(properties, index))
        })
        .unwrap_or_default();

    Some(SelectedFeatureState {
        layer_id: layer_id.get(),
        layer_index: layer.map(|layer| layer.index),
        layer_name: layer.map(|layer| layer.name.clone()),
        feature_index,
        feature_id: feature_id.get(),
        properties,
    })
}

fn log_entries(world: &World) -> Vec<LogEntryState> {
    let Some(buffer) = world.get_resource::<rgis_ui::log_buffer::LogBuffer>() else {
        return vec![];
    };
    let Ok(buffer) = buffer.0.lock() else {
        return vec![];
    };
    let mut entries: Vec<LogEntryState> = buffer
        .iter()
        .rev()
        .filter(|entry| entry.level <= bevy::log::Level::WARN)
        .take(MAX_LOG_ENTRIES)
        .map(|entry| LogEntryState {
            level: entry.level.to_string(),
            target: entry.target.clone(),
            message: entry.message.clone(),
        })
        .collect();
    entries.reverse();
    entries
}
