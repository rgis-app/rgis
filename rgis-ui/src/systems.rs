use std::sync::Arc;
use bevy::{ecs::query::QueryIter, prelude::*, window::PrimaryWindow};
use bevy_egui::{
    egui::{self, Widget},
    EguiContexts, EguiPrimaryContextPass, EguiTextureHandle,
};
use bevy_egui_window::Window;
use geo::{Distance, Geodesic, Haversine, Rhumb};

use crate::windows::add_layer::file::{OpenFileJob, SelectedFile};

fn render_bottom(
    mut bevy_egui_ctx: EguiContexts,
    mouse_pos: Res<rgis_mouse::MousePos>,
    target_crs: Res<rgis_crs::TargetCrs>,
    mut open_change_crs_window_event_writer: MessageWriter<rgis_ui_messages::OpenChangeCrsWindowMessage>,
    mut bottom_panel_height: ResMut<rgis_units::BottomPanelHeight>,
) -> Result {
    let bevy_egui_ctx_mut = bevy_egui_ctx.ctx_mut()?;
    crate::panels::bottom::Bottom {
        egui_ctx: bevy_egui_ctx_mut,
        mouse_pos: &mouse_pos,
        target_crs: &target_crs,
        open_change_crs_window_event_writer: &mut open_change_crs_window_event_writer,
        bottom_panel_height: &mut bottom_panel_height,
    }
    .render();
    Ok(())
}

fn render_side(
    mut bevy_egui_ctx: EguiContexts,
    layer_order: Res<rgis_layers::LayerOrder>,
    layer_query: Query<(
        Entity,
        &rgis_primitives::LayerId,
        &rgis_layers::LayerName,
        &rgis_layers::LayerVisible,
        &rgis_layers::LayerColor,
        &rgis_layers::LayerPointSize,
        &rgis_layers::LayerData,
        &rgis_layers::LayerCrs,
    )>,
    mut events: crate::panels::side::Events,
    mut side_panel_width: ResMut<rgis_units::SidePanelWidth>,
) -> Result {
    let bevy_egui_ctx_mut = bevy_egui_ctx.ctx_mut()?;

    // Build snapshots in top-to-bottom order to avoid passing Query across lifetime boundaries
    let snapshots: Vec<crate::panels::side::LayerSnapshot> = layer_order
        .iter_top_to_bottom()
        .filter_map(|entity| {
            let (_entity, layer_id, name, visible, color, _point_size, data, crs) =
                layer_query.get(entity).ok()?;
            Some(crate::panels::side::LayerSnapshot {
                layer_id: *layer_id,
                name: name.0.clone(),
                visible: visible.0,
                color: color.clone(),
                is_vector: data.is_vector(),
                is_active: data.is_active(),
                geom_type: data.geom_type(),
                crs: crs.clone(),
                unprojected_fc: data.unprojected_feature_collection().cloned(),
            })
        })
        .collect();

    crate::panels::side::Side {
        egui_ctx: bevy_egui_ctx_mut,
        snapshots,
        events: &mut events,
        side_panel_width: &mut side_panel_width,
    }
    .render();
    Ok(())
}

fn handle_open_file_job(
    mut finished_jobs: bevy_jobs::FinishedJobs,
    mut selected_file: ResMut<SelectedFile>,
) {
    while let Some(outcome) = finished_jobs.take_next::<OpenFileJob>().flatten() {
        selected_file.0 = Some(outcome);
    }
}

fn handle_download_layer(
    mut events: MessageReader<rgis_events::DownloadLayerMessage>,
    id_map: Res<rgis_layers::LayerIdToEntity>,
    layer_query: Query<(&rgis_layers::LayerName, &rgis_layers::LayerData)>,
    mut job_spawner: bevy_jobs::JobSpawner,
) {
    for event in events.read() {
        let Some(entity) = id_map.get(event.layer_id) else {
            warn!("Could not find layer for download");
            continue;
        };
        let Ok((name, data)) = layer_query.get(entity) else {
            warn!("Could not find layer for download");
            continue;
        };

        let Some(fc) = data.unprojected_feature_collection() else {
            warn!("Cannot download raster layer");
            continue;
        };

        match rgis_layers::export::export_feature_collection(fc, event.format) {
            Ok(data) => {
                let default_name = format!("{}.{}", name.0, event.format.extension());
                job_spawner.spawn(crate::save_file::SaveFileJob {
                    data: data.into_bytes(),
                    default_name,
                    format: event.format,
                });
            }
            Err(e) => {
                error!("Failed to export layer: {}", e);
            }
        }
    }
}

fn handle_save_file_job(mut finished_jobs: bevy_jobs::FinishedJobs) {
    while let Some(outcome) = finished_jobs.take_next::<crate::save_file::SaveFileJob>() {
        if let Err(e) = outcome {
            error!("Failed to save file: {}", e);
        }
    }
}

