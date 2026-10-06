use bevy::prelude::*;

fn handle_toggle_layer_visibility_events(
    mut toggle_layer_visibility_event_reader: MessageReader<
        rgis_events::ToggleLayerVisibilityMessage,
    >,
    mut commands: Commands,
    id_map: Res<crate::LayerIdToEntity>,
    mut layer_query: Query<&mut crate::LayerVisible>,
) {
    for event in toggle_layer_visibility_event_reader.read() {
        let Some(entity) = id_map.get(event.0) else {
            warn!("Could not find layer entity for id {:?}", event.0);
            continue;
        };
        let Ok(mut visible) = layer_query.get_mut(entity) else {
            warn!("Could not find LayerVisible component");
            continue;
        };
        visible.0 = !visible.0;
        if visible.0 {
            commands.trigger(rgis_events::LayerBecameVisibleEvent(event.0));
        } else {
            commands.trigger(rgis_events::LayerBecameHiddenEvent(event.0));
        }
    }
}

fn handle_update_color_events(
    mut update_events: MessageReader<rgis_ui_messages::UpdateLayerColorMessage>,
    mut updated_events: MessageWriter<rgis_events::LayerColorUpdatedMessage>,
    id_map: Res<crate::LayerIdToEntity>,
    mut layer_query: Query<&mut crate::LayerColor>,
) {
    for event in update_events.read() {
        let event = match event {
            rgis_ui_messages::UpdateLayerColorMessage::Stroke(layer_id, color) => {
                let Some(entity) = id_map.get(*layer_id) else {
                    warn!("Could not find layer");
                    continue;
                };
                let Ok(mut layer_color) = layer_query.get_mut(entity) else {
                    warn!("Could not find LayerColor component");
                    continue;
                };
                layer_color.stroke = *color;
                rgis_events::LayerColorUpdatedMessage::Stroke(*layer_id)
            }
            rgis_ui_messages::UpdateLayerColorMessage::Fill(layer_id, color) => {
                let Some(entity) = id_map.get(*layer_id) else {
                    warn!("Could not find layer");
                    continue;
                };
                let Ok(mut layer_color) = layer_query.get_mut(entity) else {
                    warn!("Could not find LayerColor component");
                    continue;
                };
                layer_color.fill = Some(*color);
                rgis_events::LayerColorUpdatedMessage::Fill(*layer_id)
            }
        };
        updated_events.write(event);
    }
}

fn handle_update_point_size_events(
    mut update_events: MessageReader<rgis_ui_messages::UpdateLayerPointSizeMessage>,
    mut updated_events: MessageWriter<rgis_events::LayerPointSizeUpdatedMessage>,
    id_map: Res<crate::LayerIdToEntity>,
    mut layer_query: Query<&mut crate::LayerPointSize>,
) {
    for rgis_ui_messages::UpdateLayerPointSizeMessage(layer_id, point_size) in update_events.read() {
        let Some(entity) = id_map.get(*layer_id) else {
            warn!("Could not find layer");
            continue;
        };
        let Ok(mut layer_point_size) = layer_query.get_mut(entity) else {
            warn!("Could not find LayerPointSize component");
            continue;
        };
        layer_point_size.0 = *point_size;
        updated_events.write(rgis_events::LayerPointSizeUpdatedMessage(*layer_id));
    }
}

fn handle_rename_layer_events(
    mut rename_events: MessageReader<rgis_ui_messages::RenameLayerMessage>,
    id_map: Res<crate::LayerIdToEntity>,
    mut layer_query: Query<&mut crate::LayerName>,
) {
    for rgis_ui_messages::RenameLayerMessage(layer_id, new_name) in rename_events.read() {
        let Some(entity) = id_map.get(*layer_id) else {
            warn!("Could not find layer");
            continue;
        };
        let Ok(mut layer_name) = layer_query.get_mut(entity) else {
            warn!("Could not find LayerName component");
            continue;
        };
        layer_name.0.clone_from(new_name);
    }
}

fn handle_delete_layer_events(
    mut delete_layer_event_reader: MessageReader<rgis_events::DeleteLayerMessage>,
    mut id_map: ResMut<crate::LayerIdToEntity>,
    mut layer_order: ResMut<crate::LayerOrder>,
    mut commands: Commands,
) {
    for event in delete_layer_event_reader.read() {
        if let Some(entity) = id_map.get(event.0) {
            layer_order.remove(entity);
            id_map.remove(event.0);
            commands.entity(entity).despawn();
        }
        commands.trigger(rgis_events::DespawnMeshesEvent(event.0));
    }
}

fn handle_move_layer_events(
    mut move_layer_event_reader: MessageReader<rgis_events::MoveLayerMessage>,
    mut layer_z_index_updated_event_writer: MessageWriter<rgis_events::LayerZIndexUpdatedMessage>,
    id_map: Res<crate::LayerIdToEntity>,
    mut layer_order: ResMut<crate::LayerOrder>,
    layer_id_query: Query<&rgis_primitives::LayerId>,
) {
    for event in move_layer_event_reader.read() {
        let Some(entity) = id_map.get(event.0) else {
            warn!("Could not find layer");
            continue;
        };
        let Some(old_z_index) = layer_order.index_of(entity) else {
            warn!("Could not find layer in order");
            continue;
        };

        let new_z_index = match event.1 {
            rgis_events::MoveDirection::Up => {
                if old_z_index < layer_order.count() - 1 {
                    old_z_index + 1
                } else {
                    old_z_index
                }
            }
            rgis_events::MoveDirection::Down => {
                if old_z_index > 0 {
                    old_z_index - 1
                } else {
                    old_z_index
                }
            }
        };
        if new_z_index != old_z_index {
            let Some(other_entity) = layer_order.get(new_z_index) else {
                warn!("Could not find layer at new index");
                continue;
            };
            let Ok(other_layer_id) = layer_id_query.get(other_entity) else {
                warn!("Could not find LayerId for other layer");
                continue;
            };

            layer_order.swap(old_z_index, new_z_index);

            layer_z_index_updated_event_writer
                .write(rgis_events::LayerZIndexUpdatedMessage(event.0));
            layer_z_index_updated_event_writer
                .write(rgis_events::LayerZIndexUpdatedMessage(*other_layer_id));
        }
    }
}

