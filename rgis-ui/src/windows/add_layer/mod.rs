use bevy::{ecs::system::SystemParam, prelude::*};
use bevy_egui::egui;
use geo_file_loader::FileFormat;
use rgis_events::LoadFileMessage;
use rgis_ui_messages::{HideAddLayerWindowMessage, ShowAddLayerWindowMessage};

pub mod by_file;
pub mod by_text;
pub mod file;
pub mod library;

#[derive(SystemParam)]
pub struct Events<'w, 's> {
    pub load_file_event_writer: MessageWriter<'w, LoadFileMessage>,
    pub show_add_layer_window_event_reader: MessageReader<'w, 's, ShowAddLayerWindowMessage>,
    pub hide_add_layer_window_events: ResMut<'w, bevy::prelude::Messages<HideAddLayerWindowMessage>>,
}

pub struct AddLayer<'a> {
    pub state: &'a mut State,
    pub selected_file: &'a mut file::SelectedFile,
    pub geodesy_ctx: &'a rgis_crs::GeodesyContext,
}

#[derive(PartialEq, Eq)]
enum Source {
    Unselected,
    Library,
    File,
    Text,
}

pub struct State {
    pub text_edit_contents: String,
    pub crs_input: String,
    pub crs_input_mode: crate::widgets::crs_input::CrsInputMode,
    selected_source: Source,
    pub selected_format: Option<FileFormat>,
    pub crs_input_outcome: Option<crate::widgets::crs_input::Outcome>,
}

const DEFAULT_CRS_INPUT: &str = "4326";

impl Default for State {
    fn default() -> Self {
        State {
            text_edit_contents: "".into(),
            crs_input: DEFAULT_CRS_INPUT.into(),
            crs_input_mode: Default::default(),
            crs_input_outcome: None,
            selected_format: None,
            selected_source: Source::Unselected,
        }
    }
}

impl State {
    pub fn reset(&mut self) {
        self.text_edit_contents = String::new();
        self.crs_input = DEFAULT_CRS_INPUT.into();
        self.crs_input_mode = Default::default();
        self.crs_input_outcome = None;
        self.selected_source = Source::Unselected;
        self.selected_format = None;
    }
}

pub enum AddLayerOutput {
    LoadFromText {
        text: String,
        file_format: FileFormat,
        source_crs: rgis_primitives::Crs,
    },
    LoadFromFile {
        file_name: String,
        file_format: FileFormat,
        bytes: Vec<u8>,
        source_crs: rgis_primitives::Crs,
    },
    LoadFromLibrary {
        name: String,
        url: String,
        source_crs: rgis_primitives::Crs,
    },
    OpenFile,
}

impl AddLayer<'_> {
    pub fn render(&mut self, ui: &mut egui::Ui) -> Option<AddLayerOutput> {
        let mut output = None;

        ui.label("Layer source:");

        let library_radio = ui.radio_value(&mut self.state.selected_source, Source::Library, "Library");
        crate::widget_registry::register("Library", library_radio.rect);
        let file_radio = ui.radio_value(&mut self.state.selected_source, Source::File, "File");
        crate::widget_registry::register("File", file_radio.rect);
        let text_radio = ui.radio_value(&mut self.state.selected_source, Source::Text, "Text");
        crate::widget_registry::register("Text", text_radio.rect);

        if self.state.selected_source == Source::Unselected {
            return None;
        }

        // If the user switched to "Text" and and they don't have a plaintext format selected, unselect their selection
        if self.state.selected_source == Source::Text
            && self
                .state
                .selected_format
                .map(|f| !f.is_plaintext())
                .unwrap_or(false)
        {
            self.state.selected_format = None;
        }

        ui.separator();

        match self.state.selected_source {
            Source::Unselected => {}
            Source::Library => {
                if let Some(new_output) = (library::LibraryWidget {
                    geodesy_ctx: self.geodesy_ctx,
                })
                .show(ui)
                {
                    output = Some(new_output);
                }
            }
            Source::File => {
                if let Some(new_output) = (by_file::ByFile {
                    selected_file: self.selected_file,
                    state: self.state,
                    geodesy_ctx: self.geodesy_ctx,
                })
                .show(ui)
                {
                    output = Some(new_output);
                }
            }
            Source::Text => {
                if let Some(new_output) = (by_text::ByText {
                    state: self.state,
                    geodesy_ctx: self.geodesy_ctx,
                })
                .show(ui)
                {
                    output = Some(new_output);
                }
            }
        }

        output
    }
}