fn render_manage_layer_window(
    mut state: Local<crate::ManageLayerWindowState>,
    mut bevy_egui_ctx: EguiContexts,
    id_map: Res<rgis_layers::LayerIdToEntity>,
    layer_query: Query<(
        &rgis_layers::LayerName,
        &rgis_layers::LayerColor,
        &rgis_layers::LayerPointSize,
        &rgis_layers::LayerData,
        &rgis_layers::LayerCrs,
    )>,
    mut color_events: ResMut<Messages<rgis_ui_messages::UpdateLayerColorMessage>>,
    mut point_size_events: ResMut<Messages<rgis_ui_messages::UpdateLayerPointSizeMessage>>,
    mut show_manage_layer_window_event_reader: MessageReader<
        rgis_ui_messages::ShowManageLayerWindowMessage,
    >,
    mut duplicate_layer_events: ResMut<Messages<rgis_events::DuplicateLayerMessage>>,
    mut rename_events: ResMut<Messages<rgis_ui_messages::RenameLayerMessage>>,
    mut name_edit_buffer: Local<String>,
    mut name_edit_layer_id: Local<Option<rgis_primitives::LayerId>>,
    side_panel_width: Res<rgis_units::SidePanelWidth>,
    top_panel_height: Res<rgis_units::TopPanelHeight>,
) -> Result {
    if let Some(event) = show_manage_layer_window_event_reader.read().last() {
        *state = Some(event.0);
    }

    let Some(layer_id) = *state else {
        return Ok(());
    };

    let Some(entity) = id_map.get(layer_id) else {
        *state = None;
        return Ok(());
    };
    let Ok((name, color, point_size, data, crs)) = layer_query.get(entity) else {
        *state = None;
        return Ok(());
    };

    // Initialize or reset the edit buffer when switching layers
    if *name_edit_layer_id != Some(layer_id) {
        *name_edit_layer_id = Some(layer_id);
        name_edit_buffer.clone_from(&name.0);
    }

    let bevy_egui_ctx_mut = bevy_egui_ctx.ctx_mut()?;
    let default_pos = egui::pos2(side_panel_width.0 + 4.0, top_panel_height.0 + 4.0);
    let mut is_open = true;
    egui::Window::new("Manage Layer")
        .default_pos(default_pos)
        .open(&mut is_open)
        .show(bevy_egui_ctx_mut, |ui| {
            crate::windows::manage_layer::ManageLayer {
                layer_id,
                name,
                color,
                point_size,
                data,
                crs,
                color_events: &mut color_events,
                point_size_events: &mut point_size_events,
                duplicate_layer_events: &mut duplicate_layer_events,
                rename_events: &mut rename_events,
                name_edit_buffer: &mut name_edit_buffer,
            }
            .render(ui);
        });

    if !is_open {
        *state = None;
    }
    Ok(())
}

fn render_add_layer_window(
    mut is_visible: Local<bool>,
    mut selected_file: ResMut<SelectedFile>,
    mut bevy_egui_ctx: EguiContexts,
    mut job_spawner: bevy_jobs::JobSpawner,
    mut state: Local<crate::windows::add_layer::State>,
    mut events: crate::windows::add_layer::Events,
    geodesy_ctx: Res<rgis_crs::GeodesyContext>,
    side_panel_width: Res<rgis_units::SidePanelWidth>,
    top_panel_height: Res<rgis_units::TopPanelHeight>,
) -> Result {
    if crate::widget_registry::take_close_request("Add Layer") {
        state.reset();
        *is_visible = false;
        return Ok(());
    }

    if !events.show_add_layer_window_event_reader.is_empty() {
        *is_visible = true;
    }

    if !events.hide_add_layer_window_events.is_empty() {
        state.reset();
        *is_visible = false;
    }

    let mut output = None;

    if *is_visible {
        let bevy_egui_ctx_mut = bevy_egui_ctx.ctx_mut()?;
        let default_pos = egui::pos2(side_panel_width.0 + 4.0, top_panel_height.0 + 4.0);
        egui::Window::new("Add Layer")
            .resizable(false)
            .default_pos(default_pos)
            .open(&mut is_visible)
            .show(bevy_egui_ctx_mut, |ui| {
                output = crate::windows::add_layer::AddLayer {
                    state: &mut state,
                    selected_file: &mut selected_file,
                    geodesy_ctx: &geodesy_ctx,
                }
                .render(ui);
            });
    } else {
        state.reset();
    }

    if let Some(output) = output {
        use crate::windows::add_layer::AddLayerOutput;
        match output {
            AddLayerOutput::LoadFromText {
                text,
                file_format,
                source_crs,
            } => {
                events.load_file_event_writer.write(
                    rgis_events::LoadFileMessage::FromBytes {
                        file_name: "Inputted file".into(),
                        file_format,
                        bytes: text.into(),
                        source_crs,
                    },
                );
                events.hide_add_layer_window_events.write_default();
                state.reset();
            }
            AddLayerOutput::LoadFromFile {
                file_name,
                file_format,
                bytes,
                source_crs,
            } => {
                events.load_file_event_writer.write(
                    rgis_events::LoadFileMessage::FromBytes {
                        file_name,
                        file_format,
                        bytes: bytes.into(),
                        source_crs,
                    },
                );
                events.hide_add_layer_window_events.write_default();
                state.reset();
            }
            AddLayerOutput::LoadFromLibrary {
                name,
                url,
                source_crs,
            } => {
                events.load_file_event_writer.write(
                    rgis_events::LoadFileMessage::FromNetwork {
                        name,
                        url,
                        source_crs,
                    },
                );
                events.hide_add_layer_window_events.write_default();
                state.reset();
            }
            AddLayerOutput::OpenFile => {
                job_spawner.spawn(OpenFileJob);
            }
        }
    }

    Ok(())
}

