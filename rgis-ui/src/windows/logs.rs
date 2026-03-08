use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::egui;

#[derive(SystemParam)]
pub struct Logs<'w> {
    buffer: Res<'w, crate::log_buffer::LogBuffer>,
}

fn level_color(level: &tracing::Level) -> egui::Color32 {
    match *level {
        tracing::Level::ERROR => egui::Color32::RED,
        tracing::Level::WARN => egui::Color32::YELLOW,
        tracing::Level::INFO => egui::Color32::GREEN,
        tracing::Level::DEBUG => egui::Color32::LIGHT_BLUE,
        tracing::Level::TRACE => egui::Color32::GRAY,
    }
}

impl egui::Widget for Logs<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let entries: Vec<_> = if let Ok(buf) = self.buffer.0.lock() {
            buf.iter().rev().cloned().collect()
        } else {
            Vec::new()
        };

        let response = egui::ScrollArea::vertical()
            .max_height(400.0)
            .show(ui, |ui| {
                if entries.is_empty() {
                    ui.label("No log entries yet.");
                } else {
                    for entry in &entries {
                        ui.horizontal(|ui| {
                            let level_str = match entry.level {
                                tracing::Level::ERROR => "ERROR",
                                tracing::Level::WARN => " WARN",
                                tracing::Level::INFO => " INFO",
                                tracing::Level::DEBUG => "DEBUG",
                                tracing::Level::TRACE => "TRACE",
                            };
                            ui.colored_label(level_color(&entry.level), level_str);
                            ui.colored_label(egui::Color32::DARK_GRAY, &entry.target);
                            ui.label(&entry.message);
                        });
                    }
                }
            });

        crate::widget_registry::register("Logs", response.inner_rect);
        ui.allocate_rect(response.inner_rect, egui::Sense::hover())
    }
}

impl bevy_egui_window::Window for Logs<'_> {
    type Item<'w, 's> = Logs<'w>;

    fn title(&self) -> &str {
        "Logs"
    }

    fn default_width(&self) -> f32 {
        600.
    }
}
