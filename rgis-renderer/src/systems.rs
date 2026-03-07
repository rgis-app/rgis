use bevy::prelude::*;

use crate::{jobs::MeshBuildingJob, RenderEntityIndex, RenderEntityType};

fn handle_picking_click(
    on: On<Pointer<Click>>,
    layer_query: Query<
        &rgis_primitives::LayerId,
        Or<(With<crate::Point>, With<crate::Polygon>, With<crate::LineString>)>,
    >,
    layers: Res<rgis_layers::Layers>,
    current_tool: Res<State<rgis_settings::Tool>>,
    mut bevy_egui_ctx: bevy_egui::EguiContexts,
    mut feature_selected_writer: MessageWriter<rgis_map_messages::FeatureSelectedMessage>,
    mut render_props_writer: MessageWriter<rgis_ui_messages::RenderFeaturePropertiesMessage>,
) {
    if *current_tool.get() != rgis_settings::Tool::Query {
        return;
    }

    if let Ok(ctx) = bevy_egui_ctx.ctx_mut() {
        if ctx.is_pointer_over_area() {
            return;
        }
    }

    // Only handle on parent entities (which have LayerId + geometry marker)
    let Ok(_layer_id) = layer_query.get(on.event_target()) else {
        return;
    };

    // Get the hit position in world space (== projected coordinates)
    let Some(hit_position) = on.event().event.hit.position else {
        return;
    };

    let coord = geo::Coord {
        x: num_t::Num::new(f64::from(hit_position.x)),
        y: num_t::Num::new(f64::from(hit_position.y)),
    };

    if let Some((layer, feature)) = layers.feature_from_click(coord) {
        render_props_writer.write(rgis_ui_messages::RenderFeaturePropertiesMessage {
            layer_id: layer.id,
            properties: feature.properties.clone(),
        });
        feature_selected_writer
            .write(rgis_map_messages::FeatureSelectedMessage(layer.id, feature.id));
    }
}

