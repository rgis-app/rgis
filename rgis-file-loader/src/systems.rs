use bevy::prelude::*;
use rgis_primitives::Crs;

#[derive(Clone)]
struct FetchUserData {
    file_format: geo_file_loader::FileFormat,
    source_crs: Crs,
}

fn handle_network_fetch_finished_jobs(
    mut load_event_reader: ResMut<Messages<rgis_events::LoadFileMessage>>,
    mut finished_jobs: bevy_jobs::FinishedJobs,
    mut render_message_event_writer: MessageWriter<rgis_ui_messages::RenderTextMessage>,
) {
    while let Some(outcome) =
        finished_jobs.take_next::<bevy_jobs_fetch::NetworkFetchJob<FetchUserData>>()
    {
        match outcome {
            Ok(fetched) => {
                load_event_reader.write(rgis_events::LoadFileMessage::FromBytes {
                    file_format: fetched.user_data.file_format,
                    bytes: fetched.bytes,
                    file_name: fetched.name,
                    source_crs: fetched.user_data.source_crs,
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
    mut load_event_reader: ResMut<Messages<rgis_events::LoadFileMessage>>,
    mut job_spawner: bevy_jobs::JobSpawner,
) {
    for event in load_event_reader.drain() {
        match event {
            rgis_events::LoadFileMessage::FromNetwork {
                url,
                name,
                file_format,
                source_crs,
            } => job_spawner.spawn(bevy_jobs_fetch::NetworkFetchJob {
                url,
                user_data: FetchUserData {
                    file_format,
                    source_crs,
                },
                name,
            }),
            rgis_events::LoadFileMessage::FromBytes {
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
    mut create_layer_event_writer: MessageWriter<rgis_events::CreateLayerMessage>,
    mut create_raster_layer_event_writer: MessageWriter<rgis_events::CreateRasterLayerMessage>,
    mut render_message_event_writer: MessageWriter<rgis_ui_messages::RenderTextMessage>,
    geodesy_ctx: Res<rgis_crs::GeodesyContext>,
) {
    while let Some(outcome) = finished_jobs.take_next::<crate::jobs::LoadFileJob>() {
        match outcome {
            Ok(crate::jobs::LoadFileJobOutcome::Vector {
                feature_collection,
                name,
                source_crs,
            }) => {
                create_layer_event_writer.write(rgis_events::CreateLayerMessage {
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
                            rgis_crs::epsg_code_to_geodesy_op_handle(&mut *ctx, detected_epsg)
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
                    rgis_events::CreateRasterLayerMessage {
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

pub fn configure(app: &mut App) {
    app.add_systems(
        Update,
        (
            handle_network_fetch_finished_jobs,
            handle_load_file_events,
            handle_load_file_job_finished_events,
        )
            .in_set(rgis_primitives::RgisSet::FileLoading),
    );
}