fn handle_open_change_crs_window_event(
    mut events: MessageReader<rgis_ui_messages::OpenChangeCrsWindowMessage>,
    mut is_visible: ResMut<crate::ChangeCrsWindowVisible>,
) {
    if events.read().last().is_some() {
        is_visible.0 = true;
    }
}

fn render_change_crs_window(
    mut is_visible: ResMut<crate::ChangeCrsWindowVisible>,
    target_crs: Res<rgis_crs::TargetCrs>,
    mut bevy_egui_ctx: EguiContexts,
    mut text_field_value: Local<String>,
    mut crs_input_mode: Local<crate::widgets::crs_input::CrsInputMode>,
    mut change_crs_event_writer: MessageWriter<rgis_events::ChangeCrsMessage>,
    mut crs_input_outcome: Local<Option<crate::widgets::crs_input::Outcome>>,
    geodesy_ctx: Res<rgis_crs::GeodesyContext>,
    side_panel_width: Res<rgis_units::SidePanelWidth>,
    top_panel_height: Res<rgis_units::TopPanelHeight>,
) -> Result {
    if crate::widget_registry::take_close_request("Change CRS") {
        is_visible.0 = false;
        return Ok(());
    }

    if !is_visible.0 {
        return Ok(());
    }

    let bevy_egui_ctx_mut = bevy_egui_ctx.ctx_mut()?;
    let default_pos = egui::pos2(side_panel_width.0 + 4.0, top_panel_height.0 + 4.0);
    egui::Window::new("Change CRS")
        .default_pos(default_pos)
        .open(&mut is_visible.0)
        .show(bevy_egui_ctx_mut, |ui| {
            crate::windows::change_crs::ChangeCrs {
                text_field_value: &mut text_field_value,
                crs_input_mode: &mut crs_input_mode,
                change_crs_event_writer: &mut change_crs_event_writer,
                target_crs: (*target_crs).clone(),
                crs_input_outcome: &mut crs_input_outcome,
                geodesy_ctx: &geodesy_ctx,
            }
            .render(ui);
        });
    Ok(())
}

fn render_feature_properties_window(
    mut state: Local<crate::FeaturePropertiesWindowState>,
    mut bevy_egui_ctx: EguiContexts,
    id_map: Res<rgis_layers::LayerIdToEntity>,
    layer_name_query: Query<&rgis_layers::LayerName>,
    mut render_message_events: ResMut<Messages<rgis_ui_messages::RenderFeaturePropertiesMessage>>,
    side_panel_width: Res<rgis_units::SidePanelWidth>,
    top_panel_height: Res<rgis_units::TopPanelHeight>,
) -> Result {
    if let Some(event) = render_message_events.drain().last() {
        if let Some(properties) = event.properties {
            *state = Some(crate::FeaturePropertiesWindowData {
                layer_id: event.layer_id,
                properties,
            });
        }
    }

    let Some(ref data) = *state else {
        return Ok(());
    };

    let Some(entity) = id_map.get(data.layer_id) else {
        return Ok(());
    };
    let Ok(layer_name) = layer_name_query.get(entity) else {
        return Ok(());
    };

    let bevy_egui_ctx_mut = bevy_egui_ctx.ctx_mut()?;
    let default_pos = egui::pos2(side_panel_width.0 + 4.0, top_panel_height.0 + 4.0);
    let properties = &data.properties;
    let mut is_open = true;
    egui::Window::new("Layer Feature Properties")
        .id(egui::Id::new("Layer Feature Properties Window"))
        .default_pos(default_pos)
        .open(&mut is_open)
        .show(bevy_egui_ctx_mut, |ui| {
            crate::windows::feature_properties::FeatureProperties {
                layer_name: &layer_name.0,
                properties,
            }
            .render(ui);
        });

    if !is_open {
        *state = None;
    }
    Ok(())
}

fn render_attribute_table_window(
    mut state: Local<crate::AttributeTableWindowState>,
    mut bevy_egui_ctx: EguiContexts,
    id_map: Res<rgis_layers::LayerIdToEntity>,
    layer_query: Query<(&rgis_layers::LayerName, &rgis_layers::LayerData)>,
    mut show_events: MessageReader<rgis_ui_messages::ShowAttributeTableMessage>,
    side_panel_width: Res<rgis_units::SidePanelWidth>,
    top_panel_height: Res<rgis_units::TopPanelHeight>,
    mut center_camera_on_feature_writer: MessageWriter<rgis_events::CenterCameraOnFeatureMessage>,
    mut feature_selected_writer: MessageWriter<rgis_events::FeatureSelectedMessage>,
) -> Result {
    if let Some(event) = show_events.read().last() {
        *state = Some(event.0);
    }

    let Some(layer_id) = *state else {
        return Ok(());
    };

    let Some(entity) = id_map.get(layer_id) else {
        *state = None;
        return Ok(());
    };
    let Ok((name, data)) = layer_query.get(entity) else {
        *state = None;
        return Ok(());
    };

    let bevy_egui_ctx_mut = bevy_egui_ctx.ctx_mut()?;
    let default_pos = egui::pos2(side_panel_width.0 + 4.0, top_panel_height.0 + 40.0);
    let mut is_open = true;
    let mut action = None;
    egui::Window::new(format!("Attribute Table: {}", name.0))
        .id(egui::Id::new("Attribute Table Window"))
        .default_pos(default_pos)
        .default_size([600.0, 400.0])
        .resizable(true)
        .open(&mut is_open)
        .show(bevy_egui_ctx_mut, |ui| {
            action = crate::windows::attribute_table::AttributeTable { data }.render(ui);
        });

    match action {
        Some(crate::windows::attribute_table::AttributeTableAction::ZoomToFeature(feature_id)) => {
            center_camera_on_feature_writer.write(rgis_events::CenterCameraOnFeatureMessage(layer_id, feature_id));
        }
        Some(crate::windows::attribute_table::AttributeTableAction::SelectFeature(feature_id)) => {
            feature_selected_writer.write(rgis_events::FeatureSelectedMessage(layer_id, feature_id));
        }
        None => {}
    }

    if !is_open {
        *state = None;
    }
    Ok(())
}