fn layer_loaded(
    layers: Res<rgis_layers::Layers>,
    mut event_reader: MessageReader<rgis_layer_messages::LODChangedMessage>,
    mut job_spawner: bevy_jobs::JobSpawner,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut meshes_spawned_event_writer: MessageWriter<rgis_renderer_messages::MeshesSpawnedMessage>,
    query: LayerEntitiesWithColorMaterialsOrImagesQuery,
) {
    for event in event_reader.read() {
        let Some(layer) = layers.get(event.0) else {
            continue;
        };

        match &layer.data {
            rgis_layers::LayerData::Raster { raster, projected_grid: Some(grid) } => {
                let Some(layer_with_index) = layers.get_with_index(event.0) else {
                    continue;
                };
                crate::spawn_raster(
                    raster,
                    grid,
                    layer_with_index.0,
                    layer_with_index.1,
                    &mut commands,
                    &mut images,
                    &mut meshes,
                    &mut materials,
                );
                meshes_spawned_event_writer.write(event.0.into());
                crate::RENDERED_LAYER_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            rgis_layers::LayerData::Vector { .. } => {
                let Some(feature_collection) = layer.get_current_lod_feature_collection() else {
                    continue;
                };

                // Despawn old meshes before spawning new ones
                for (_, entity) in query.iter().filter(|(i, _)| **i == layer.id) {
                    commands.entity(entity).despawn();
                }

                job_spawner.spawn(MeshBuildingJob {
                    layer_id: layer.id,
                    geometry: geo::Geometry::GeometryCollection(
                        feature_collection.to_geometry_collection(),
                    ),
                    is_selected: false,
                });
            }
            _ => {}
        }
    }
}

fn handle_mesh_building_job_outcome(
    mut commands: Commands,
    mut assets_meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    layers: Res<rgis_layers::Layers>,
    mut meshes_spawned_event_writer: MessageWriter<rgis_renderer_messages::MeshesSpawnedMessage>,
    mut finished_jobs: bevy_jobs::FinishedJobs,
    asset_server: Res<AssetServer>,
    query: LayerEntitiesWithColorMaterialsOrImagesQuery,
) {
    while let Some(outcome) = finished_jobs.take_next::<MeshBuildingJob>() {
        let crate::jobs::MeshBuildingJobOutcome {
            geometry_mesh,
            layer_id,
            is_selected,
        } = match outcome {
            Ok(outcome) => outcome,
            Err(e) => {
                error!("Error processing MeshBuildingJobOutcome: {:?}", e);
                continue;
            }
        };
        let Some(layer_with_index) = layers.get_with_index(layer_id) else {
            continue;
        };

        // Despawn old meshes before spawning new ones
        for (_, entity) in query.iter().filter(|(i, _)| **i == layer_id) {
            commands.entity(entity).despawn();
        }

        crate::spawn_geometry_meshes(
            geometry_mesh,
            &mut materials,
            layer_with_index,
            &mut commands,
            &mut assets_meshes,
            &asset_server,
            is_selected,
        );

        meshes_spawned_event_writer.write(layer_id.into());
        crate::RENDERED_LAYER_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

// TODO
fn handle_layer_z_index_updated_event(
    mut layer_z_index_updated_event_reader: MessageReader<rgis_layer_messages::LayerZIndexUpdatedMessage>,
    mut query: Query<(&mut Transform, &RenderEntityType)>,
    layers: Res<rgis_layers::Layers>,
    index: Res<RenderEntityIndex>,
) {
    for event in layer_z_index_updated_event_reader.read() {
        let Some(layer_with_index) = layers.get_with_index(event.0) else {
            continue;
        };

        for &entity in index.get(event.0) {
            if let Ok((mut transform, render_entity)) = query.get_mut(entity) {
                let z_index = crate::ZIndex::calculate(layer_with_index.1, *render_entity);
                transform.translation.z = z_index.0 as f32;
            }
        }
    }
}

fn handle_layer_point_size_updated_event(
    mut events: MessageReader<rgis_layer_messages::LayerPointSizeUpdatedMessage>,
    layers: Res<rgis_layers::Layers>,
    mut sprite_query: Query<(&mut Sprite, &crate::PointSprite)>,
    camera_query: Query<&GlobalTransform, With<Camera>>,
    index: Res<RenderEntityIndex>,
) {
    let changed_layers: Vec<rgis_primitives::LayerId> =
        events.read().map(|event| event.0).collect();

    if changed_layers.is_empty() {
        return;
    }

    let camera_transform = camera_query.single().unwrap();
    let (camera_scale, _, _) = camera_transform.to_scale_rotation_translation();

    for layer_id in changed_layers {
        let Some(layer) = layers.get(layer_id) else {
            continue;
        };
        for &entity in index.get(layer_id) {
            if let Ok((mut sprite, point_sprite)) = sprite_query.get_mut(entity) {
                sprite.custom_size = Some(
                    camera_scale.truncate() * layer.point_size * point_sprite.relative_scale,
                );
            }
        }
    }
}

fn handle_despawn_meshes_event(
    event: On<rgis_renderer_messages::DespawnMeshesEvent>,
    mut commands: Commands,
    index: Res<RenderEntityIndex>,
) {
    for &entity in index.get(event.0) {
        commands.entity(entity).despawn();
    }
}

fn handle_layer_became_hidden_event(
    event: On<rgis_layer_messages::LayerBecameHiddenEvent>,
    mut query: Query<&mut Visibility>,
    index: Res<RenderEntityIndex>,
) {
    for &entity in index.get(event.0) {
        if let Ok(mut visibility) = query.get_mut(entity) {
            *visibility = Visibility::Hidden;
        }
    }
}

fn handle_layer_became_visible_event(
    event: On<rgis_layer_messages::LayerBecameVisibleEvent>,
    mut query: Query<&mut Visibility>,
    index: Res<RenderEntityIndex>,
) {
    for &entity in index.get(event.0) {
        if let Ok(mut visibility) = query.get_mut(entity) {
            *visibility = Visibility::Visible;
        }
    }
}

fn handle_point_color_updated_event(
    mut event_reader: MessageReader<rgis_layer_messages::LayerColorUpdatedMessage>,
    layers: Res<rgis_layers::Layers>,
    point_layer_query: Query<&Children, With<crate::Point>>,
    mut sprite_fill_query: Query<&mut Sprite, (With<crate::Fill>, Without<crate::Stroke>)>,
    mut sprite_stroke_query: Query<&mut Sprite, (With<crate::Stroke>, Without<crate::Fill>)>,
    index: Res<RenderEntityIndex>,
) {
    for event in event_reader.read() {
        let (layer_id, is_fill) = match event {
            rgis_layer_messages::LayerColorUpdatedMessage::Fill(layer_id) => (layer_id, true),
            rgis_layer_messages::LayerColorUpdatedMessage::Stroke(layer_id) => (layer_id, false),
        };
        let Some(layer) = layers.get(*layer_id) else {
            continue;
        };

        for &entity in index.get(layer.id) {
            if let Ok(children) = point_layer_query.get(entity) {
                for child in children.iter() {
                    if is_fill {
                        if let Ok(mut sprite) = sprite_fill_query.get_mut(child) {
                            sprite.color = layer.color.fill.unwrap();
                        }
                    } else {
                        if let Ok(mut sprite) = sprite_stroke_query.get_mut(child) {
                            sprite.color = layer.color.stroke;
                        }
                    }
                }
            }
        }
    }
}

fn handle_line_string_color_updated_event(
    mut event_reader: MessageReader<rgis_layer_messages::LayerColorUpdatedMessage>,
    layers: Res<rgis_layers::Layers>,
    line_string_layer_query: Query<&Children, With<crate::LineString>>,
    mut material_fill_query: Query<
        &mut MeshMaterial2d<ColorMaterial>,
        (With<crate::Fill>, Without<crate::Stroke>),
    >,
    mut material_stroke_query: Query<
        &mut MeshMaterial2d<ColorMaterial>,
        (With<crate::Stroke>, Without<crate::Fill>),
    >,
    mut materials: ResMut<Assets<ColorMaterial>>,
    index: Res<RenderEntityIndex>,
) {
    for event in event_reader.read() {
        let (layer_id, is_fill) = match event {
            rgis_layer_messages::LayerColorUpdatedMessage::Fill(layer_id) => (layer_id, true),
            rgis_layer_messages::LayerColorUpdatedMessage::Stroke(layer_id) => (layer_id, false),
        };
        let Some(layer) = layers.get(*layer_id) else {
            continue;
        };

        for &entity in index.get(layer.id) {
            if let Ok(children) = line_string_layer_query.get(entity) {
                for child in children.iter() {
                    if is_fill {
                        if let Ok(mut color_material) = material_fill_query.get_mut(child) {
                            materials.get_mut(&mut color_material.0).unwrap().color =
                                layer.color.fill.unwrap();
                        }
                    } else {
                        if let Ok(mut color_material) = material_stroke_query.get_mut(child) {
                            materials.get_mut(&mut color_material.0).unwrap().color =
                                layer.color.stroke;
                        }
                    }
                }
            }
        }
    }
}

fn handle_polygon_color_updated_event(
    mut event_reader: MessageReader<rgis_layer_messages::LayerColorUpdatedMessage>,
    layers: Res<rgis_layers::Layers>,
    polygon_layer_query: Query<&Children, With<crate::Polygon>>,
    mut material_fill_query: Query<
        &mut MeshMaterial2d<ColorMaterial>,
        (With<crate::Fill>, Without<crate::Stroke>),
    >,
    mut material_stroke_query: Query<
        &mut MeshMaterial2d<ColorMaterial>,
        (With<crate::Stroke>, Without<crate::Fill>),
    >,
    mut materials: ResMut<Assets<ColorMaterial>>,
    index: Res<RenderEntityIndex>,
) {
    for event in event_reader.read() {
        let (layer_id, is_fill) = match event {
            rgis_layer_messages::LayerColorUpdatedMessage::Fill(layer_id) => (layer_id, true),
            rgis_layer_messages::LayerColorUpdatedMessage::Stroke(layer_id) => (layer_id, false),
        };
        let Some(layer) = layers.get(*layer_id) else {
            continue;
        };

        for &entity in index.get(layer.id) {
            if let Ok(children) = polygon_layer_query.get(entity) {
                for child in children.iter() {
                    if is_fill {
                        if let Ok(mut color_material) = material_fill_query.get_mut(child) {
                            materials.get_mut(&mut color_material.0).unwrap().color =
                                layer.color.fill.unwrap();
                        }
                    } else {
                        if let Ok(mut color_material) = material_stroke_query.get_mut(child) {
                            materials.get_mut(&mut color_material.0).unwrap().color =
                                layer.color.stroke;
                        }
                    }
                }
            }
        }
    }
}

fn handle_crs_changed_events(
    _event: On<rgis_crs_messages::CrsChangedEvent>,
    query: Query<(&rgis_primitives::LayerId, Entity), With<MeshMaterial2d<ColorMaterial>>>,
    mut commands: Commands,
) {
    // FIXME: there's a race condition here where we'll delete newly generated projected geometry
    // meshes if this gets executed after we project the new geometries. We should add a filter
    // in here for the old CRS.
    for (_, entity) in &query {
        commands.entity(entity).despawn();
    }
}

type CameraGlobalTransformQuery<'world, 'state, 'a> =
    Query<'world, 'state, &'a GlobalTransform, (With<Camera>, Changed<GlobalTransform>)>;

type LayerEntitiesWithColorMaterialsOrImagesQuery<'world, 'state, 'a> = Query<
    'world,
    'state,
    (&'a rgis_primitives::LayerId, Entity),
    Or<(With<MeshMaterial2d<ColorMaterial>>, With<Sprite>)>,
>;

fn handle_camera_scale_changed_event(
    query: CameraGlobalTransformQuery,
    mut sprite_query: Query<(
        &mut Sprite,
        &rgis_primitives::LayerId,
        &crate::PointSprite,
    )>,
    layers: Res<rgis_layers::Layers>,
) {
    if let Ok(camera_global_transform) = query.single() {
        let (scale, _, _) = camera_global_transform.to_scale_rotation_translation();

        for (mut sprite, layer_id, point_sprite) in &mut sprite_query {
            if let Some(layer) = layers.get(*layer_id) {
                sprite.custom_size = Some(
                    scale.truncate() * layer.point_size * point_sprite.relative_scale,
                );
            }
        }
    }
}

type SelectedFeatureQuery<'world, 'state, 'a> = Query<
    'world,
    'state,
    (Entity, &'a RenderEntityType),
    Or<(With<MeshMaterial2d<ColorMaterial>>, With<Sprite>)>,
>;

fn handle_feature_selected_event_despawn(
    event_reader: MessageReader<rgis_map_messages::FeatureSelectedMessage>,
    mut commands: Commands,
    query: SelectedFeatureQuery,
) {
    if !event_reader.is_empty() {
        for (entity, entity_type) in query.iter() {
            match entity_type {
                RenderEntityType::SelectedPolygon
                | RenderEntityType::SelectedLineString
                | RenderEntityType::SelectedPoint => commands.entity(entity).despawn(),
                _ => (),
            }
        }
    }
}

fn handle_feature_selected_event_spawn(
    mut event_reader: MessageReader<rgis_map_messages::FeatureSelectedMessage>,
    layers: Res<rgis_layers::Layers>,
    mut job_spawner: bevy_jobs::JobSpawner,
) {
    for event in event_reader.read() {
        let Some(layer) = layers.get(event.0) else {
            return;
        };
        let Some(feature) = layer.get_projected_feature(event.1) else {
            return;
        };
        let Some(geometry) = feature.geometry.as_ref() else {
            return;
        };
        job_spawner.spawn(MeshBuildingJob {
            layer_id: event.0,
            geometry: geometry.clone(),
            is_selected: true,
        });
    }
}

pub fn configure(app: &mut App) {
    app.add_systems(
        Update,
        (
            layer_loaded,
            handle_point_color_updated_event,
            handle_line_string_color_updated_event,
            handle_polygon_color_updated_event,
            handle_layer_point_size_updated_event,
            handle_layer_z_index_updated_event,
            handle_mesh_building_job_outcome,
            handle_camera_scale_changed_event,
            // Despawn old selection entities before spawning new ones
            (
                handle_feature_selected_event_despawn,
                handle_feature_selected_event_spawn,
            )
                .chain(),
        ),
    );
    app.add_observer(handle_picking_click);
    app.add_observer(handle_layer_became_hidden_event);
    app.add_observer(handle_layer_became_visible_event);
    app.add_observer(handle_despawn_meshes_event);
    app.add_observer(handle_crs_changed_events);
}
