use bevy::{prelude::*, window::PrimaryWindow};

pub fn configure(app: &mut App) {
    app.add_systems(Startup, init_camera);
    app.add_systems(
        Update,
        (
            center_camera,
            pan_camera_system,
            handle_meshes_spawned_events,
            zoom_camera_system,
            update_lod_based_on_zoom,
        ),
    );
    app.add_observer(handle_change_crs_event);
}

fn update_lod_based_on_zoom(
    mut layers: ResMut<rgis_layers::Layers>,
    mut lod_changed_event_writer: MessageWriter<rgis_layer_messages::LODChangedMessage>,
    query: Query<&Transform, (With<Camera>, Changed<Transform>)>,
) {
    if let Ok(transform) = query.get_single() {
        let scale = transform.scale.x;
        let lod = (scale.log2().abs().round() as u8).clamp(0, 10);
        for layer in layers.iter_mut() {
            if layer.current_lod != Some(lod) {
                layer.current_lod = Some(lod);
                lod_changed_event_writer.write(rgis_layer_messages::LODChangedMessage(layer.id));
            }
        }
    }
}

fn init_camera(mut commands: Commands) {
    commands.spawn((Camera2d, Camera { ..default() }));
}

fn handle_change_crs_event(
    event: On<rgis_crs_messages::CrsChangedEvent>,
    mut query: Query<&mut Transform, With<Camera>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    ui_margins: rgis_units::UiMargins,
    geodesy_ctx: Res<rgis_geodesy::GeodesyContext>,
) -> Result {
    let window = windows.single()?;
    let Ok(mut transform) = query.single_mut() else {
        return Ok(());
    };
    let map_area = rgis_units::MapArea {
        window,
        left_offset_px: ui_margins.left.0,
        right_offset_px: 0.,
        top_offset_px: ui_margins.top.0,
        bottom_offset_px: ui_margins.bottom.0,
    };
    let rect = map_area.projected_geo_rect(&transform, window);

    {
        let geodesy_ctx = geodesy_ctx.read().unwrap();
        let transformer = geo_geodesy::Transformer::from_geodesy(
            &*geodesy_ctx,
            event.old.op_handle,
            event.new.op_handle,
        )?;
        transformer.transform(&mut (rect.into()))?;
    }

    crate::utils::center_camera_on_projected_world_rect(rect, &mut transform, map_area);

    Ok(())
}

fn pan_camera_system(
    mut pan_camera_event_reader: MessageReader<rgis_camera_messages::PanCameraMessage>,
    mut query: Query<&mut Transform, With<Camera>>,
) {
    if pan_camera_event_reader.is_empty() {
        return;
    }
    let Ok(mut transform) = query.single_mut() else {
        return;
    };
    let mut camera_offset = crate::CameraOffset::from_transform(&transform);
    let camera_scale = crate::CameraScale::from_transform(&transform);

    for event in pan_camera_event_reader.read() {
        camera_offset.pan_x(event.x, camera_scale);
        camera_offset.pan_y(event.y, camera_scale);
    }
    crate::utils::set_camera_transform(&mut transform, camera_offset, camera_scale);
}

fn zoom_camera_system(
    mut zoom_camera_event_reader: MessageReader<rgis_camera_messages::ZoomCameraMessage>,
    mut query: Query<&mut Transform, With<Camera>>,
    mut recalculate_mouse_position_event_writer: MessageWriter<
        rgis_camera_messages::RecalculateMousePositionMessage,
    >,
) {
    if zoom_camera_event_reader.is_empty() {
        return;
    }
    let Ok(mut transform) = query.single_mut() else {
        return;
    };
    let mut camera_offset = crate::CameraOffset::from_transform(&transform);
    let mut mouse_offset = camera_offset;
    let before_scale = crate::CameraScale::from_transform(&transform);
    let mut camera_scale = before_scale;
    let mut set = false;
    for event in zoom_camera_event_reader.read() {
        // Set mouse_offset based on the first event's coordinate
        if !set {
            set = true;
            mouse_offset = match crate::CameraOffset::from_coord(event.coord) {
                Ok(offset) => offset,
                Err(e) => {
                    error!("Error creating camera offset: {:?}", e);
                    continue;
                }
            };
        }
        // Adjust the camera scale based on the zoom amount from the event
        camera_scale.zoom(event.amount);
    }

    // Check if the new camera scale is a normal floating-point number
    if camera_scale.0.is_normal() {
        // Calculate the difference between mouse and camera offsets
        let xd = mouse_offset.x - camera_offset.x;
        let yd = mouse_offset.y - camera_offset.y;

        // Adjust the camera offset based on the scale change
        camera_offset.x -= xd * (1.0 - before_scale.0 / camera_scale.0);
        camera_offset.y -= yd * (1.0 - before_scale.0 / camera_scale.0);

        // Ensure the new camera offset values are finite before applying the transform
        if camera_offset.x.is_finite() && camera_offset.y.is_finite() {
            // Update the camera transform with the new offset and scale
            crate::utils::set_camera_transform(&mut transform, camera_offset, camera_scale);
            recalculate_mouse_position_event_writer.write_default();
        }
    }
}

fn handle_meshes_spawned_events(
    mut meshes_spawned_event_reader: MessageReader<rgis_renderer_messages::MeshesSpawnedMessage>,
    mut center_camera_event_writer: MessageWriter<rgis_camera_messages::CenterCameraMessage>,
    mut has_moved: Local<bool>,
) {
    for event in meshes_spawned_event_reader.read() {
        if !(*has_moved) {
            center_camera_event_writer.write(event.0.into());
            *has_moved = true;
        }
    }
}

fn center_camera(
    layers: Res<rgis_layers::Layers>,
    mut event_reader: MessageReader<rgis_camera_messages::CenterCameraMessage>,
    mut query: Query<&mut Transform, With<Camera>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    ui_margins: rgis_units::UiMargins,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    for event in event_reader.read() {
        let Some(layer) = layers.get(event.0) else {
            continue;
        };

        let bounding_rect = if let rgis_layers::LayerData::Raster { projected_grid: Some(grid), .. } = &layer.data {
            use geo_projected::WrapTo;
            geo::Rect::new(
                geo::Coord {
                    x: grid.extent.min().x,
                    y: grid.extent.min().y,
                },
                geo::Coord {
                    x: grid.extent.max().x,
                    y: grid.extent.max().y,
                },
            )
            .wrap::<geo_projected::Projected>()
        } else if let Some(fc) = layer.get_projected_feature_collection_or_log() {
            match fc.bounding_rect() {
                Ok(r) => r,
                Err(_) => continue,
            }
        } else {
            continue;
        };

        let Ok(mut transform) = query.single_mut() else {
            continue;
        };

        debug!("Moving camera to look at new layer");
        let map_area = rgis_units::MapArea {
            window,
            right_offset_px: 0.,
            left_offset_px: ui_margins.left.0,
            bottom_offset_px: ui_margins.bottom.0,
            top_offset_px: ui_margins.top.0,
        };
        crate::utils::center_camera_on_projected_world_rect(
            bounding_rect,
            &mut transform,
            map_area,
        );
    }
}
