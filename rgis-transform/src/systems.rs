use std::sync::Arc;
use bevy::prelude::*;

fn handle_layer_created_events(
    mut layer_created_event_reader: MessageReader<rgis_events::LayerCreatedMessage>,
    id_map: Res<rgis_layers::LayerIdToEntity>,
    layer_query: Query<(&rgis_layers::LayerCrs, &rgis_layers::LayerData)>,
    target_crs: Res<rgis_crs::TargetCrs>,
    mut job_spawner: bevy_jobs::JobSpawner,
    geodesy_ctx: Res<rgis_crs::GeodesyContext>,
) {
    for event in layer_created_event_reader.read() {
        let Some(entity) = id_map.get(event.0) else {
            continue;
        };
        let Ok((crs, data)) = layer_query.get(entity) else {
            continue;
        };

        match data {
            rgis_layers::LayerData::Raster { raster, .. } => {
                job_spawner.spawn(crate::jobs::ReprojectRasterExtentJob {
                    extent: raster.extent,
                    width_px: raster.width,
                    height_px: raster.height,
                    layer_id: event.0,
                    source_crs: crs.0.clone(),
                    target_crs: target_crs.0.clone(),
                    geodesy_ctx: geodesy_ctx.clone_for_async(),
                });
            }
            rgis_layers::LayerData::Vector { unprojected_feature_collection, .. } => {
                job_spawner.spawn(crate::jobs::ReprojectGeometryJob {
                    feature_collection: Arc::clone(unprojected_feature_collection),
                    layer_id: event.0,
                    source_crs: crs.0.clone(),
                    target_crs: target_crs.0.clone(),
                    geodesy_ctx: geodesy_ctx.clone_for_async(),
                });
            }
        }
    }
}

fn handle_reproject_geometry_job_completion_events(
    mut finished_jobs: bevy_jobs::FinishedJobs,
    id_map: Res<rgis_layers::LayerIdToEntity>,
    mut layer_query: Query<&mut rgis_layers::LayerData>,
    mut layer_reprojected_event_writer: MessageWriter<rgis_events::LayerReprojectedMessage>,
    target_crs: Res<rgis_crs::TargetCrs>,
) {
    while let Some(outcome) = finished_jobs.take_next::<crate::jobs::ReprojectGeometryJob>() {
        let outcome = match outcome {
            Ok(o) => o,
            Err(e) => {
                error!("Encountered an error reprojecting geometry: {:?}", e);
                continue;
            }
        };

        if outcome.target_crs != target_crs.0 {
            error!("Encountered a reprojected geometry with a different CRS than the current target CRS");
            continue;
        }

        let Some(entity) = id_map.get(outcome.layer_id) else {
            continue;
        };
        let Ok(mut data) = layer_query.get_mut(entity) else {
            continue;
        };

        match data.as_mut() {
            rgis_layers::LayerData::Vector {
                projected_feature_collection,
                ..
            } => {
                *projected_feature_collection = Some(outcome.feature_collection);
            }
            rgis_layers::LayerData::Raster { .. } => {}
        }

        layer_reprojected_event_writer
            .write(rgis_events::LayerReprojectedMessage(outcome.layer_id));
    }
}

fn handle_reproject_raster_extent_job_completion_events(
    mut finished_jobs: bevy_jobs::FinishedJobs,
    id_map: Res<rgis_layers::LayerIdToEntity>,
    mut layer_query: Query<&mut rgis_layers::LayerData>,
    mut layer_reprojected_event_writer: MessageWriter<rgis_events::LayerReprojectedMessage>,
    target_crs: Res<rgis_crs::TargetCrs>,
) {
    while let Some(outcome) = finished_jobs.take_next::<crate::jobs::ReprojectRasterExtentJob>() {
        let outcome = match outcome {
            Ok(o) => o,
            Err(e) => {
                error!("Encountered an error reprojecting raster extent: {:?}", e);
                continue;
            }
        };

        if outcome.target_crs != target_crs.0 {
            error!("Encountered a reprojected raster extent with a different CRS than the current target CRS");
            continue;
        }

        let Some(entity) = id_map.get(outcome.layer_id) else {
            continue;
        };
        let Ok(mut data) = layer_query.get_mut(entity) else {
            continue;
        };

        match data.as_mut() {
            rgis_layers::LayerData::Raster { projected_grid, .. } => {
                *projected_grid = Some(outcome.projected_grid);
            }
            rgis_layers::LayerData::Vector { .. } => {}
        }

        layer_reprojected_event_writer
            .write(rgis_events::LayerReprojectedMessage(outcome.layer_id));
    }
}

fn handle_crs_changed_events(
    _event: On<rgis_events::CrsChangedEvent>,
    mut layer_query: Query<(&rgis_primitives::LayerId, &rgis_layers::LayerCrs, &mut rgis_layers::LayerData)>,
    target_crs: Res<rgis_crs::TargetCrs>,
    mut job_spawner: bevy_jobs::JobSpawner,
    geodesy_ctx: Res<rgis_crs::GeodesyContext>,
) {
    for (layer_id, crs, mut data) in layer_query.iter_mut() {
        data.clear_projected();

        match data.as_ref() {
            rgis_layers::LayerData::Raster { raster, .. } => {
                job_spawner.spawn(crate::jobs::ReprojectRasterExtentJob {
                    extent: raster.extent,
                    width_px: raster.width,
                    height_px: raster.height,
                    layer_id: *layer_id,
                    source_crs: crs.0.clone(),
                    target_crs: target_crs.0.clone(),
                    geodesy_ctx: geodesy_ctx.clone_for_async(),
                });
            }
            rgis_layers::LayerData::Vector { unprojected_feature_collection, .. } => {
                job_spawner.spawn(crate::jobs::ReprojectGeometryJob {
                    feature_collection: Arc::clone(unprojected_feature_collection),
                    layer_id: *layer_id,
                    source_crs: crs.0.clone(),
                    target_crs: target_crs.0.clone(),
                    geodesy_ctx: geodesy_ctx.clone_for_async(),
                });
            }
        }
    }
}

pub fn configure(app: &mut App) {
    app.add_systems(
        Update,
        (
            handle_layer_created_events,
            handle_reproject_geometry_job_completion_events,
            handle_reproject_raster_extent_job_completion_events,
        )
            .in_set(rgis_primitives::RgisSet::Transform),
    );
    app.add_observer(handle_crs_changed_events);
}
