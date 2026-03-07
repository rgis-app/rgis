use bevy::prelude::*;
use rgis_primitives::Crs;

#[derive(Clone)]
struct SourceCrs(Crs);

fn handle_network_fetch_finished_jobs(
    mut load_event_reader: ResMut<Messages<rgis_file_loader_messages::LoadFileMessage>>,
    mut finished_jobs: bevy_jobs::FinishedJobs,
    mut render_message_event_writer: MessageWriter<rgis_ui_messages::RenderTextMessage>,
) {
    while let Some(outcome) =
        finished_jobs.take_next::<bevy_jobs_fetch::NetworkFetchJob<SourceCrs>>()
    {
        match outcome {
            Ok(fetched) => {
                load_event_reader.write(rgis_file_loader_messages::LoadFileMessage::FromBytes {
                    file_format: geo_file_loader::FileFormat::GeoJson,
                    bytes: fetched.bytes,
                    file_name: fetched.name,
                    source_crs: fetched.user_data.0,
                });
            }
            Err(e) => {
                let msg = format!("Could not fetch file: {e}");
                error!("{msg}");
                render_message_event_writer
                    .write(rgis_ui_messages::RenderTextMessage(msg));
            }
        }
    }
}

fn handle_load_file_events(
    mut load_event_reader: ResMut<Messages<rgis_file_loader_messages::LoadFileMessage>>,
    mut job_spawner: bevy_jobs::JobSpawner,
) {
    for event in load_event_reader.drain() {
        match event {
            rgis_file_loader_messages::LoadFileMessage::FromNetwork {
                url,
                name,
                source_crs,
            } => job_spawner.spawn(bevy_jobs_fetch::NetworkFetchJob {
                url,
                user_data: SourceCrs(source_crs),
                name,
            }),
            rgis_file_loader_messages::LoadFileMessage::FromBytes {
                file_name,
                bytes,
                file_format,
                source_crs,
            } => job_spawner.spawn(crate::jobs::LoadFileJob {
                source_crs,
                name: file_name,
                bytes,
                file_format,
            }),
        };
    }
}

fn handle_load_file_job_finished_events(
    mut finished_jobs: bevy_jobs::FinishedJobs,
    mut create_layer_event_writer: MessageWriter<rgis_layer_messages::CreateLayerMessage>,
    mut create_raster_layer_event_writer: MessageWriter<rgis_layer_messages::CreateRasterLayerMessage>,
    mut render_message_event_writer: MessageWriter<rgis_ui_messages::RenderTextMessage>,
    geodesy_ctx: Res<rgis_geodesy::GeodesyContext>,
) {
    while let Some(outcome) = finished_jobs.take_next::<crate::jobs::LoadFileJob>() {
        match outcome {
            Ok(crate::jobs::LoadFileJobOutcome::Vector {
                feature_collection,
                name,
                source_crs,
            }) => {
                create_layer_event_writer.write(rgis_layer_messages::CreateLayerMessage {
                    name,
                    feature_collection: std::sync::Arc::new(feature_collection),
                    source_crs,
                });
            }
            Ok(crate::jobs::LoadFileJobOutcome::Raster {
                raster,
                name,
                mut source_crs,
            }) => {
                // Override source CRS with the EPSG code detected from GeoTIFF metadata
                if let Some(detected_epsg) = raster.epsg_code {
                    if Some(detected_epsg) != source_crs.epsg_code {
                        let mut ctx = geodesy_ctx.write().unwrap();
                        if let Ok(op_handle) =
                            rgis_geodesy::epsg_code_to_geodesy_op_handle(&mut *ctx, detected_epsg)
                        {
                            source_crs = Crs {
                                epsg_code: Some(detected_epsg),
                                proj_string: None,
                                op_handle,
                            };
                        }
                    }
                }
                create_raster_layer_event_writer.write(
                    rgis_layer_messages::CreateRasterLayerMessage {
                        raster,
                        name,
                        source_crs,
                    },
                );
            }
            Err(e) => {
                let msg = format!("Error loading file: {e}");
                error!("{msg}");
                render_message_event_writer
                    .write(rgis_ui_messages::RenderTextMessage(msg));
            }
        }
    }
}

fn handle_lod_builder_job_finished_events(
    mut finished_jobs: bevy_jobs::FinishedJobs,
    mut layers: ResMut<rgis_layers::Layers>,
    mut layer_reprojected_event_writer: EventWriter<rgis_events::LayerReprojectedEvent>,
) {
    while let Some(outcome) = finished_jobs.take_next::<crate::lod_builder::LODBuilderJob>() {
        match outcome {
            Ok(outcome) => {
                let Some(layer) = layers.get_mut(outcome.layer_id) else {
                    continue;
                };
                layer.projected_feature_collection = Some(outcome.lods);
                layer_reprojected_event_writer
                    .write(rgis_events::LayerReprojectedEvent(outcome.layer_id));
            }
            Err(e) => {
                bevy::log::error!("Encountered error when building LODs: {:?}", e);
            }
        }
    }
}

pub fn configure(app: &mut App) {
    app.add_systems(
        Update,
        (
            handle_network_fetch_finished_jobs,
            handle_load_file_events,
            handle_load_file_job_finished_events,
            handle_lod_builder_job_finished_events,
        ),
    );
}
