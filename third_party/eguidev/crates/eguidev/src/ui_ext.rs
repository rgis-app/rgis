//! Helper extensions for recording common widgets with explicit ids.

use std::ops::RangeInclusive;

use egui::{collapsing_header::CollapsingResponse, scroll_area::ScrollAreaOutput};

use crate::{
    instrument::{active_inner, capture_layout, container, swallow_panic},
    types::{RoleState, WidgetRange, WidgetRole, WidgetValue},
    widget_registry::{WidgetMeta, record_widget},
};

/// Options for selected-aware buttons.
#[derive(Debug, Clone, Copy, Default)]
pub struct ButtonOptions {
    /// Whether the button is in a selected/toggled state.
    pub selected: bool,
}

/// Options for third-state checkboxes.
#[derive(Debug, Clone, Copy, Default)]
pub struct CheckboxOptions {
    /// Whether the checkbox should render as indeterminate.
    pub indeterminate: bool,
}

/// Options for text-edit widgets.
#[derive(Debug, Clone, Copy, Default)]
pub struct TextEditOptions {
    /// Whether the edit is multiline.
    pub multiline: bool,
    /// Whether the edit masks its contents.
    pub password: bool,
}

/// Options for progress bars.
#[derive(Debug, Clone, Default)]
pub struct ProgressBarOptions {
    /// Optional overlay text rendered inside the bar.
    pub text: Option<String>,
    /// Whether to show a percentage when `text` is absent.
    pub show_percentage: bool,
}

/// Helper extensions for recording common widgets with explicit ids.
///
/// Prefer these for standard widgets; they auto-populate role/type/value/label metadata.
pub trait DevUiExt {
    /// Add a button with an explicit id.
    fn dev_button(
        &mut self,
        id: impl Into<String>,
        text: impl Into<egui::WidgetText>,
    ) -> egui::Response;

    /// Add a selected-aware button with explicit metadata.
    fn dev_button_with(
        &mut self,
        id: impl Into<String>,
        text: impl Into<egui::WidgetText>,
        options: ButtonOptions,
    ) -> egui::Response;

    /// Add a link with an explicit id.
    fn dev_link(
        &mut self,
        id: impl Into<String>,
        text: impl Into<egui::WidgetText>,
    ) -> egui::Response;

    /// Add a hyperlink with an explicit id, using its URL as label and value metadata.
    fn dev_hyperlink(&mut self, id: impl Into<String>, url: impl ToString) -> egui::Response;

    /// Add a hyperlink with an explicit id, label, and URL metadata.
    fn dev_hyperlink_to(
        &mut self,
        id: impl Into<String>,
        label: impl Into<egui::WidgetText>,
        url: impl ToString,
    ) -> egui::Response;

