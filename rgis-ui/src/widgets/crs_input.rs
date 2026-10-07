use bevy_egui::egui;
use geodesy::ctx::Context;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrsInputMode {
    Epsg,
    Proj,
}

impl Default for CrsInputMode {
    fn default() -> Self {
        CrsInputMode::Epsg
    }
}

pub struct CrsInput<'a> {
    pub geodesy_ctx: &'a rgis_crs::GeodesyContext,
    pub text_field_value: &'a mut String,
    pub input_mode: &'a mut CrsInputMode,
    outcome: &'a mut Option<Outcome>,
    request_focus: bool,
}

impl<'a> CrsInput<'a> {
    pub fn new(
        geodesy_ctx: &'a rgis_crs::GeodesyContext,
        outcome: &'a mut Option<Outcome>,
        text_field_value: &'a mut String,
        input_mode: &'a mut CrsInputMode,
        request_focus: bool,
    ) -> Self {
        Self {
            geodesy_ctx,
            outcome,
            text_field_value,
            input_mode,
            request_focus,
        }
    }
}

pub type Outcome = Result<(geodesy::ctx::OpHandle, Option<u16>, Option<String>), Error>;

/// The CRS the user entered, or `None` if nothing has parsed yet.
pub fn outcome_crs(outcome: Option<&Outcome>) -> Option<rgis_primitives::Crs> {
    let Some(Ok((op_handle, epsg_code, proj_string))) = outcome else {
        return None;
    };
    Some(rgis_primitives::Crs {
        epsg_code: *epsg_code,
        proj_string: proj_string.clone(),
        op_handle: *op_handle,
    })
}

impl egui::Widget for CrsInput<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.radio_value(self.input_mode, CrsInputMode::Epsg, "EPSG Code");
                ui.radio_value(self.input_mode, CrsInputMode::Proj, "PROJ String");
            });

            match self.input_mode {
                CrsInputMode::Epsg => {
                    ui.add(EpsgCodeInputFieldWidget::new(
                        self.geodesy_ctx,
                        self.text_field_value,
                        self.outcome,
                        self.request_focus,
                    ));
                }
                CrsInputMode::Proj => {
                    ui.add(ProjStringInputFieldWidget::new(
                        self.geodesy_ctx,
                        self.text_field_value,
                        self.outcome,
                        self.request_focus,
                    ));
                }
            }

            let Some(outcome) = self.outcome else { return };

            match outcome {
                Ok((op_handle, _, _)) => {
                    ui.vertical(|ui| {
                        let geodesy_ctx = self.geodesy_ctx.read().unwrap();
                        let Ok(steps) = geodesy_ctx.steps(*op_handle) else {
                            return;
                        };
                        for step in steps {
                            ui.label(egui::RichText::new(step).code());
                        }
                    });
                }
                Err(e) => {
                    ui.label(format!("{e}"));
                }
            }
        })
        .response
    }
}

struct EpsgCodeInputFieldWidget<'a> {
    geodesy_ctx: &'a rgis_crs::GeodesyContext,
    text_field_value: &'a mut String,
    outcome: &'a mut Option<Outcome>,
    parsed_text_field_value: Option<u16>,
    request_focus: bool,
}

impl<'a> EpsgCodeInputFieldWidget<'a> {
    pub fn new(
        geodesy_ctx: &'a rgis_crs::GeodesyContext,
        text_field_value: &'a mut String,
        outcome: &'a mut Option<Outcome>,
        request_focus: bool,
    ) -> Self {
        Self {
            geodesy_ctx,
            text_field_value,
            outcome,
            parsed_text_field_value: None,
            request_focus,
        }
    }
}

