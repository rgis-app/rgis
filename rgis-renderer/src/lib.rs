use bevy::{ecs::relationship::RelatedSpawnerCommands, prelude::*};
use std::sync::atomic::{AtomicBool, AtomicU32};

mod jobs;
pub(crate) mod particles;
mod render_entity_index;
mod systems;
mod z_index;

pub use render_entity_index::RenderEntityIndex;

/// Counter incremented each time meshes are spawned for a layer (for test polling).
///
/// This must remain a global static (rather than an ECS Resource) because it is
/// read from the `#[wasm_bindgen]` FFI function `get_rendered_layer_count` in
/// `rgis/src/lib.rs`, which executes outside of Bevy's ECS schedule and cannot
/// access `Res<T>`.
pub static RENDERED_LAYER_COUNT: AtomicU32 = AtomicU32::new(0);

/// Number of entities currently fading in or out. Tests can poll this to wait
/// for animations to finish before taking screenshots.
pub static ACTIVE_FADE_COUNT: AtomicU32 = AtomicU32::new(0);

/// When false, fade-in/fade-out animations and the pulsing selection highlight
/// are disabled. Entities spawn at full opacity and despawns are immediate.
pub static ANIMATIONS_ENABLED: AtomicBool = AtomicBool::new(true);

use rgis_layers::LayerIndex;
use z_index::ZIndex;

#[derive(Clone, Copy, Component, PartialEq, Eq)]
pub enum RenderEntityType {
    Polygon,
    LineString,
    PointFill,
    PointStroke,
    SelectedPolygon,
    SelectedLineString,
    SelectedPoint,
    Raster,
}

pub struct Plugin;

impl bevy::app::Plugin for Plugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RenderEntityIndex>();
        app.add_observer(render_entity_index::on_add_layer_id);
        app.add_observer(render_entity_index::on_remove_layer_id);
        app.add_observer(on_add_fade_in);
        app.add_observer(on_remove_fade_in);
        app.add_observer(on_add_fade_out);
        app.add_observer(on_remove_fade_out);
        app.add_observer(on_add_visibility_fade_out);
        app.add_observer(on_remove_visibility_fade_out);
        systems::configure(app);
    }
}

