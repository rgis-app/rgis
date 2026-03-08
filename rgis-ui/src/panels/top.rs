use bevy::prelude::*;
use bevy_egui::egui;
use bevy_egui_window as window;

pub struct Top<'a, 'w> {
    pub app_exit_events: &'a mut Messages<AppExit>,
    pub egui_ctx: &'a mut bevy_egui::egui::Context,
    pub window: &'a mut Window,
    pub app_settings: &'a mut rgis_settings::RgisSettings,
    pub current_tool: &'a rgis_settings::Tool,
    pub next_tool: &'a mut NextState<rgis_settings::Tool>,
    pub top_panel_height: &'a mut rgis_units::TopPanelHeight,
    pub is_debug_window_open:
        &'a mut window::IsWindowOpen<crate::windows::debug::Debug<'static, 'static>>,
    pub is_logs_window_open:
        &'a mut window::IsWindowOpen<crate::windows::logs::Logs<'static>>,
    pub show_add_layer_window_event_writer:
        &'a mut MessageWriter<'w, rgis_ui_messages::ShowAddLayerWindowMessage>,
    pub clear_color: &'a mut ClearColor,
    pub logo_texture_id: Option<egui::TextureId>,
}

impl Top<'_, '_> {
    pub fn render(&mut self) {
        let current_tool = *self.current_tool;
        let inner_response = egui::TopBottomPanel::top("top_panel").show(self.egui_ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                let mut new_tool = current_tool;

                if let Some(texture_id) = self.logo_texture_id {
                    let logo_size = egui::vec2(20.0, 20.0);
                    let logo_response = ui.image(egui::load::SizedTexture::new(texture_id, logo_size));
                    crate::widget_registry::register("Logo", logo_response.rect);
                } else {
                    ui.label("rgis");
                }
                let file_response = ui.menu_button("File", |ui| {
                    if ui.button("Add Layer...").clicked() {
                        self.show_add_layer_window_event_writer.write_default();
                        ui.close();
                    }
                    ui.separator();
                    ui.add(crate::widgets::exit::Exit {
                        app_exit_events: self.app_exit_events,
                    });
                });
                crate::widget_registry::register("File", file_response.response.rect);
                let view_response = ui.menu_button("View", |ui| {
                    ui.add(crate::widgets::full_screen::FullScreen {
                        window: self.window,
                    });
                    if ui
                        .button(format!(
                            "{} scale",
                            if self.app_settings.show_scale {
                                "Hide"
                            } else {
                                "Show"
                            }
                        ))
                        .clicked()
                    {
                        self.app_settings.show_scale = !self.app_settings.show_scale;
                    }
                    if ui.button("Toggle dark mode").clicked() {
                        self.app_settings.dark_mode = !self.app_settings.dark_mode;
                        let visuals = if self.app_settings.dark_mode {
                            egui::Visuals::dark()
                        } else {
                            egui::Visuals::light()
                        };
                        self.clear_color.0 = crate::systems::egui_color_to_bevy_color(visuals.extreme_bg_color);
                        self.egui_ctx.set_visuals(visuals);
                        ui.close();
                    }
                });
                crate::widget_registry::register("View", view_response.response.rect);
                let help_response = ui.menu_button("Help", |ui| {
                    let debug_btn = ui.button("Debug stats");
                    crate::widget_registry::register("Debug stats", debug_btn.rect);
                    if debug_btn.clicked() {
                        self.is_debug_window_open.0 = true;
                    }
                    let logs_btn = ui.button("Logs");
                    crate::widget_registry::register("Logs", logs_btn.rect);
                    if logs_btn.clicked() {
                        self.is_logs_window_open.0 = true;
                    }
                    let source_btn = ui.button("Source code");
                    crate::widget_registry::register("Source code", source_btn.rect);
                    if source_btn.clicked() {
                        ui.ctx().open_url(egui::OpenUrl {
                            url: String::from("https://github.com/frewsxcv/rgis"),
                            new_tab: true,
                        });
                    }
                });
                crate::widget_registry::register("Help", help_response.response.rect);

                ui.separator();

                let pan_btn = ui
                    .add_enabled(
                        current_tool != rgis_settings::Tool::Pan,
                        egui::Button::new("Pan")
                            .selected(current_tool == rgis_settings::Tool::Pan),
                    );
                crate::widget_registry::register("Pan Tool", pan_btn.rect);
                if pan_btn.clicked()
                {
                    new_tool = rgis_settings::Tool::Pan;
                }

                let query_btn = ui
                    .add_enabled(
                        current_tool != rgis_settings::Tool::Query,
                        egui::Button::new("Query")
                            .selected(current_tool == rgis_settings::Tool::Query),
                    );
                crate::widget_registry::register("Query Tool", query_btn.rect);
                if query_btn.clicked()
                {
                    new_tool = rgis_settings::Tool::Query;
                }

                let measure_btn = ui
                    .add_enabled(
                        current_tool != rgis_settings::Tool::Measure,
                        egui::Button::new("Measure")
                            .selected(current_tool == rgis_settings::Tool::Measure),
                    );
                crate::widget_registry::register("Measure Tool", measure_btn.rect);
                if measure_btn.clicked()
                {
                    new_tool = rgis_settings::Tool::Measure;
                }

                if current_tool == rgis_settings::Tool::Query
                    && new_tool != rgis_settings::Tool::Query
                {
                    // send DeselectAllFeatures event
                }

                if new_tool != current_tool {
                    self.next_tool.set(new_tool);
                }
            });
        });

        self.top_panel_height.0 = inner_response.response.rect.height();
    }
}