    /// Add an image with an explicit id and developer-provided description.
    fn dev_image<'a>(
        &mut self,
        id: impl Into<String>,
        description: impl Into<String>,
        source: impl Into<egui::ImageSource<'a>>,
    ) -> egui::Response;

    /// Add a label with an explicit id.
    fn dev_label(
        &mut self,
        id: impl Into<String>,
        text: impl Into<egui::WidgetText>,
    ) -> egui::Response;

    /// Add a checkbox with an explicit id.
    fn dev_checkbox(
        &mut self,
        id: impl Into<String>,
        value: &mut bool,
        text: impl Into<egui::WidgetText>,
    ) -> egui::Response;

    /// Add an indeterminate-aware checkbox with explicit metadata.
    fn dev_checkbox_with(
        &mut self,
        id: impl Into<String>,
        value: &mut bool,
        text: impl Into<egui::WidgetText>,
        options: CheckboxOptions,
    ) -> egui::Response;

    /// Add a text edit with an explicit id.
    fn dev_text_edit(&mut self, id: impl Into<String>, text: &mut String) -> egui::Response;

    /// Add a text edit with explicit mode metadata.
    fn dev_text_edit_with(
        &mut self,
        id: impl Into<String>,
        text: &mut String,
        options: TextEditOptions,
    ) -> egui::Response;

    /// Add a slider with an explicit id.
    fn dev_slider(
        &mut self,
        id: impl Into<String>,
        value: &mut f32,
        range: RangeInclusive<f32>,
    ) -> egui::Response;

    /// Add a combo box with an explicit id.
    fn dev_combo_box<T: ToString>(
        &mut self,
        id: impl Into<String>,
        label: impl Into<String>,
        selected: &mut usize,
        options: &[T],
    ) -> egui::Response;

    /// Add a float drag value with an explicit id.
    fn dev_drag_value(&mut self, id: impl Into<String>, value: &mut f32) -> egui::Response;

    /// Add a float drag value with an explicit constrained range.
    fn dev_drag_value_range(
        &mut self,
        id: impl Into<String>,
        value: &mut f32,
        range: RangeInclusive<f32>,
    ) -> egui::Response;

    /// Add an integer drag value with an explicit id.
    fn dev_drag_value_i32(&mut self, id: impl Into<String>, value: &mut i32) -> egui::Response;

    /// Add an integer drag value with an explicit constrained range.
    fn dev_drag_value_i32_range(
        &mut self,
        id: impl Into<String>,
        value: &mut i32,
        range: RangeInclusive<i32>,
    ) -> egui::Response;

    /// Add a multiline text edit with an explicit id.
    fn dev_text_edit_multiline(
        &mut self,
        id: impl Into<String>,
        text: &mut String,
    ) -> egui::Response;

    /// Add a toggle value with an explicit id.
    fn dev_toggle_value(
        &mut self,
        id: impl Into<String>,
        selected: &mut bool,
        text: impl Into<egui::WidgetText>,
    ) -> egui::Response;

    /// Add a radio value with an explicit id.
    fn dev_radio_value<V: PartialEq + Clone>(
        &mut self,
        id: impl Into<String>,
        current: &mut V,
        alternative: V,
        text: impl Into<egui::WidgetText>,
    ) -> egui::Response;

    /// Add a selectable value with an explicit id.
    fn dev_selectable_value<V: PartialEq + Clone>(
        &mut self,
        id: impl Into<String>,
        current: &mut V,
        alternative: V,
        text: impl Into<egui::WidgetText>,
    ) -> egui::Response;

    /// Add a separator with an explicit id.
    fn dev_separator(&mut self, id: impl Into<String>) -> egui::Response;

    /// Add a spinner with an explicit id.
    fn dev_spinner(&mut self, id: impl Into<String>) -> egui::Response;

    /// Add a progress bar with an explicit id.
    fn dev_progress_bar(&mut self, id: impl Into<String>, progress: f32) -> egui::Response;

    /// Add a progress bar with explicit overlay text options.
    fn dev_progress_bar_with(
        &mut self,
        id: impl Into<String>,
        progress: f32,
        options: ProgressBarOptions,
    ) -> egui::Response;

    /// Add a color-edit button with an explicit id.
    fn dev_color_edit(
        &mut self,
        id: impl Into<String>,
        color: &mut egui::Color32,
    ) -> egui::Response;

    /// Add a menu button with an explicit id.
    fn dev_menu_button<R>(
        &mut self,
        id: impl Into<String>,
        text: impl Into<egui::WidgetText>,
        add_contents: impl FnOnce(&mut egui::Ui) -> R,
    ) -> egui::InnerResponse<Option<R>>;

    /// Add a collapsing header with an explicit id, bound to app state.
    fn dev_collapsing<R>(
        &mut self,
        id: impl Into<String>,
        open: &mut bool,
        heading: impl Into<egui::WidgetText>,
        add_contents: impl FnOnce(&mut egui::Ui) -> R,
    ) -> CollapsingResponse<R>;
}

impl DevUiExt for egui::Ui {
    fn dev_button(
        &mut self,
        id: impl Into<String>,
        text: impl Into<egui::WidgetText>,
    ) -> egui::Response {
        let id = id.into();
        let (text, label) = widget_text_parts(text);
        let response = self.button(text);
        record_widget_with_layout(
            self,
            id,
            &response,
            WidgetRole::Button,
            Some(label),
            None,
            None,
        );
        response
    }