impl egui::Widget for EpsgCodeInputFieldWidget<'_> {
    fn ui(mut self, ui: &mut egui::Ui) -> egui::Response {
        ui.horizontal(|ui| {
            ui.label("EPSG:");
            let edit_field = ui.text_edit_singleline(self.text_field_value);
            if self.request_focus {
                edit_field.request_focus();
            }

            *self.outcome = if edit_field.changed()
                || (!self.text_field_value.is_empty() && self.outcome.is_none())
            {
                ui.add(ValidIconWidget);
                Some(parse_epsg_input_value(
                    self.geodesy_ctx,
                    self.text_field_value,
                    &mut self.parsed_text_field_value,
                ))
            } else if let Some(n) = self.outcome.take() {
                if n.is_ok() {
                    ui.add(ValidIconWidget);
                } else {
                    ui.add(InvalidIconWidget);
                }
                Some(n)
            } else {
                ui.add(InvalidIconWidget);
                None
            };
        })
        .response
    }
}

struct ProjStringInputFieldWidget<'a> {
    geodesy_ctx: &'a rgis_crs::GeodesyContext,
    text_field_value: &'a mut String,
    outcome: &'a mut Option<Outcome>,
    request_focus: bool,
}

impl<'a> ProjStringInputFieldWidget<'a> {
    pub fn new(
        geodesy_ctx: &'a rgis_crs::GeodesyContext,
        text_field_value: &'a mut String,
        outcome: &'a mut Option<Outcome>,
        request_focus: bool,
    ) -> Self {
        Self {
            geodesy_ctx,
            text_field_value,
            outcome,
            request_focus,
        }
    }
}

impl egui::Widget for ProjStringInputFieldWidget<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        ui.horizontal(|ui| {
            ui.label("PROJ:");
            let edit_field = ui.text_edit_singleline(self.text_field_value);
            if self.request_focus {
                edit_field.request_focus();
            }

            *self.outcome = if edit_field.changed()
                || (!self.text_field_value.is_empty() && self.outcome.is_none())
            {
                ui.add(ValidIconWidget);
                Some(parse_proj_input_value(
                    self.geodesy_ctx,
                    self.text_field_value,
                ))
            } else if let Some(n) = self.outcome.take() {
                if n.is_ok() {
                    ui.add(ValidIconWidget);
                } else {
                    ui.add(InvalidIconWidget);
                }
                Some(n)
            } else {
                ui.add(InvalidIconWidget);
                None
            };
        })
        .response
    }
}

#[derive(Debug)]
pub enum Error {
    ParseIntError(std::num::ParseIntError),
    Geodesy(rgis_crs::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::ParseIntError(e) => write!(f, "ParseIntError: {}", e),
            Error::Geodesy(e) => write!(f, "Geodesy error: {}", e),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::num::ParseIntError> for Error {
    fn from(err: std::num::ParseIntError) -> Self {
        Error::ParseIntError(err)
    }
}

impl From<rgis_crs::Error> for Error {
    fn from(err: rgis_crs::Error) -> Self {
        Error::Geodesy(err)
    }
}

fn parse_epsg_input_value(
    geodesy_ctx: &rgis_crs::GeodesyContext,
    input: &str,
    parsed_text_field_value: &mut Option<u16>,
) -> Outcome {
    let mut geodesy_ctx = geodesy_ctx.write().unwrap();
    let parsed = u16::from_str(input);
    *parsed_text_field_value = parsed.as_ref().ok().copied();
    let parsed = parsed?;
    let outcome = rgis_crs::epsg_code_to_geodesy_op_handle(&mut *geodesy_ctx, parsed)
        .map_err(Error::Geodesy)?;
    Ok((outcome, Some(parsed), None))
}

fn parse_proj_input_value(
    geodesy_ctx: &rgis_crs::GeodesyContext,
    input: &str,
) -> Outcome {
    let mut geodesy_ctx = geodesy_ctx.write().unwrap();
    let op_handle = rgis_crs::proj_string_to_geodesy_op_handle(&mut *geodesy_ctx, input)
        .map_err(Error::Geodesy)?;
    Ok((op_handle, None, Some(input.to_string())))
}

struct ValidIconWidget;

impl egui::Widget for ValidIconWidget {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        ui.label(egui::RichText::new("✅"))
    }
}

struct InvalidIconWidget;

impl egui::Widget for InvalidIconWidget {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        ui.label(egui::RichText::new("❌").color(ui.visuals().error_fg_color))
    }
}
