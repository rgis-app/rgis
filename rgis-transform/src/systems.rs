use std::sync::Arc;
use bevy::prelude::*;

fn handle_layer_created_events(
    mut layer_created_event_reader: MessageReader<rgis_layer_messages::LayerCreatedMessage>,
    layers: Res<rgis_layers::Layers>,
    target_crs: Res<rgis_crs::TargetCrs>,
    mut job_spawner: bevy_jobs::JobSpawner,
    geodesy_ctx: Res<rgis_geodesy::GeodesyContext>,
) {
    for event in layer_created_event_reader.read() {
        let Some(layer) = layers.get(event.0) else {
            continue;
        };

        match &layer.data {
            rgis_layers::LayerData::Raster { raster, .. } => {
                job_spawner.spawn(crate::jobs::ReprojectRasterExtentJob {
                    extent: raster.extent,
                    layer_id: event.0,
                    source_crs: layer.crs.clone(),
                    target_crs: target_crs.0.clone(),
                    geodesy_ctx: geodesy_ctx.clone_for_async(),
                });
            }
            rgis_layers::LayerData::Vector { unprojected_feature_collection, .. } => {
                job_spawner.spawn(crate::jobs::ReprojectGeometryJob {
                    feature_collection: Arc::clone(unprojected_feature_collection),
                    layer_id: event.0,
                    source_crs: layer.crs.clone(),
                    target_crs: target_crs.0.clone(),
                    geodesy_ctx: geodesy_ctx.clone_for_async(),
                });
            }
        }
    }
}

fn handle_reproject_geometry_job_completion_events(
    mut finished_jobs: bevy_jobs::FinishedJobs,
    mut job_spawner: bevy_jobs::JobSpawner,
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

        job_spawner.spawn(rgis_file_loader::LODBuilderJob {
            projected: outcome.feature_collection,
            layer_id: outcome.layer_id,
        });
    }
}

fn handle_reproject_raster_extent_job_completion_events(
    mut finished_jobs: bevy_jobs::FinishedJobs,
    mut layers: ResMut<rgis_layers::Layers>,
    mut layer_reprojected_event_writer: MessageWriter<rgis_layer_messages::LayerReprojectedMessage>,
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

        let Some(layer) = layers.get_mut(outcome.layer_id) else {
            continue;
        };

        match &mut layer.data {
            rgis_layers::LayerData::Raster { projected_grid, .. } => {
                *projected_grid = Some(outcome.projected_grid);
            }
            rgis_layers::LayerData::Vector { .. } => {}
        }

        layer_reprojected_event_writer
            .write(rgis_layer_messages::LayerReprojectedMessage(outcome.layer_id));
    }
}

fn handle_crs_changed_events(
    _event: On<rgis_crs_messages::CrsChangedEvent>,
    mut layers: ResMut<rgis_layers::Layers>,
    target_crs: Res<rgis_crs::TargetCrs>,
    mut job_spawner: bevy_jobs::JobSpawner,
    geodesy_ctx: Res<rgis_geodesy::GeodesyContext>,
) {
    layers.clear_projected();

    for layer in layers.iter() {
        match &layer.data {
            rgis_layers::LayerData::Raster { raster, .. } => {
                job_spawner.spawn(crate::jobs::ReprojectRasterExtentJob {
                    extent: raster.extent,
                    layer_id: layer.id,
                    source_crs: layer.crs.clone(),
                    target_crs: target_crs.0.clone(),
                    geodesy_ctx: geodesy_ctx.clone_for_async(),
                });
            }
            rgis_layers::LayerData::Vector { unprojected_feature_collection, .. } => {
                job_spawner.spawn(crate::jobs::ReprojectGeometryJob {
                    feature_collection: Arc::clone(unprojected_feature_collection),
                    layer_id: layer.id,
                    source_crs: layer.crs.clone(),
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
        ),
    );
    app.add_observer(handle_crs_changed_events);
}