    fn dev_button_with(
        &mut self,
        id: impl Into<String>,
        text: impl Into<egui::WidgetText>,
        options: ButtonOptions,
    ) -> egui::Response {
        let id = id.into();
        let (text, label) = widget_text_parts(text);
        let response = self.add(egui::Button::new(text).selected(options.selected));
        record_widget_with_layout(
            self,
            id,
            &response,
            WidgetRole::Button,
            Some(label),
            None,
            Some(RoleState::Button {
                selected: options.selected,
            }),
        );
        response
    }

    fn dev_link(
        &mut self,
        id: impl Into<String>,
        text: impl Into<egui::WidgetText>,
    ) -> egui::Response {
        let id = id.into();
        let (text, label) = widget_text_parts(text);
        let response = self.link(text);
        record_widget_with_layout(
            self,
            id,
            &response,
            WidgetRole::Link,
            Some(label),
            None,
            None,
        );
        response
    }

    fn dev_hyperlink(&mut self, id: impl Into<String>, url: impl ToString) -> egui::Response {
        let id = id.into();
        let url = url.to_string();
        let response = self.hyperlink(url.clone());
        record_widget_with_layout(
            self,
            id,
            &response,
            WidgetRole::Link,
            Some(url.clone()),
            Some(WidgetValue::Text(url)),
            None,
        );
        response
    }

    fn dev_hyperlink_to(
        &mut self,
        id: impl Into<String>,
        label: impl Into<egui::WidgetText>,
        url: impl ToString,
    ) -> egui::Response {
        let id = id.into();
        let (label, text) = widget_text_parts(label);
        let url = url.to_string();
        let response = self.hyperlink_to(label, url.clone());
        record_widget_with_layout(
            self,
            id,
            &response,
            WidgetRole::Link,
            Some(text),
            Some(WidgetValue::Text(url)),
            None,
        );
        response
    }

