use bevy::prelude::*;

fn handle_toggle_layer_visibility_events(
    mut toggle_layer_visibility_event_reader: MessageReader<
        rgis_layer_messages::ToggleLayerVisibilityMessage,
    >,
    mut commands: Commands,
    mut layers: ResMut<crate::Layers>,
) {
    for event in toggle_layer_visibility_event_reader.read() {
        let Some(layer) = layers.get_mut(event.0) else {
            warn!("Could not find layer");
            continue;
        };
        layer.visible = !layer.visible;
        if layer.visible {
            commands.trigger(rgis_layer_messages::LayerBecameVisibleEvent(event.0));
        } else {
            commands.trigger(rgis_layer_messages::LayerBecameHiddenEvent(event.0));
        }
    }
}

fn handle_update_color_events(
    mut update_events: MessageReader<rgis_ui_messages::UpdateLayerColorMessage>,
    mut updated_events: MessageWriter<rgis_layer_messages::LayerColorUpdatedMessage>,
    mut layers: ResMut<crate::Layers>,
) {
    for event in update_events.read() {
        let event = match event {
            rgis_ui_messages::UpdateLayerColorMessage::Stroke(layer_id, color) => {
                let Some(layer) = layers.get_mut(*layer_id) else {
                    warn!("Could not find layer");
                    continue;
                };
                layer.color.stroke = *color;
                rgis_layer_messages::LayerColorUpdatedMessage::Stroke(*layer_id)
            }
            rgis_ui_messages::UpdateLayerColorMessage::Fill(layer_id, color) => {
                let Some(layer) = layers.get_mut(*layer_id) else {
                    warn!("Could not find layer");
                    continue;
                };
                layer.color.fill = Some(*color);
                rgis_layer_messages::LayerColorUpdatedMessage::Fill(*layer_id)
            }
        };
        updated_events.write(event);
    }
}

fn handle_update_point_size_events(
    mut update_events: MessageReader<rgis_ui_messages::UpdateLayerPointSizeMessage>,
    mut updated_events: MessageWriter<rgis_layer_messages::LayerPointSizeUpdatedMessage>,
    mut layers: ResMut<crate::Layers>,
) {
    for rgis_ui_messages::UpdateLayerPointSizeMessage(layer_id, point_size) in update_events.read() {
        let Some(layer) = layers.get_mut(*layer_id) else {
            warn!("Could not find layer");
            continue;
        };
        layer.point_size = *point_size;
        updated_events.write(rgis_layer_messages::LayerPointSizeUpdatedMessage(*layer_id));
    }
}

fn handle_delete_layer_events(
    mut delete_layer_event_reader: MessageReader<rgis_layer_messages::DeleteLayerMessage>,
    mut commands: Commands,
    mut layers: ResMut<crate::Layers>,
) {
    for event in delete_layer_event_reader.read() {
        layers.remove(event.0);
        commands.trigger(rgis_renderer_messages::DespawnMeshesEvent(event.0));
    }
}

fn handle_move_layer_events(
    mut move_layer_event_reader: MessageReader<rgis_layer_messages::MoveLayerMessage>,
    mut layer_z_index_updated_event_writer: MessageWriter<rgis_layer_messages::LayerZIndexUpdatedMessage>,
    mut layers: ResMut<crate::Layers>,
) {
    for event in move_layer_event_reader.read() {
        let old_z_index = match layers.get_with_index(event.0) {
            Some(result) => result.1 .0,
            None => {
                warn!("Could not find layer");
                continue;
            }
        };

        let new_z_index = match event.1 {
            rgis_layer_messages::MoveDirection::Up => {
                if old_z_index < layers.count() - 1 {
                    old_z_index + 1
                } else {
                    old_z_index
                }
            }
            rgis_layer_messages::MoveDirection::Down => {
                if old_z_index > 0 {
                    old_z_index - 1
                } else {
                    old_z_index
                }
            }
        };
        if new_z_index != old_z_index {
            let Some(other_layer_id) = layers.data.get(new_z_index).map(|l| l.id) else {
                warn!("Could not find layer");
                continue;
            };

            layers.data.swap(old_z_index, new_z_index);

            layer_z_index_updated_event_writer
                .write(rgis_layer_messages::LayerZIndexUpdatedMessage(event.0));
            layer_z_index_updated_event_writer
                .write(rgis_layer_messages::LayerZIndexUpdatedMessage(other_layer_id));
        }
    }
}

fn handle_create_layer_events(
    mut create_layer_events: ResMut<Messages<rgis_layer_messages::CreateLayerMessage>>,
    mut layer_created_event_writer: MessageWriter<rgis_layer_messages::LayerCreatedMessage>,
    mut layers: ResMut<crate::Layers>,
) {
    for event in create_layer_events.drain() {
        let layer_id = layers.add(event.feature_collection, event.name, event.source_crs);
        let Some(layer) = layers.get_mut(layer_id) else {
            continue;
        };
        layer.current_lod = Some(0);
        layer_created_event_writer.write(rgis_layer_messages::LayerCreatedMessage(layer_id));
    }
}

fn handle_create_raster_layer_events(
    mut create_raster_layer_events: ResMut<Messages<rgis_layer_messages::CreateRasterLayerMessage>>,
    mut layer_created_event_writer: MessageWriter<rgis_layer_messages::LayerCreatedMessage>,
    mut layers: ResMut<crate::Layers>,
) {
    for event in create_raster_layer_events.drain() {
        let layer_id = layers.add_raster(event.raster, event.name, event.source_crs);
        layer_created_event_writer.write(rgis_layer_messages::LayerCreatedMessage(layer_id));
    }
}

use crate::LayerWithIndex;

fn handle_duplicate_layer_events(
    mut duplicate_layer_event_reader: MessageReader<rgis_layer_messages::DuplicateLayerMessage>,
    mut create_layer_event_writer: MessageWriter<rgis_layer_messages::CreateLayerMessage>,
    layers: Res<crate::Layers>,
) {
    for event in duplicate_layer_event_reader.read() {
        if let Some(LayerWithIndex(layer, _)) = layers.get_with_index(event.0) {
            if let Some(fc) = layer.unprojected_feature_collection() {
                let new_name = format!("Copy of {}", layer.name);
                create_layer_event_writer.write(rgis_layer_messages::CreateLayerMessage {
                    feature_collection: fc.clone(),
                    name: new_name,
                    source_crs: layer.crs.clone(),
                });
            }
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
        ),
    );
}
