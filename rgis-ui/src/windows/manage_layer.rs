use bevy::prelude::*;
use bevy_egui::egui;
use rgis_layer_messages::DuplicateLayerMessage;
use rgis_ui_messages::{UpdateLayerColorMessage, UpdateLayerPointSizeMessage, UpdateLayerStrokeWidthMessage};

pub struct ManageLayer<'a> {
    pub state: &'a mut crate::ManageLayerWindowState,
    pub layers: &'a rgis_layers::Layers,
    pub egui_ctx: &'a mut bevy_egui::egui::Context,
    pub color_events: &'a mut Messages<UpdateLayerColorMessage>,
    pub point_size_events: &'a mut Messages<UpdateLayerPointSizeMessage>,
    pub stroke_width_events: &'a mut Messages<UpdateLayerStrokeWidthMessage>,
    pub duplicate_layer_events: &'a mut Messages<DuplicateLayerMessage>,
}

impl ManageLayer<'_> {
    pub fn render(&mut self) {
        let Some(layer_id) = *self.state else {
            return;
        };
        let Some(layer) = self.layers.get(layer_id) else {
            warn!(
                "Could not find layer with ID {:?}, closing manage layer window",
                layer_id
            );
            *self.state = None;
            return;
        };
        let mut is_open = true;
        egui::Window::new("Manage Layer")
            .open(&mut is_open)
            .show(self.egui_ctx, |ui| {
                egui::Grid::new("manage_layer_window_grid")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("Name");
                        ui.label(&layer.name);
                        ui.end_row();
                        ui.label("CRS");
                        match layer.crs.epsg_code {
                            Some(code) => ui.label(format!("EPSG {code}")),
                            None => ui.label("Custom PROJ"),
                        };
                        ui.end_row();
                        if let Some(geom_type) = layer.geom_type() {
                            if geom_type.has_fill() {
                                if let Some(fill) = layer.color.fill {
                                    ui.label("Fill color");
                                    ui.add(crate::widgets::fill_color::FillColor {
                                        layer_id,
                                        color: fill,
                                        color_events: self.color_events,
                                    });
                                    ui.end_row();
                                } else {
                                    error!("Expected layer to have a fill color");
                                }
                            }
                            ui.label("Stroke color");
                            ui.add(crate::widgets::stroke_color::StrokeColor {
                                layer_id,
                                color: layer.color.stroke,
                                color_events: self.color_events,
                            });
                            ui.end_row();
                            if geom_type.contains(geo_geom_type::GeomType::POINT)
                                || geom_type
                                    .contains(geo_geom_type::GeomType::MULTI_POINT)
                            {
                                ui.label("Point size");
                                ui.add(crate::widgets::point_size::PointSize {
                                    layer_id,
                                    size: layer.point_size,
                                    size_events: self.point_size_events,
                                });
                                ui.end_row();
                            }
                            let response = ui.label("Stroke width");
                            crate::widget_registry::register("Stroke width", response.rect);
                            ui.add(crate::widgets::stroke_width::StrokeWidth {
                                layer_id,
                                width: layer.stroke_width,
                                width_events: self.stroke_width_events,
                            });
                            ui.end_row();
                        }
                    });
                ui.separator();
                if ui.button("Duplicate").clicked() {
                    self.duplicate_layer_events
                        .write(DuplicateLayerMessage(layer_id));
                }
            });

        if !is_open {
            *self.state = None;
        }
    }
}