    fn dev_image<'a>(
        &mut self,
        id: impl Into<String>,
        description: impl Into<String>,
        source: impl Into<egui::ImageSource<'a>>,
    ) -> egui::Response {
        let id = id.into();
        let description = description.into();
        let response = self.image(source);
        record_widget_with_layout(
            self,
            id,
            &response,
            WidgetRole::Image,
            Some(description.clone()),
            Some(WidgetValue::Text(description)),
            None,
        );
        response
    }

    fn dev_label(
        &mut self,
        id: impl Into<String>,
        text: impl Into<egui::WidgetText>,
    ) -> egui::Response {
        let id = id.into();
        let (text, label) = widget_text_parts(text);
        let response = self.label(text);
        record_widget_with_layout(
            self,
            id,
            &response,
            WidgetRole::Label,
            Some(label.clone()),
            Some(WidgetValue::Text(label)),
            None,
        );
        response
    }

    fn dev_checkbox(
        &mut self,
        id: impl Into<String>,
        value: &mut bool,
        text: impl Into<egui::WidgetText>,
    ) -> egui::Response {
        let id = id.into();
        if let Some(updated) = take_bool_override(self, &id) {
            *value = updated;
        }
        let (text, label) = widget_text_parts(text);
        let response = self.checkbox(value, text);
        record_widget_with_layout(
            self,
            id,
            &response,
            WidgetRole::Checkbox,
            Some(label),
            Some(WidgetValue::Bool(*value)),
            None,
        );
        response
    }

    fn dev_checkbox_with(
        &mut self,
        id: impl Into<String>,
        value: &mut bool,
        text: impl Into<egui::WidgetText>,
        options: CheckboxOptions,
    ) -> egui::Response {
        let id = id.into();
        if let Some(updated) = take_bool_override(self, &id) {
            *value = updated;
        }
        let (text, label) = widget_text_parts(text);
        let response =
            self.add(egui::Checkbox::new(value, text).indeterminate(options.indeterminate));
        record_widget_with_layout(
            self,
            id,
            &response,
            WidgetRole::Checkbox,
            Some(label),
            Some(WidgetValue::Bool(*value)),
            Some(RoleState::Checkbox {
                indeterminate: options.indeterminate,
            }),
        );
        response
    }

    fn dev_text_edit(&mut self, id: impl Into<String>, text: &mut String) -> egui::Response {
        self.dev_text_edit_with(id, text, TextEditOptions::default())
    }

    fn dev_text_edit_with(
        &mut self,
        id: impl Into<String>,
        text: &mut String,
        options: TextEditOptions,
    ) -> egui::Response {
        let id = id.into();
        if let Some(updated) = take_text_override(self, &id) {
            *text = updated;
        }
        let builder = if options.multiline {
            egui::TextEdit::multiline(text)
        } else {
            egui::TextEdit::singleline(text)
        }
        .password(options.password);
        let response = self.add(builder);
        record_widget_with_layout(
            self,
            id,
            &response,
            WidgetRole::TextEdit,
            None,
            Some(WidgetValue::Text(text.clone())),
            Some(RoleState::TextEdit {
                multiline: options.multiline,
                password: options.password,
            }),
        );
        response
    }

    fn dev_slider(
        &mut self,
        id: impl Into<String>,
        value: &mut f32,
        range: RangeInclusive<f32>,
    ) -> egui::Response {
        let id = id.into();
        let range_meta = widget_range_from_f32(&range);
        if let Some(updated) = take_float_override(self, &id) {
            *value = updated;
        }
        let response = self.add(egui::Slider::new(value, range));
        record_widget_with_layout(
            self,
            id,
            &response,
            WidgetRole::Slider,
            None,
            Some(WidgetValue::Float(f64::from(*value))),
            Some(RoleState::Slider { range: range_meta }),
        );
        response
    }

    fn dev_combo_box<T: ToString>(
        &mut self,
        id: impl Into<String>,
        label: impl Into<String>,
        selected: &mut usize,
        options: &[T],
    ) -> egui::Response {
        let id = id.into();
        if let Some(updated) = take_usize_override(self, &id) {
            *selected = updated;
        }
        let label_text = label.into();
        let option_labels = options.iter().map(ToString::to_string).collect::<Vec<_>>();
        let len = options.len();
        if len == 0 || *selected >= len {
            *selected = 0;
        }
        let response = if len == 0 {
            egui::ComboBox::from_label(label_text.as_str())
                .selected_text("")
                .show_ui(self, |_| {})
                .response
        } else {
            let selected_text = options
                .get(*selected)
                .map(ToString::to_string)
                .unwrap_or_default();
            egui::ComboBox::from_label(label_text.as_str())
                .selected_text(selected_text)
                .show_index(self, selected, len, |index| options[index].to_string())
        };
        record_widget_with_layout(
            self,
            id,
            &response,
            WidgetRole::ComboBox,
            Some(label_text),
            Some(WidgetValue::Int(*selected as i64)),
            Some(RoleState::ComboBox {
                options: option_labels,
            }),
        );
        response
    }

    fn dev_drag_value(&mut self, id: impl Into<String>, value: &mut f32) -> egui::Response {
        let id = id.into();
        if let Some(updated) = take_float_override(self, &id) {
            *value = updated;
        }
        let response = self.add(egui::DragValue::new(value));
        record_widget_with_layout(
            self,
            id,
            &response,
            WidgetRole::DragValue,
            None,
            Some(WidgetValue::Float(f64::from(*value))),
            Some(RoleState::DragValue { range: None }),
        );
        response
    }

    fn dev_drag_value_range(
        &mut self,
        id: impl Into<String>,
        value: &mut f32,
        range: RangeInclusive<f32>,
    ) -> egui::Response {
        let id = id.into();
        let range_meta = widget_range_from_f32(&range);
        if let Some(updated) = take_float_override(self, &id) {
            *value = updated;
        }
        let response = self.add(egui::DragValue::new(value).range(range));
        record_widget_with_layout(
            self,
            id,
            &response,
            WidgetRole::DragValue,
            None,
            Some(WidgetValue::Float(f64::from(*value))),
            Some(RoleState::DragValue {
                range: Some(range_meta),
            }),
        );
        response
    }

    fn dev_drag_value_i32(&mut self, id: impl Into<String>, value: &mut i32) -> egui::Response {
        let id = id.into();
        if let Some(updated) = take_i32_override(self, &id) {
            *value = updated;
        }
        let response = self.add(egui::DragValue::new(value));
        record_widget_with_layout(
            self,
            id,
            &response,
            WidgetRole::DragValue,
            None,
            Some(WidgetValue::Int(i64::from(*value))),
            Some(RoleState::DragValue { range: None }),
        );
        response
    }

    fn dev_drag_value_i32_range(
        &mut self,
        id: impl Into<String>,
        value: &mut i32,
        range: RangeInclusive<i32>,
    ) -> egui::Response {
        let id = id.into();
        let range_meta = widget_range_from_i32(&range);
        if let Some(updated) = take_i32_override(self, &id) {
            *value = updated;
        }
        let response = self.add(egui::DragValue::new(value).range(range));
        record_widget_with_layout(
            self,
            id,
            &response,
            WidgetRole::DragValue,
            None,
            Some(WidgetValue::Int(i64::from(*value))),
            Some(RoleState::DragValue {
                range: Some(range_meta),
            }),
        );
        response
    }

    fn dev_text_edit_multiline(
        &mut self,
        id: impl Into<String>,
        text: &mut String,
    ) -> egui::Response {
        self.dev_text_edit_with(
            id,
            text,
            TextEditOptions {
                multiline: true,
                password: false,
            },
        )
    }

    fn dev_toggle_value(
        &mut self,
        id: impl Into<String>,
        selected: &mut bool,
        text: impl Into<egui::WidgetText>,
    ) -> egui::Response {
        let id = id.into();
        if let Some(updated) = take_bool_override(self, &id) {
            *selected = updated;
        }
        let (text, label) = widget_text_parts(text);
        let response = self.toggle_value(selected, text);
        record_widget_with_layout(
            self,
            id,
            &response,
            WidgetRole::Toggle,
            Some(label),
            Some(WidgetValue::Bool(*selected)),
            None,
        );
        response
    }

    fn dev_radio_value<V: PartialEq + Clone>(
        &mut self,
        id: impl Into<String>,
        current: &mut V,
        alternative: V,
        text: impl Into<egui::WidgetText>,
    ) -> egui::Response {
        record_choice_widget(
            self,
            id.into(),
            current,
            alternative,
            text,
            ChoiceWidgetMeta::new(WidgetRole::Radio),
            Self::radio_value,
        )
    }

    fn dev_selectable_value<V: PartialEq + Clone>(
        &mut self,
        id: impl Into<String>,
        current: &mut V,
        alternative: V,
        text: impl Into<egui::WidgetText>,
    ) -> egui::Response {
        record_choice_widget(
            self,
            id.into(),
            current,
            alternative,
            text,
            ChoiceWidgetMeta::new(WidgetRole::Selectable),
            Self::selectable_value,
        )
    }

    fn dev_separator(&mut self, id: impl Into<String>) -> egui::Response {
        let id = id.into();
        let response = self.separator();
        record_widget_with_layout(self, id, &response, WidgetRole::Separator, None, None, None);
        response
    }

    fn dev_spinner(&mut self, id: impl Into<String>) -> egui::Response {
        let id = id.into();
        let response = self.spinner();
        record_widget_with_layout(self, id, &response, WidgetRole::Spinner, None, None, None);
        response
    }

    fn dev_progress_bar(&mut self, id: impl Into<String>, progress: f32) -> egui::Response {
        self.dev_progress_bar_with(id, progress, ProgressBarOptions::default())
    }

    fn dev_progress_bar_with(
        &mut self,
        id: impl Into<String>,
        progress: f32,
        options: ProgressBarOptions,
    ) -> egui::Response {
        let id = id.into();
        let progress = progress.clamp(0.0, 1.0);
        let mut widget = egui::ProgressBar::new(progress);
        let label = if let Some(text) = options.text {
            widget = widget.text(text.clone());
            Some(text)
        } else if options.show_percentage {
            widget = widget.show_percentage();
            Some(format!("{}%", (progress * 100.0) as usize))
        } else {
            None
        };
        let response = self.add(widget);
        record_widget_with_layout(
            self,
            id,
            &response,
            WidgetRole::ProgressBar,
            label,
            Some(WidgetValue::Float(f64::from(progress))),
            None,
        );
        response
    }

    fn dev_color_edit(
        &mut self,
        id: impl Into<String>,
        color: &mut egui::Color32,
    ) -> egui::Response {
        let id = id.into();
        if let Some(updated) = take_color_override(self, &id) {
            *color = updated;
        }
        let response = self.color_edit_button_srgba(color);
        record_widget_with_layout(
            self,
            id,
            &response,
            WidgetRole::ColorPicker,
            None,
            Some(WidgetValue::Text(format_color_hex(*color))),
            None,
        );
        response
    }

    fn dev_menu_button<R>(
        &mut self,
        id: impl Into<String>,
        text: impl Into<egui::WidgetText>,
        add_contents: impl FnOnce(&mut Self) -> R,
    ) -> egui::InnerResponse<Option<R>> {
        let id = id.into();
        let menu_tag = format!("{id}.menu");
        let (text, label) = widget_text_parts(text);
        let output = self.menu_button(text, |ui| container(ui, menu_tag, add_contents));
        record_widget_with_layout(
            self,
            id,
            &output.response,
            WidgetRole::MenuButton,
            Some(label),
            Some(WidgetValue::Bool(output.inner.is_some())),
            None,
        );
        output
    }

    fn dev_collapsing<R>(
        &mut self,
        id: impl Into<String>,
        open: &mut bool,
        heading: impl Into<egui::WidgetText>,
        add_contents: impl FnOnce(&mut Self) -> R,
    ) -> CollapsingResponse<R> {
        let id = id.into();
        if let Some(updated) = take_bool_override(self, &id) {
            *open = updated;
        }
        let body_tag = format!("{id}.body");
        let (heading, label) = widget_text_parts(heading);
        let output = egui::CollapsingHeader::new(heading)
            .id_salt(id.as_str())
            .open(Some(*open))
            .show(self, |ui| container(ui, body_tag, add_contents));
        *open = !output.fully_closed();
        record_widget_with_layout(
            self,
            id,
            &output.header_response,
            WidgetRole::CollapsingHeader,
            Some(label),
            Some(WidgetValue::Bool(*open)),
            None,
        );
        output
    }
}