fn render_message_window(
    mut state: Local<crate::MessageWindowState>,
    mut bevy_egui_ctx: EguiContexts,
    mut render_message_events: ResMut<Messages<rgis_ui_messages::RenderTextMessage>>,
) -> Result {
    let bevy_egui_ctx_mut = bevy_egui_ctx.ctx_mut()?;
    if let Some(event) = render_message_events.drain().last() {
        *state = Some(event.0);
    }

    crate::windows::message::Message {
        state: &mut state,
        egui_ctx: bevy_egui_ctx_mut,
    }
    .render();
    Ok(())
}

fn render_operation_window(
    mut state: Local<crate::OperationWindowState>,
    mut events: ResMut<Messages<rgis_ui_messages::OpenOperationWindowMessage>>,
    mut bevy_egui_ctx: EguiContexts,
    create_layer_event_writer: MessageWriter<rgis_events::CreateLayerMessage>,
    render_message_event_writer: MessageWriter<rgis_ui_messages::RenderTextMessage>,
    side_panel_width: Res<rgis_units::SidePanelWidth>,
    top_panel_height: Res<rgis_units::TopPanelHeight>,
) -> Result {
    let bevy_egui_ctx_mut = bevy_egui_ctx.ctx_mut()?;
    if let Some(event) = events.drain().last() {
        *state = Some(crate::OperationWindowData {
            operation: event.operation,
            feature_collection: event.feature_collection,
            source_crs: None,
            layer_name: event.layer_name,
        });
    }

    if state.is_some() {
        let default_pos = egui::pos2(side_panel_width.0 + 4.0, top_panel_height.0 + 4.0);
        let mut is_open = true;
        egui::Window::new("Operation")
            .default_pos(default_pos)
            .open(&mut is_open)
            .show(bevy_egui_ctx_mut, |ui| {
                crate::windows::operation::Operation {
                    state: &mut state,
                    create_layer_event_writer,
                    render_message_event_writer,
                }
                .render(ui);
            });
        if !is_open {
            *state = None;
        }
    }
    Ok(())
}

fn render_in_progress(
    query: Query<&bevy_jobs::InProgressJob>,
    mut bevy_egui_ctx: EguiContexts,
) -> Result {
    let bevy_egui_ctx_mut = bevy_egui_ctx.ctx_mut()?;
    let mut in_progress_job_iter: std::iter::Peekable<
        QueryIter<'_, '_, &bevy_jobs::InProgressJob, ()>,
    > = query.iter().peekable();

    if in_progress_job_iter.peek().is_none() {
        return Ok(());
    }

    egui::Window::new("Running jobs")
        .open(&mut true)
        .title_bar(false)
        .anchor(egui::Align2::RIGHT_BOTTOM, [-5., -5.])
        .resizable(false)
        .show(bevy_egui_ctx_mut, |ui| {
            for in_progress_job in in_progress_job_iter {
                ui.add(InProgressJobWidget { in_progress_job });
            }
        });
    Ok(())
}

struct InProgressJobWidget<'a> {
    in_progress_job: &'a bevy_jobs::InProgressJob,
}

impl Widget for InProgressJobWidget<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let Self { in_progress_job } = self;

        let name = &in_progress_job.name;
        let progress = in_progress_job.progress;

        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new());
            if progress > 0 {
                egui::ProgressBar::new(f32::from(progress) / 100.)
                    .desired_width(200.)
                    .text(format!("Running '{name}'"))
                    .ui(ui);
            } else {
                ui.label(format!("Running '{name}'"));
            }
        })
        .response
    }
}

#[derive(Default)]
struct LogoTextures {
    light: Option<(Handle<Image>, egui::TextureId)>,
    dark: Option<(Handle<Image>, egui::TextureId)>,
}