fn on_add_fade_in(_trigger: On<Add, FadeIn>) {
    ACTIVE_FADE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

fn on_remove_fade_in(_trigger: On<Remove, FadeIn>) {
    ACTIVE_FADE_COUNT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
}

fn on_add_fade_out(_trigger: On<Add, FadeOut>) {
    ACTIVE_FADE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

fn on_remove_fade_out(_trigger: On<Remove, FadeOut>) {
    ACTIVE_FADE_COUNT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
}

fn on_add_visibility_fade_out(_trigger: On<Add, VisibilityFadeOut>) {
    ACTIVE_FADE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

fn on_remove_visibility_fade_out(_trigger: On<Remove, VisibilityFadeOut>) {
    ACTIVE_FADE_COUNT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
}

const SELECTED_COLOR: Color = Color::srgb(0.0, 0.9, 1.0); // bright cyan

#[derive(Copy, Clone, Component)]
struct Point;

#[derive(Copy, Clone, Component)]
struct LineString;

#[derive(Copy, Clone, Component)]
struct Polygon;

#[derive(Component)]
pub struct PointSprite {
    pub relative_scale: f32,
}

#[derive(Copy, Clone, Component)]
struct Fill;

#[derive(Copy, Clone, Component)]
struct Stroke;

/// Tracks a fade-in animation. Entities with this component will have their
/// alpha interpolated from 0 to their target alpha over `duration` seconds.
#[derive(Component)]
struct FadeIn {
    elapsed: f32,
    duration: f32,
    target_alpha: f32,
}

/// Marks an entity for fade-out and despawn. Alpha interpolates from current
/// value to 0, then the entity is despawned.
#[derive(Clone, Copy, Component)]
struct FadeOut {
    elapsed: f32,
    duration: f32,
}

/// Marks an entity for fade-out due to visibility toggle. Alpha interpolates
/// from current value to 0, then the entity's `Visibility` is set to `Hidden`.
/// Unlike `FadeOut`, the entity is NOT despawned.
#[derive(Clone, Copy, Component)]
struct VisibilityFadeOut {
    elapsed: f32,
    duration: f32,
}

/// Remembers the fully-opaque target alpha for a render entity so that
/// visibility fade-in can restore it correctly.
#[derive(Clone, Copy, Component)]
struct TargetAlpha(f32);

const FADE_DURATION: f32 = 0.4;

fn animations_enabled() -> bool {
    ANIMATIONS_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

fn spawn_raster(
    raster: &geo_raster::Raster,
    grid: &rgis_layers::ProjectedRasterGrid,
    layer_id: rgis_primitives::LayerId,
    layer_visible: bool,
    layer_index: LayerIndex,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
) {
    let (bevy_format, pixel_bytes) = match raster.format {
        geo_raster::RasterFormat::R8 => {
            // Convert grayscale to RGBA for Bevy
            let mut rgba = Vec::with_capacity(raster.data.len() * 4);
            for &g in &raster.data {
                rgba.push(g);
                rgba.push(g);
                rgba.push(g);
                rgba.push(255);
            }
            (bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, rgba)
        }
        geo_raster::RasterFormat::Rgba8 => {
            (bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, raster.data.clone())
        }
    };

    let image = Image::new(
        bevy::render::render_resource::Extent3d {
            width: raster.width,
            height: raster.height,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        pixel_bytes,
        bevy_format,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    );

    let image_handle = images.add(image);

    // Build a mesh from the projected grid. Every grid vertex is emitted (even
    // invalid ones, which no triangle references) so grid indices can be used
    // as mesh indices directly.
    let positions: Vec<[f32; 3]> = grid
        .positions
        .iter()
        .map(|&[x, y]| [x, y, 0.0])
        .collect();
    let uvs = grid.uvs.clone();
    let indices: Vec<u32> = grid
        .triangles()
        .flatten()
        .map(|i| i as u32)
        .collect();

    let z_index = ZIndex::calculate(layer_index, RenderEntityType::Raster);

    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(bevy::mesh::Indices::U32(indices));

    let visibility = if layer_visible {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };

    let animate = animations_enabled();
    let material = materials.add(ColorMaterial {
        color: if animate {
            Color::srgba(1.0, 1.0, 1.0, 0.0)
        } else {
            Color::WHITE
        },
        texture: Some(image_handle),
        alpha_mode: bevy::sprite_render::AlphaMode2d::Blend,
        ..Default::default()
    });

    let mut entity_commands = commands.spawn((
        Mesh2d(meshes.add(mesh)),
        MeshMaterial2d(material),
        Transform::from_xyz(0., 0., z_index.0 as f32),
        visibility,
        layer_id,
        RenderEntityType::Raster,
        TargetAlpha(1.0),
    ));
    if animate {
        entity_commands.insert(FadeIn {
            elapsed: 0.0,
            duration: FADE_DURATION,
            target_alpha: 1.0,
        });
    }
}

/// Spawn geometry meshes for a layer. Takes individual component values.
fn spawn_geometry_meshes(
    geometry_mesh: geo_bevy::GeometryMesh,
    materials: &mut Assets<ColorMaterial>,
    layer_id: rgis_primitives::LayerId,
    layer_visible: bool,
    layer_color: &rgis_layers::LayerColor,
    layer_point_size: f32,
    layer_index: LayerIndex,
    commands: &mut Commands,
    assets_meshes: &mut Assets<Mesh>,
    asset_server: &AssetServer,
    is_selected: bool,
) {
    let visibility = if layer_visible {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    let mut entity_commands = commands.spawn((visibility, Transform::default(), layer_id));
    match geometry_mesh {
        geo_bevy::GeometryMesh::Point(_) => {
            entity_commands.insert(Point);
        }
        geo_bevy::GeometryMesh::Polygon(_) => {
            entity_commands.insert(Polygon);
        }
        geo_bevy::GeometryMesh::LineString(_) => {
            entity_commands.insert(LineString);
        }
    };
    entity_commands.with_children(|commands| match geometry_mesh {
        geo_bevy::GeometryMesh::Point(points) => {
            spawn_point_geometry(
                commands,
                &points,
                asset_server,
                is_selected,
                layer_index,
                layer_id,
                layer_color,
                layer_point_size,
            );
        }
        geo_bevy::GeometryMesh::Polygon(polygon_mesh) => {
            spawn_polygon_geometry(
                commands,
                polygon_mesh,
                materials,
                assets_meshes,
                is_selected,
                layer_index,
                layer_color,
            );
        }
        geo_bevy::GeometryMesh::LineString(line_string_mesh) => {
            spawn_linestring_geometry(
                commands,
                line_string_mesh,
                materials,
                assets_meshes,
                is_selected,
                layer_index,
                layer_color,
            );
        }
    });
}

fn spawn_linestring_geometry(
    commands: &mut RelatedSpawnerCommands<ChildOf>,
    line_string_mesh: Mesh,
    materials: &mut Assets<ColorMaterial>,
    assets_meshes: &mut Assets<Mesh>,
    is_selected: bool,
    layer_index: LayerIndex,
    layer_color: &rgis_layers::LayerColor,
) {
    let entity_type = if is_selected {
        RenderEntityType::SelectedLineString
    } else {
        RenderEntityType::LineString
    };
    spawn_helper(
        materials,
        if is_selected {
            SELECTED_COLOR
        } else {
            layer_color.stroke
        },
        layer_index,
        line_string_mesh,
        commands,
        assets_meshes,
        entity_type,
        false,
    );
}

fn spawn_polygon_geometry(
    commands: &mut RelatedSpawnerCommands<ChildOf>,
    polygon_mesh: geo_bevy::PolygonMesh,
    materials: &mut Assets<ColorMaterial>,
    assets_meshes: &mut Assets<Mesh>,
    is_selected: bool,
    layer_index: LayerIndex,
    layer_color: &rgis_layers::LayerColor,
) {
    let polygon_entity_type = if is_selected {
        RenderEntityType::SelectedPolygon
    } else {
        RenderEntityType::Polygon
    };
    let line_string_entity_type = if is_selected {
        RenderEntityType::SelectedLineString
    } else {
        RenderEntityType::LineString
    };
    // Fill
    spawn_helper(
        materials,
        if is_selected {
            SELECTED_COLOR
        } else {
            match layer_color.fill {
                Some(color) => color,
                None => {
                    bevy::log::error!("Expected a fill color for polygon, but none was provided.");
                    SELECTED_COLOR
                }
            }
        },
        layer_index,
        polygon_mesh.mesh,
        commands,
        assets_meshes,
        polygon_entity_type,
        true,
    );
    // Exterior border
    spawn_helper(
        materials,
        layer_color.stroke,
        layer_index,
        polygon_mesh.exterior_mesh,
        commands,
        assets_meshes,
        line_string_entity_type,
        false,
    );
    // Interior borders
    for mesh in polygon_mesh.interior_meshes {
        spawn_helper(
            materials,
            layer_color.stroke,
            layer_index,
            mesh,
            commands,
            assets_meshes,
            line_string_entity_type,
            false,
        );
    }
}

fn spawn_point_geometry(
    commands: &mut RelatedSpawnerCommands<ChildOf>,
    points: &[geo_bevy::SpritePosition],
    asset_server: &AssetServer,
    is_selected: bool,
    layer_index: LayerIndex,
    layer_id: rgis_primitives::LayerId,
    layer_color: &rgis_layers::LayerColor,
    layer_point_size: f32,
) {
    let circle = asset_server.load("circle.png");
    let (stroke_entity_type, fill_entity_type) = if is_selected {
        (
            RenderEntityType::SelectedPoint,
            RenderEntityType::SelectedPoint,
        )
    } else {
        (RenderEntityType::PointStroke, RenderEntityType::PointFill)
    };
    // Stroke
    spawn_point_sprites(
        commands,
        points,
        layer_index,
        stroke_entity_type,
        layer_color.stroke,
        circle.clone(),
        1.0,
        false,
        layer_id,
        layer_point_size,
    );

    // Fill
    spawn_point_sprites(
        commands,
        points,
        layer_index,
        fill_entity_type,
        if is_selected {
            SELECTED_COLOR
        } else {
            layer_color.fill.unwrap()
        },
        circle.clone(),
        0.7, // Fill should be smaller than stroke.
        true,
        layer_id,
        layer_point_size,
    );
}

fn spawn_helper<'a>(
    materials: &'a mut Assets<ColorMaterial>,
    color: bevy::color::Color,
    layer_index: LayerIndex,
    mesh: Mesh,
    commands: &'a mut RelatedSpawnerCommands<ChildOf>,
    assets_meshes: &'a mut Assets<Mesh>,
    entity_type: RenderEntityType,
    is_fill: bool,
) -> bevy::ecs::system::EntityCommands<'a> {
    let animate = animations_enabled();
    let material = if animate {
        let mut faded_color = color;
        faded_color.set_alpha(0.0);
        materials.add(ColorMaterial {
            color: faded_color,
            alpha_mode: bevy::sprite_render::AlphaMode2d::Blend,
            ..Default::default()
        })
    } else {
        materials.add(color)
    };
    let z_index = ZIndex::calculate(layer_index, entity_type);
    let target_alpha = color.alpha();
    let mut entity_commands = commands.spawn((
        Mesh2d(assets_meshes.add(mesh)),
        Transform::from_xyz(0., 0., z_index.0 as f32),
        MeshMaterial2d(material),
        entity_type,
        TargetAlpha(target_alpha),
    ));
    if animate {
        entity_commands.insert(FadeIn {
            elapsed: 0.0,
            duration: FADE_DURATION,
            target_alpha,
        });
    }
    if is_fill {
        entity_commands.insert(Fill);
    } else {
        entity_commands.insert(Stroke);
    }
    entity_commands
}

fn spawn_point_sprites(
    commands: &mut RelatedSpawnerCommands<ChildOf>,
    points: &[geo_bevy::SpritePosition],
    layer_index: LayerIndex,
    entity_type: RenderEntityType,
    color: Color,
    circle: Handle<Image>,
    scale: f32,
    is_fill: bool,
    layer_id: rgis_primitives::LayerId,
    _layer_point_size: f32,
) {
    let animate = animations_enabled();
    let sprite_color = if animate {
        let mut faded_color = color;
        faded_color.set_alpha(0.0);
        faded_color
    } else {
        color
    };
    let target_alpha = color.alpha();
    for coord in points {
        let z_index = ZIndex::calculate(layer_index, entity_type);
        let transform = Transform::from_xyz(coord.x, coord.y, z_index.0 as f32);
        let mut entity_commands = commands.spawn((
            Sprite {
                color: sprite_color,
                image: circle.clone(),
                ..Default::default()
            },
            entity_type,
            transform,
            layer_id,
            PointSprite {
                relative_scale: scale,
            },
            TargetAlpha(target_alpha),
        ));
        if animate {
            entity_commands.insert(FadeIn {
                elapsed: 0.0,
                duration: FADE_DURATION,
                target_alpha,
            });
        }
        if is_fill {
            entity_commands.insert(Fill);
        } else {
            entity_commands.insert(Stroke);
        }
    }
}