/// Helper extensions for recording scroll areas with explicit ids.
pub trait DevScrollAreaExt {
    /// Show a scroll area and record it with DevMCP metadata.
    fn dev_show<R>(
        self,
        ui: &mut egui::Ui,
        id: impl Into<String>,
        add_contents: impl FnOnce(&mut egui::Ui) -> R,
    ) -> ScrollAreaOutput<R>;

    /// Show a scroll area with viewport access and record it with DevMCP metadata.
    fn dev_show_viewport<R>(
        self,
        ui: &mut egui::Ui,
        id: impl Into<String>,
        add_contents: impl FnOnce(&mut egui::Ui, egui::Rect) -> R,
    ) -> ScrollAreaOutput<R>;
}

impl DevScrollAreaExt for egui::ScrollArea {
    fn dev_show<R>(
        self,
        ui: &mut egui::Ui,
        id: impl Into<String>,
        add_contents: impl FnOnce(&mut egui::Ui) -> R,
    ) -> ScrollAreaOutput<R> {
        self.dev_show_viewport(ui, id, |ui, _| add_contents(ui))
    }

    fn dev_show_viewport<R>(
        self,
        ui: &mut egui::Ui,
        id: impl Into<String>,
        add_contents: impl FnOnce(&mut egui::Ui, egui::Rect) -> R,
    ) -> ScrollAreaOutput<R> {
        let id = id.into();
        let output = self.show_viewport(ui, |ui, rect| {
            let _guard = super::instrument::begin_container(ui, id.clone());
            add_contents(ui, rect)
        });
        super::instrument::record_scroll_area(ui, id, &output);
        output
    }
}