fn render_top(
    mut bevy_egui_ctx: EguiContexts,
    mut app_exit_events: ResMut<Messages<AppExit>>,
    mut windows: Query<&mut bevy::window::Window, With<PrimaryWindow>>,
    mut app_settings: ResMut<rgis_settings::RgisSettings>,
    current_tool: Res<State<rgis_settings::Tool>>,
    mut next_tool: ResMut<NextState<rgis_settings::Tool>>,
    mut top_panel_height: ResMut<rgis_units::TopPanelHeight>,
    mut is_debug_window_open: ResMut<
        bevy_egui_window::IsWindowOpen<crate::windows::debug::Debug<'static, 'static>>,
    >,
    mut is_logs_window_open: ResMut<
        bevy_egui_window::IsWindowOpen<crate::windows::logs::Logs<'static>>,
    >,
    mut show_add_layer_window_event_writer: MessageWriter<rgis_ui_messages::ShowAddLayerWindowMessage>,
    mut clear_color: ResMut<ClearColor>,
    asset_server: Res<AssetServer>,
    mut logo_textures: Local<LogoTextures>,
) -> Result {
    if logo_textures.light.is_none() {
        let handle: Handle<Image> = asset_server.load("logo-black.png");
        let texture_id = bevy_egui_ctx.add_image(EguiTextureHandle::Strong(handle.clone()));
        logo_textures.light = Some((handle, texture_id));
    }
    if logo_textures.dark.is_none() {
        let handle: Handle<Image> = asset_server.load("logo-white.png");
        let texture_id = bevy_egui_ctx.add_image(EguiTextureHandle::Strong(handle.clone()));
        logo_textures.dark = Some((handle, texture_id));
    }

    let logo_texture_id = if app_settings.dark_mode {
        logo_textures.dark.as_ref()
    } else {
        logo_textures.light.as_ref()
    }
    .filter(|(handle, _)| asset_server.is_loaded_with_dependencies(handle))
    .map(|(_, id)| *id);

    let bevy_egui_ctx_mut = bevy_egui_ctx.ctx_mut()?;
    let Ok(mut window) = windows.single_mut() else {
        return Ok(());
    };

    crate::panels::top::Top {
        egui_ctx: bevy_egui_ctx_mut,
        app_exit_events: &mut app_exit_events,
        window: &mut window,
        app_settings: &mut app_settings,
        current_tool: current_tool.get(),
        next_tool: &mut next_tool,
        top_panel_height: &mut top_panel_height,
        is_debug_window_open: &mut is_debug_window_open,
        is_logs_window_open: &mut is_logs_window_open,
        show_add_layer_window_event_writer: &mut show_add_layer_window_event_writer,
        clear_color: &mut clear_color,
        logo_texture_id,
    }
    .render();
    Ok(())
}