fn handle_create_layer_events(
    mut create_layer_events: ResMut<Messages<rgis_events::CreateLayerMessage>>,
    mut layer_created_event_writer: MessageWriter<rgis_events::LayerCreatedMessage>,
    mut commands: Commands,
    mut layer_order: ResMut<crate::LayerOrder>,
    mut id_map: ResMut<crate::LayerIdToEntity>,
) {
    for event in create_layer_events.drain() {
        let layer_id = rgis_primitives::LayerId::new();
        let geom_type = geo_geom_type::determine(event.feature_collection.geometry_iter());
        let color = crate::make_layer_color(geom_type);
        let z_index = layer_order.count();

        let entity = commands
            .spawn(crate::LayerBundle {
                marker: crate::LayerMarker,
                id: layer_id,
                name: crate::LayerName(event.name),
                visible: crate::LayerVisible(true),
                color,
                crs: crate::LayerCrs(event.source_crs),
                point_size: crate::LayerPointSize(5.0),
                data: crate::LayerData::Vector {
                    unprojected_feature_collection: event.feature_collection,
                    projected_feature_collection: None,
                    geom_type,
                },
                z_index: crate::LayerZIndex(z_index),
            })
            .id();

        layer_order.push(entity);
        id_map.insert(layer_id, entity);
        layer_created_event_writer.write(rgis_events::LayerCreatedMessage(layer_id));
    }
}

fn handle_create_raster_layer_events(
    mut create_raster_layer_events: ResMut<Messages<rgis_events::CreateRasterLayerMessage>>,
    mut layer_created_event_writer: MessageWriter<rgis_events::LayerCreatedMessage>,
    mut commands: Commands,
    mut layer_order: ResMut<crate::LayerOrder>,
    mut id_map: ResMut<crate::LayerIdToEntity>,
) {
    for event in create_raster_layer_events.drain() {
        let layer_id = rgis_primitives::LayerId::new();
        let z_index = layer_order.count();

        let entity = commands
            .spawn(crate::LayerBundle {
                marker: crate::LayerMarker,
                id: layer_id,
                name: crate::LayerName(event.name),
                visible: crate::LayerVisible(true),
                color: crate::LayerColor {
                    fill: None,
                    stroke: Color::WHITE,
                },
                crs: crate::LayerCrs(event.source_crs),
                point_size: crate::LayerPointSize(5.0),
                data: crate::LayerData::Raster {
                    raster: event.raster,
                    projected_grid: None,
                },
                z_index: crate::LayerZIndex(z_index),
            })
            .id();

        layer_order.push(entity);
        id_map.insert(layer_id, entity);
        layer_created_event_writer.write(rgis_events::LayerCreatedMessage(layer_id));
    }
}

fn handle_duplicate_layer_events(
    mut duplicate_layer_event_reader: MessageReader<rgis_events::DuplicateLayerMessage>,
    mut create_layer_event_writer: MessageWriter<rgis_events::CreateLayerMessage>,
    id_map: Res<crate::LayerIdToEntity>,
    layer_query: Query<(&crate::LayerName, &crate::LayerCrs, &crate::LayerData)>,
) {
    for event in duplicate_layer_event_reader.read() {
        let Some(entity) = id_map.get(event.0) else {
            warn!("Could not find layer to duplicate");
            continue;
        };
        let Ok((name, crs, data)) = layer_query.get(entity) else {
            continue;
        };
        if let Some(fc) = data.unprojected_feature_collection() {
            let new_name = format!("Copy of {}", name.0);
            create_layer_event_writer.write(rgis_events::CreateLayerMessage {
                feature_collection: fc.clone(),
                name: new_name,
                source_crs: crs.0.clone(),
            });
        }
    }
}

fn track_selected_feature(
    mut feature_selected_reader: MessageReader<rgis_events::FeatureSelectedMessage>,
    mut features_deselected_reader: MessageReader<rgis_events::FeaturesDeselectedMessage>,
    mut delete_layer_reader: MessageReader<rgis_events::DeleteLayerMessage>,
    mut selected_feature: ResMut<crate::SelectedFeature>,
) {
    if features_deselected_reader.read().next().is_some() {
        selected_feature.0 = None;
    }
    if let Some(event) = feature_selected_reader.read().last() {
        selected_feature.0 = Some((event.0, event.1));
    }
    for event in delete_layer_reader.read() {
        if selected_feature.0.is_some_and(|(layer_id, _)| layer_id == event.0) {
            selected_feature.0 = None;
        }
    }
}

pub fn configure(app: &mut App) {
    app.add_systems(
        Update,
        (
            handle_toggle_layer_visibility_events,
            handle_update_color_events,
            handle_update_point_size_events,
            handle_rename_layer_events,
            handle_move_layer_events,
            handle_delete_layer_events,
            // handle_duplicate_layer_events writes CreateLayerMessage events,
            // so it must run before handle_create_layer_events which drains them
            (
                handle_duplicate_layer_events,
                handle_create_layer_events,
            )
                .chain(),
            handle_create_raster_layer_events,
            track_selected_feature,
        )
            .in_set(rgis_primitives::RgisSet::LayerProcessing),
    );
}