/// Consume a queued widget value override for a custom instrumented widget.
pub fn take_widget_value_override(ui: &egui::Ui, id: &str) -> Option<WidgetValue> {
    let inner = active_inner()?;
    let viewport_id = ui.ctx().viewport_id();
    inner.take_widget_value_update(viewport_id, id)
}

fn widget_text_parts(text: impl Into<egui::WidgetText>) -> (egui::WidgetText, String) {
    let text = text.into();
    let label = text.text().to_string();
    (text, label)
}

fn take_bool_override(ui: &egui::Ui, id: &str) -> Option<bool> {
    match take_widget_value_override(ui, id) {
        Some(WidgetValue::Bool(updated)) => Some(updated),
        _ => None,
    }
}

fn take_text_override(ui: &egui::Ui, id: &str) -> Option<String> {
    match take_widget_value_override(ui, id) {
        Some(WidgetValue::Text(updated)) => Some(updated),
        _ => None,
    }
}

fn take_color_override(ui: &egui::Ui, id: &str) -> Option<egui::Color32> {
    match take_widget_value_override(ui, id) {
        Some(WidgetValue::Text(updated)) => parse_color_hex(&updated),
        _ => None,
    }
}

fn take_float_override(ui: &egui::Ui, id: &str) -> Option<f32> {
    match take_widget_value_override(ui, id) {
        Some(WidgetValue::Float(updated)) => Some(updated as f32),
        Some(WidgetValue::Int(updated)) => Some(updated as f32),
        _ => None,
    }
}