fn set_egui_theme(
    mut bevy_egui_ctx: EguiContexts,
    mut clear_color: ResMut<ClearColor>,
    app_settings: Res<rgis_settings::RgisSettings>,
) -> Result {
    let bevy_egui_ctx_mut = bevy_egui_ctx.ctx_mut()?;
    let dark_mode = app_settings.dark_mode;
    let egui_visuals = if dark_mode {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    // Set the background color of the map
    clear_color.0 = egui_color_to_bevy_color(egui_visuals.extreme_bg_color);
    // Set the egui theme
    bevy_egui_ctx_mut.set_visuals(egui_visuals);
    Ok(())
}

pub fn egui_color_to_bevy_color(egui_color: bevy_egui::egui::Color32) -> Color {
    Color::srgb_u8(egui_color.r(), egui_color.g(), egui_color.b())
}

struct AllDistances {
    haversine: f64,
    geodesic: f64,
    rhumb: f64,
}


fn calculate_all_distances(
    start: geo::Coord<f64>,
    end: geo::Coord<f64>,
    geodesy_ctx: &rgis_crs::GeodesyContext,
    target_crs: &rgis_crs::TargetCrs,
) -> Option<AllDistances> {
    let mut geodesy_ctx_inner = geodesy_ctx.write().ok()?;
    let target_epsg_code = 4326; // WGS 84

    let target_op_handle =
        rgis_crs::epsg_code_to_geodesy_op_handle(&mut *geodesy_ctx_inner, target_epsg_code).ok()?;

    let transformer = geo_geodesy::Transformer::from_geodesy(
        &*geodesy_ctx_inner,
        target_crs.0.op_handle,
        target_op_handle,
    )
    .ok()?;

    let mut start_lat_lon = geo::Geometry::Point(geo::Point::new(start.x, start.y));
    let mut end_lat_lon = geo::Geometry::Point(geo::Point::new(end.x, end.y));

    transformer.transform(&mut start_lat_lon).ok()?;
    transformer.transform(&mut end_lat_lon).ok()?;

    let (Some(geo::Geometry::Point(start_point)), Some(geo::Geometry::Point(end_point))) =
        (Some(start_lat_lon), Some(end_lat_lon))
    else {
        return None;
    };

    // geo_geodesy::Transformer::transform() already converts from radians to degrees
    Some(AllDistances {
        haversine: Haversine.distance(start_point, end_point),
        geodesic: Geodesic.distance(start_point, end_point),
        rhumb: Rhumb.distance(start_point, end_point),
    })
}
fn render_measure_tool(
    mut bevy_egui_ctx: EguiContexts,
    current_tool: Res<State<rgis_settings::Tool>>,
    measure_state: Res<rgis_mouse::MeasureState>,
    mouse_pos: Res<rgis_mouse::MousePos>,
    geodesy_ctx: Res<rgis_crs::GeodesyContext>,
    target_crs: Res<rgis_crs::TargetCrs>,
    camera_q: Query<&Transform, With<Camera>>,
    windows: Query<&bevy::window::Window, With<PrimaryWindow>>,
) -> Result {
    if *current_tool.get() != rgis_settings::Tool::Measure {
        return Ok(());
    }

    let Some(start) = measure_state.start else {
        return Ok(());
    };

    // Use locked end point if set, otherwise follow cursor
    let end = measure_state.end.unwrap_or(mouse_pos.0);
    let transform = camera_q.single()?;
    let window = windows.single()?;

    // Project points to screen for rendering
    let start_screen_pos =
        project_to_screen(geo::Coord { x: start.x.0, y: start.y.0 }, transform, window);
    let end_screen_pos =
        project_to_screen(geo::Coord { x: end.x.0, y: end.y.0 }, transform, window);

    let start_coord = geo::Coord {
        x: start.x.0,
        y: start.y.0,
    };
    let end_coord = geo::Coord {
        x: end.x.0,
        y: end.y.0,
    };

    let all_distances = calculate_all_distances(start_coord, end_coord, &geodesy_ctx, &target_crs);

    let bevy_egui_ctx_mut = bevy_egui_ctx.ctx_mut()?;
    let painter = bevy_egui_ctx_mut.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("measure_tool"),
    ));

    painter.line_segment(
        [start_screen_pos, end_screen_pos],
        egui::Stroke::new(2.0, egui::Color32::RED),
    );

    // Draw drag handles at endpoints: white fill with red border
    painter.circle(start_screen_pos, 8.0, egui::Color32::WHITE, egui::Stroke::new(2.0, egui::Color32::RED));
    painter.circle(end_screen_pos, 8.0, egui::Color32::WHITE, egui::Stroke::new(2.0, egui::Color32::RED));

    // Distance panel with live distances for all methods
    let entries: &[(&str, f64, &str)] = &[
        ("Haversine", all_distances.as_ref().map_or(0.0, |d| d.haversine), "Great-circle distance using the Haversine formula"),
        ("Geodesic", all_distances.as_ref().map_or(0.0, |d| d.geodesic), "Geodesic distance on the WGS84 ellipsoid (most accurate)"),
        ("Rhumb", all_distances.as_ref().map_or(0.0, |d| d.rhumb), "Distance along a rhumb line (constant bearing)"),
    ];
    egui::Window::new("Distances")
        .title_bar(false)
        .anchor(egui::Align2::RIGHT_BOTTOM, [-8.0, -8.0])
        .resizable(false)
        .auto_sized()
        .show(bevy_egui_ctx_mut, |ui| {
            for &(name, dist, description) in entries {
                let dist_str = if dist.is_finite() {
                    crate::widgets::scale_bar::distance_to_readable_string(dist as f32)
                } else {
                    "N/A".to_string()
                };
                let label = ui.label(format!("{name}: {dist_str}")).on_hover_text(description);
                crate::widget_registry::register(name, label.rect);
            }
        });

    Ok(())
}

fn project_to_screen(
    projected: geo::Coord<f64>,
    camera_transform: &Transform,
    window: &bevy::window::Window,
) -> egui::Pos2 {
    let sc = rgis_units::ScreenCoord::from_projected(projected, camera_transform, window);
    egui::Pos2::new(sc.x as f32, sc.y as f32)
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, SystemSet)]
enum RenderSystemSet {
    RenderingMessageWindow,
    RenderingTopBottom,
    Side,
    Windows,
}

pub fn configure(app: &mut App) {
    app.add_systems(
        PostStartup,
        (bevy_egui::setup_primary_egui_context_system, set_egui_theme).chain(),
    );

    app.configure_sets(
        EguiPrimaryContextPass,
        (
            RenderSystemSet::RenderingMessageWindow,
            RenderSystemSet::RenderingTopBottom,
            RenderSystemSet::Side,
            RenderSystemSet::Windows,
        )
            .chain(),
    );

    app.add_systems(
        EguiPrimaryContextPass,
        (
            crate::widgets::scale_bar::render_map_scale.in_set(RenderSystemSet::Side),
            crate::widgets::zoom_buttons::render_zoom_buttons.in_set(RenderSystemSet::Side),
            render_message_window.in_set(RenderSystemSet::RenderingMessageWindow),
            render_top.in_set(RenderSystemSet::RenderingTopBottom),
            render_bottom.in_set(RenderSystemSet::RenderingTopBottom),
            render_side.in_set(RenderSystemSet::Side),
            render_in_progress.in_set(RenderSystemSet::Side),
            render_manage_layer_window.in_set(RenderSystemSet::Windows),
            render_add_layer_window.in_set(RenderSystemSet::Windows),
            render_change_crs_window.in_set(RenderSystemSet::Windows),
            render_feature_properties_window.in_set(RenderSystemSet::Windows),
            render_attribute_table_window.in_set(RenderSystemSet::Windows),
            render_operation_window.in_set(RenderSystemSet::Windows),
            render_measure_tool.in_set(RenderSystemSet::RenderingTopBottom),
        ),
    );

    app.add_systems(
        Update,
        (
            handle_open_change_crs_window_event,
            handle_open_file_job,
            handle_save_file_job,
            handle_download_layer,
            perform_operation,
            handle_debug_window_close_request,
            handle_fill_color_requests,
        ),
    );

    crate::windows::debug::Debug::setup(app);
    crate::windows::logs::Logs::setup(app);
    crate::windows::welcome::Welcome::setup(app);
    app.add_systems(
        EguiPrimaryContextPass,
        bevy_egui_window::render_window_system::<crate::windows::debug::Debug>
            .run_if(bevy_egui_window::run_if_is_window_open::<crate::windows::debug::Debug>),
    );
    app.add_systems(
        EguiPrimaryContextPass,
        bevy_egui_window::render_window_system::<crate::windows::logs::Logs>
            .run_if(bevy_egui_window::run_if_is_window_open::<crate::windows::logs::Logs>),
    );
    app.add_systems(
        EguiPrimaryContextPass,
        crate::windows::welcome::render_welcome_window_system
            .run_if(bevy_egui_window::run_if_is_window_open::<crate::windows::welcome::Welcome>),
    );
}

