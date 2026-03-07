use bevy::prelude::*;
use bevy_egui::egui;
use rgis_ui_messages::UpdateLayerStrokeWidthMessage;

pub struct StrokeWidth<'a> {
    pub layer_id: rgis_primitives::LayerId,
    pub width: f32,
    pub width_events: &'a mut Messages<UpdateLayerStrokeWidthMessage>,
}

impl egui::Widget for StrokeWidth<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let mut new_width = self.width;
        let response = ui.add(egui::Slider::new(&mut new_width, 0.1..=50.0));
        if response.changed() {
            self.width_events
                .write(UpdateLayerStrokeWidthMessage(self.layer_id, new_width));
        }
        response
    }
}