fn take_i32_override(ui: &egui::Ui, id: &str) -> Option<i32> {
    match take_widget_value_override(ui, id) {
        Some(WidgetValue::Int(updated)) => i32::try_from(updated).ok(),
        Some(WidgetValue::Float(updated)) => Some(updated as i32),
        _ => None,
    }
}

fn take_usize_override(ui: &egui::Ui, id: &str) -> Option<usize> {
    match take_widget_value_override(ui, id) {
        Some(WidgetValue::Int(updated)) => usize::try_from(updated).ok(),
        Some(WidgetValue::Float(updated)) => Some(updated as usize),
        _ => None,
    }
}

struct ChoiceWidgetMeta {
    role: WidgetRole,
}

impl ChoiceWidgetMeta {
    fn new(role: WidgetRole) -> Self {
        Self { role }
    }
}

fn record_choice_widget<V>(
    ui: &mut egui::Ui,
    id: String,
    current: &mut V,
    alternative: V,
    text: impl Into<egui::WidgetText>,
    meta: ChoiceWidgetMeta,
    add_widget: impl FnOnce(&mut egui::Ui, &mut V, V, egui::WidgetText) -> egui::Response,
) -> egui::Response
where
    V: PartialEq + Clone,
{
    let (text, label) = widget_text_parts(text);
    let selected_value = alternative.clone();
    if take_bool_override(ui, &id).is_some_and(|updated| updated) {
        *current = alternative.clone();
    }
    let response = add_widget(ui, current, alternative, text);
    let selected = *current == selected_value;
    record_widget_with_layout(
        ui,
        id,
        &response,
        meta.role,
        Some(label),
        Some(WidgetValue::Bool(selected)),
        None,
    );
    response
}

/// Record a standard widget with consistent metadata, visibility, and active context lookup.
fn record_widget_with_layout(
    ui: &egui::Ui,
    id: String,
    response: &egui::Response,
    role: WidgetRole,
    label: Option<String>,
    value: Option<WidgetValue>,
    role_state: Option<RoleState>,
) {
    let Some(inner) = active_inner() else {
        return;
    };
    let visible = ui.is_visible() && ui.is_rect_visible(response.rect);
    let layout = Some(capture_layout(ui, response));
    swallow_panic("record_widget_with_layout", || {
        record_widget(
            &inner.widgets,
            id,
            response,
            WidgetMeta {
                role,
                label,
                value,
                layout,
                role_state,
                visible,
                ..Default::default()
            },
        );
    });
}

fn widget_range_from_f32(range: &RangeInclusive<f32>) -> WidgetRange {
    WidgetRange {
        min: f64::from(*range.start()),
        max: f64::from(*range.end()),
    }
}

fn widget_range_from_i32(range: &RangeInclusive<i32>) -> WidgetRange {
    WidgetRange {
        min: f64::from(*range.start()),
        max: f64::from(*range.end()),
    }
}

/// Parse a CSS-style `#RRGGBB` or `#RRGGBBAA` color literal.
pub fn parse_color_hex(value: &str) -> Option<egui::Color32> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 8 {
        return None;
    }
    let bytes = u32::from_str_radix(hex, 16).ok()?;
    let r = ((bytes >> 24) & 0xff) as u8;
    let g = ((bytes >> 16) & 0xff) as u8;
    let b = ((bytes >> 8) & 0xff) as u8;
    let a = (bytes & 0xff) as u8;
    Some(egui::Color32::from_rgba_unmultiplied(r, g, b, a))
}

pub fn format_color_hex(color: egui::Color32) -> String {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
}