fn handle_fill_color_requests(
    layer_order: Res<rgis_layers::LayerOrder>,
    layer_id_query: Query<&rgis_primitives::LayerId>,
    mut color_events: ResMut<Messages<rgis_ui_messages::UpdateLayerColorMessage>>,
) {
    for rgba in crate::widget_registry::take_fill_color_requests() {
        // Apply to the first layer
        if let Some(entity) = layer_order.iter_top_to_bottom().next() {
            if let Ok(layer_id) = layer_id_query.get(entity) {
                color_events.write(rgis_ui_messages::UpdateLayerColorMessage::Fill(
                    *layer_id,
                    Color::linear_rgba(rgba[0], rgba[1], rgba[2], rgba[3]),
                ));
            }
        }
    }
}

fn handle_debug_window_close_request(
    mut is_window_open: ResMut<
        bevy_egui_window::IsWindowOpen<crate::windows::debug::Debug<'static, 'static>>,
    >,
) {
    if crate::widget_registry::take_close_request("Debug") {
        is_window_open.0 = false;
    }
}

fn perform_operation(
    mut events: ResMut<Messages<rgis_ui_messages::PerformOperationMessage>>,
    id_map: Res<rgis_layers::LayerIdToEntity>,
    layer_query: Query<(&rgis_layers::LayerName, &rgis_layers::LayerData, &rgis_layers::LayerCrs)>,
    mut open_operation_window_event_writer: MessageWriter<rgis_ui_messages::OpenOperationWindowMessage>,
    mut create_layer_event_writer: MessageWriter<rgis_events::CreateLayerMessage>,
    mut render_message_event_writer: MessageWriter<rgis_ui_messages::RenderTextMessage>,
) {
    for event in events.drain() {
        let Some(entity) = id_map.get(event.layer_id) else {
            error!("Layer not found, cannot perform operation");
            continue;
        };
        let Ok((name, data, crs)) = layer_query.get(entity) else {
            error!("Layer not found, cannot perform operation");
            continue;
        };

        let Some(_) = data.unprojected_feature_collection() else {
            error!("Cannot perform operation on raster layer");
            continue;
        };

        let mut operation = event.operation;

        let Some(fc) = data.unprojected_feature_collection() else {
            error!("Layer has no unprojected feature collection, cannot perform operation");
            continue;
        };

        match operation.next_action() {
            rgis_geo_ops::Action::RenderUi => {
                open_operation_window_event_writer.write(
                    rgis_ui_messages::OpenOperationWindowMessage {
                        operation,
                        feature_collection: Arc::clone(fc),
                        layer_name: name.0.clone(),
                    },
                );
            }
            rgis_geo_ops::Action::Perform => {
                // TODO: perform in background job
                let op_name = operation.name().to_string();
                let outcome = operation.perform(fc);

                match outcome {
                    Ok(rgis_geo_ops::Outcome::FeatureCollection(feature_collection)) => {
                        create_layer_event_writer.write(rgis_events::CreateLayerMessage {
                            feature_collection: Arc::new(feature_collection),
                            name: format!("{} of {}", op_name, name.0),
                            source_crs: crs.0.clone(),
                        });
                    }
                    Ok(rgis_geo_ops::Outcome::Text(text)) => {
                        render_message_event_writer.write(rgis_ui_messages::RenderTextMessage(text));
                    }
                    Err(e) => {
                        error!("Encountered an error during the operation: {}", e);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_measure_tool() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(bevy::asset::AssetPlugin::default());
        app.add_plugins(bevy::window::WindowPlugin::default());
        app.add_plugins(bevy::input::InputPlugin);
        // Initialize Shader asset to satisfy bevy_egui requirement
        app.init_asset::<bevy::prelude::Shader>();
        app.init_asset::<bevy::prelude::Image>();

        app.add_plugins(bevy_egui::EguiPlugin::default());
        app.add_plugins(rgis_events::RgisEventsPlugin);
        app.add_plugins(rgis_crs::Plugin::default());
        app.add_plugins(bevy::state::app::StatesPlugin);

        app.insert_state(rgis_settings::Tool::Measure);
        app.insert_resource(rgis_settings::RgisSettings {
            show_scale: true,
            dark_mode: false,
        });

        app.insert_resource(rgis_mouse::MeasureState {
            start: Some(geo::Coord {
                x: 0.0.into(),
                y: 0.0.into(),
            }),
            end: None,
            dragging: None,
        });

        app.insert_resource(rgis_mouse::MousePos(geo::Coord {
            x: 10.0.into(),
            y: 10.0.into(),
        }));

        // Spawn an entity with Transform and Camera, which is what the system queries for.
        // We avoid using Camera2d bundle/component to avoid pulling in too many render dependencies.
        app.world_mut().spawn((
            Transform::default(),
            Camera::default(),
        ));

        app.update();

        app.add_systems(EguiPrimaryContextPass, render_measure_tool);
        app.update();
    }

    #[test]
    fn test_project_to_screen() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(bevy::window::WindowPlugin::default());

        let window_entity = app
            .world_mut()
            .spawn(bevy::window::Window {
                resolution: bevy::window::WindowResolution::new(800, 600),
                ..default()
            })
            .id();

        app.update();

        let window = app.world().get::<bevy::window::Window>(window_entity).unwrap();
        let camera_transform = Transform::from_translation(Vec3::new(0.0, 0.0, 0.0));
        let projected = geo::Coord { x: 0.0, y: 0.0 };

        let screen_pos = super::project_to_screen(projected, &camera_transform, window);

        assert_eq!(screen_pos.x, 400.0);
        assert_eq!(screen_pos.y, 300.0);
    }

    #[test]
    fn test_calculate_all_distances() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(rgis_events::RgisEventsPlugin);
        app.add_plugins(rgis_crs::Plugin::default());

        app.update();

        // Manually set TargetCrs to 4326 (WGS84) to simplify test and verify logic without projection issues
        let op_handle = {
            let geodesy_ctx = app.world().resource::<rgis_crs::GeodesyContext>();
            let mut geodesy_ctx_inner = geodesy_ctx.write().unwrap();
            rgis_crs::epsg_code_to_geodesy_op_handle(&mut *geodesy_ctx_inner, 4326).unwrap()
        };

        app.insert_resource(rgis_crs::TargetCrs(rgis_primitives::Crs {
            epsg_code: Some(4326),
            proj_string: None,
            op_handle,
        }));

        let geodesy_ctx = app.world().resource::<rgis_crs::GeodesyContext>();
        let target_crs = app.world().resource::<rgis_crs::TargetCrs>();

        // San Francisco (lon: -122.4194, lat: 37.7749)
        let start = geo::Coord {
            x: -122.4194,
            y: 37.7749,
        };
        // New York City (lon: -74.0060, lat: 40.7128)
        let end = geo::Coord {
            x: -74.0060,
            y: 40.7128,
        };

        let distances =
            super::calculate_all_distances(start, end, geodesy_ctx, target_crs).unwrap();

        // All distances must be finite (regression: a double .to_degrees() conversion
        // previously caused Geodesic to return NaN)
        assert!(distances.haversine.is_finite(), "Haversine was {}", distances.haversine);
        assert!(distances.geodesic.is_finite(), "Geodesic was {}", distances.geodesic);
        assert!(distances.rhumb.is_finite(), "Rhumb was {}", distances.rhumb);

        // Haversine distance is approx 4,129 km
        assert!(
            distances.haversine > 4_120_000.0 && distances.haversine < 4_140_000.0,
            "Haversine distance was {}",
            distances.haversine
        );

        // Geodesic distance is approx 4,139 km
        assert!(
            distances.geodesic > 4_130_000.0 && distances.geodesic < 4_150_000.0,
            "Geodesic distance was {}",
            distances.geodesic
        );

        // Rhumb line distance SF-NYC is longer than great-circle, approx 4,181 km
        assert!(
            distances.rhumb > 4_170_000.0 && distances.rhumb < 4_200_000.0,
            "Rhumb distance was {}",
            distances.rhumb
        );

    }

    /// Regression test: geo_geodesy::Transformer::transform() already converts
    /// output from radians to degrees. A previous bug called .to_degrees() again,
    /// turning valid coordinates like (-122, 37) into (-6692, 2282). This
    /// caused Geodesic to return NaN and Rhumb to return wildly wrong values.
    #[test]
    fn test_double_degrees_conversion_causes_geodesic_nan() {
        use geo::Distance;

        // These are the coordinates produced by the double .to_degrees() bug:
        // e.g. (-122.4194).to_degrees() = -6692.4
        let start = geo::Point::new(-6692.4, 2282.0);
        let end = geo::Point::new(-4395.1, 2552.0);

        // Geodesic returns NaN for these out-of-range coordinates
        assert!(
            Geodesic.distance(start, end).is_nan(),
            "Geodesic should return NaN for out-of-range coordinates"
        );

        // The correct coordinates (in degrees) should produce finite results
        let sf = geo::Point::new(-122.4194, 37.7749);
        let nyc = geo::Point::new(-74.0060, 40.7128);
        let geodesic_dist = Geodesic.distance(sf, nyc);
        assert!(
            geodesic_dist.is_finite() && geodesic_dist > 4_000_000.0,
            "Geodesic distance for valid coordinates was {}",
            geodesic_dist
        );
    }
}
