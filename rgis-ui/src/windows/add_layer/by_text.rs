use std::mem;

use bevy_egui::egui;
use geo_file_loader::FileFormat;

use super::{AddLayerOutput, State};

pub struct ByText<'a> {
    pub state: &'a mut State,
    pub geodesy_ctx: &'a rgis_crs::GeodesyContext,
}

impl<'a> ByText<'a> {
    pub fn show(self, ui: &mut egui::Ui) -> Option<AddLayerOutput> {
        let mut output = None;

        ui.label("Format:");

        let geojson_radio = ui.radio_value(
            &mut self.state.selected_format,
            Some(FileFormat::GeoJson),
            "GeoJSON",
        );
        crate::widget_registry::register("GeoJSON", geojson_radio.rect);

        let gpx_radio = ui.radio_value(
            &mut self.state.selected_format,
            Some(FileFormat::Gpx),
            "GPX",
        );
        crate::widget_registry::register("GPX", gpx_radio.rect);

        let wkt_radio = ui.radio_value(
            &mut self.state.selected_format,
            Some(FileFormat::Wkt),
            "WKT",
        );
        crate::widget_registry::register("WKT", wkt_radio.rect);

        let Some(selected_format) = self.state.selected_format else {
            return None;
        };

        ui.separator();

        ui.label("Input text:");

        egui::ScrollArea::vertical()
            .max_height(300.)
            .show(ui, |ui| {
                let text_edit =
                    egui::widgets::TextEdit::multiline(&mut self.state.text_edit_contents)
                        .code_editor()
                        .hint_text(hint_text(selected_format))
                        .show(ui);
                crate::widget_registry::register("Input text", text_edit.response.rect);
            });

        ui.separator();

        ui.label("Source CRS:");
        ui.add(crate::widgets::crs_input::CrsInput::new(
            self.geodesy_ctx,
            &mut self.state.crs_input_outcome,
            &mut self.state.crs_input,
            &mut self.state.crs_input_mode,
            false,
        ));

        let source_crs =
            crate::widgets::crs_input::outcome_crs(self.state.crs_input_outcome.as_ref());
        let submittable = !self.state.text_edit_contents.is_empty() && source_crs.is_some();

        ui.separator();

        let add_layer_button = ui.add_enabled(submittable, egui::Button::new("Add layer"));
        crate::widget_registry::register("Add layer", add_layer_button.rect);
        if add_layer_button.clicked() {
            if let Some(source_crs) = source_crs {
                let new = mem::take(&mut self.state.text_edit_contents);
                match selected_format {
                    FileFormat::Shapefile | FileFormat::GeoTiff => {
                        unreachable!()
                    }
                    file_format @ (FileFormat::Wkt | FileFormat::GeoJson | FileFormat::Gpx) => {
                        output = Some(AddLayerOutput::LoadFromText {
                            text: new,
                            file_format,
                            source_crs,
                        });
                    }
                }
            }
        }

        output
    }
}

const fn hint_text(format: FileFormat) -> &'static str {
    match format {
        FileFormat::GeoJson => "{\n  \"type\": \"FeatureCollection\",\n  \"features\": []\n}",
        FileFormat::Shapefile | FileFormat::GeoTiff => panic!("Binary formats are not textual"),
        FileFormat::Wkt => "LINESTRING (30 10, 10 30, 40 40)",
        FileFormat::Gpx => "", // TODO: add example GPX
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::crs_input::outcome_crs;

    fn show(state: &mut State, geodesy_ctx: &rgis_crs::GeodesyContext) {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ByText { state, geodesy_ctx }.show(ui);
            });
        });
    }

    fn geodesy_ctx() -> rgis_crs::GeodesyContext {
        let mut app = bevy::app::App::new();
        app.add_plugins(rgis_crs::Plugin::default());
        app.world()
            .resource::<rgis_crs::GeodesyContext>()
            .clone_for_async()
    }

    // Regression test for #281: submitting used to unwrap a CRS that only the
    // File tab ever set.
    #[test]
    fn fresh_form_defaults_to_wgs84() {
        let geodesy_ctx = geodesy_ctx();
        let mut state = State {
            selected_format: Some(FileFormat::Wkt),
            text_edit_contents: "POINT (1 2)".into(),
            ..Default::default()
        };

        show(&mut state, &geodesy_ctx);

        let crs = outcome_crs(state.crs_input_outcome.as_ref());
        assert_eq!(crs.map(|crs| crs.epsg_code), Some(Some(4326)));
    }

    #[test]
    fn invalid_crs_disables_submission() {
        let geodesy_ctx = geodesy_ctx();
        let mut state = State {
            selected_format: Some(FileFormat::Wkt),
            text_edit_contents: "POINT (1 2)".into(),
            crs_input: "not a code".into(),
            ..Default::default()
        };

        show(&mut state, &geodesy_ctx);

        assert!(matches!(state.crs_input_outcome, Some(Err(_))));
        assert!(outcome_crs(state.crs_input_outcome.as_ref()).is_none());
    }
}
