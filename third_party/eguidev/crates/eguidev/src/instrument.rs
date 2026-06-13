//! Instrumentation and geometry recording module.
#[cfg(test)]
use std::cell::Cell;
use std::{
    cell::RefCell,
    panic::{self, AssertUnwindSafe},
    sync::Arc,
};

use egui::scroll_area::{ScrollAreaOutput, State};

use crate::{
    registry::Inner,
    types::{RoleState, WidgetLayout, WidgetRole, WidgetValue},
    ui_ext::DevScrollAreaExt,
    widget_registry::{WidgetMeta, record_widget},
};

thread_local! {
    pub(crate) static ACTIVE: RefCell<Option<Arc<Inner>>> = const { RefCell::new(None) };
}

#[cfg(test)]
thread_local! {
    static TEST_LAYOUT_CAPTURE_COUNT: Cell<usize> = const { Cell::new(0) };
}

pub fn swallow_panic(label: &'static str, f: impl FnOnce()) {
    if panic::catch_unwind(AssertUnwindSafe(f)).is_err() {
        eprintln!("eguidev: recovered panic in {label}");
    }
}

pub fn active_inner() -> Option<Arc<Inner>> {
    ACTIVE.with(|active| {
        active
            .try_borrow()
            .ok()
            .and_then(|guard| guard.as_ref().cloned())
    })
}

/// Record a widget with an explicit id and geometry only.
///
/// Prefer `DevUiExt` for standard widgets. Use `id` for custom widgets where metadata is
/// unnecessary.
pub fn id(
    ui: &mut egui::Ui,
    id: impl Into<String>,
    add: impl FnOnce(&mut egui::Ui) -> egui::Response,
) -> egui::Response {
    let response = add(ui);
    let visible = ui.is_visible() && ui.is_rect_visible(response.rect);
    track_response_full(
        id,
        &response,
        WidgetMeta {
            visible,
            ..Default::default()
        },
    );
    response
}

/// Record a widget with an explicit id and explicit metadata.
///
/// Use this when you need control over role/type/value/label metadata for custom widgets.
pub fn id_with_meta(
    ui: &mut egui::Ui,
    id: impl Into<String>,
    role: WidgetRole,
    label: Option<String>,
    value: Option<WidgetValue>,
    add: impl FnOnce(&mut egui::Ui) -> egui::Response,
) -> egui::Response {
    let response = add(ui);
    let Some(inner) = active_inner() else {
        return response;
    };
    let visible = ui.is_visible() && ui.is_rect_visible(response.rect);
    let layout = Some(capture_layout(ui, &response));
    let id = id.into();
    swallow_panic("id_with_meta", || {
        record_widget(
            &inner.widgets,
            id,
            &response,
            WidgetMeta {
                role,
                label,
                value,
                layout,
                visible,
                ..Default::default()
            },
        );
    });
    response
}

/// Track an already-created widget response with explicit metadata.
///
/// Use this when you have an `egui::Response` from a custom widget and cannot
/// wrap it with [`id`] or [`id_with_meta`]. Prefer the `DevUiExt` helpers or
/// the wrapping functions for standard widgets.
pub fn track_response_full(id: impl Into<String>, response: &egui::Response, meta: WidgetMeta) {
    let Some(inner) = active_inner() else {
        return;
    };
    let id = id.into();
    swallow_panic("track_response_full", || {
        record_widget(&inner.widgets, id, response, meta);
    });
}

/// RAII guard that pops a container scope when dropped.
pub struct ContainerGuard {
    inner: Option<Arc<Inner>>,
    viewport_id: egui::ViewportId,
}

impl Drop for ContainerGuard {
    fn drop(&mut self) {
        if let Some(inner) = &self.inner {
            inner.widgets.pop_container(self.viewport_id);
        }
    }
}

/// Begin a container scope with an explicit id.
pub fn begin_container(ui: &egui::Ui, id: impl Into<String>) -> ContainerGuard {
    let inner = active_inner();
    begin_container_with_inner(ui, inner, id)
}

fn begin_container_with_inner(
    ui: &egui::Ui,
    inner: Option<Arc<Inner>>,
    id: impl Into<String>,
) -> ContainerGuard {
    let viewport_id = ui.ctx().viewport_id();
    if let Some(inner) = inner.as_ref() {
        inner.widgets.push_container(viewport_id, id.into());
    }
    ContainerGuard { inner, viewport_id }
}

/// Run a closure within a container scope with an explicit id.
///
/// The container is registered as a widget so it is discoverable via `parent()` and
/// `children()`.
pub fn container<R>(
    ui: &mut egui::Ui,
    id: impl Into<String>,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let id = id.into();
    let inner = active_inner();
    // Run contents inside a scope to get a response, with the container on the stack
    // so children get the right parent_id.
    let output = ui.scope(|ui| {
        let _guard = begin_container_with_inner(ui, inner.clone(), id.clone());
        add_contents(ui)
    });
    let Some(inner) = inner else {
        return output.inner;
    };
    // Register the container widget after content, so its rect covers all children.
    // At this point the container has been popped, so the container's own parent_id
    // is correctly set to the enclosing scope.
    swallow_panic("container", || {
        record_widget(
            &inner.widgets,
            id,
            &output.response,
            WidgetMeta {
                visible: output.response.interact_rect.is_positive(),
                ..Default::default()
            },
        );
    });
    output.inner
}

/// Scroll area state tracker with one-shot offset jumps.
#[derive(Debug, Clone)]
pub struct ScrollAreaState {
    offset: egui::Vec2,
    pending_offset: Option<egui::Vec2>,
}

impl Default for ScrollAreaState {
    fn default() -> Self {
        Self {
            offset: egui::Vec2::ZERO,
            pending_offset: None,
        }
    }
}

impl ScrollAreaState {
    /// Create a new scroll area state tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// The most recent scroll offset after egui applied input.
    pub fn offset(&self) -> egui::Vec2 {
        self.offset
    }

    /// Request a one-shot jump to the given offset on the next frame.
    pub fn jump_to(&mut self, offset: egui::Vec2) {
        self.pending_offset = Some(offset);
    }

    /// Reset tracked state and request that the next frame jump back to the origin.
    pub fn reset(&mut self) {
        self.offset = egui::Vec2::ZERO;
        self.pending_offset = Some(egui::Vec2::ZERO);
    }

    /// Show a scroll area, record DevMCP metadata, and update the tracked offset.
    pub fn show<R>(
        &mut self,
        scroll_area: egui::ScrollArea,
        ui: &mut egui::Ui,
        id: impl Into<String>,
        add_contents: impl FnOnce(&mut egui::Ui) -> R,
    ) -> ScrollAreaOutput<R> {
        self.show_viewport(scroll_area, ui, id, |ui, _| add_contents(ui))
    }

    /// Show a scroll area with viewport access, record DevMCP metadata, and update the offset.
    pub fn show_viewport<R>(
        &mut self,
        scroll_area: egui::ScrollArea,
        ui: &mut egui::Ui,
        id: impl Into<String>,
        add_contents: impl FnOnce(&mut egui::Ui, egui::Rect) -> R,
    ) -> ScrollAreaOutput<R> {
        let requested_offset = self.pending_offset;
        let scroll_area = if let Some(offset) = requested_offset {
            scroll_area.scroll_offset(offset)
        } else {
            scroll_area
        };
        // Use our extension trait method
        let output = scroll_area.dev_show_viewport(ui, id, add_contents);
        let effective_offset = State::load(ui.ctx(), output.id)
            .map(|state| state.offset)
            .unwrap_or(output.state.offset);
        self.offset = effective_offset;
        self.pending_offset = requested_offset.filter(|offset| {
            let delta = effective_offset - *offset;
            delta.length_sq() > 0.25
        });
        output
    }
}

fn sanitize_f32(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

pub fn capture_layout(ui: &egui::Ui, response: &egui::Response) -> WidgetLayout {
    #[cfg(test)]
    TEST_LAYOUT_CAPTURE_COUNT.with(|count| count.set(count.get() + 1));
    let desired_size = response.intrinsic_size.unwrap_or(response.rect.size());
    let actual_size = response.rect.size();
    let rect = response.rect;
    let clip_rect = ui.clip_rect();
    let available_rect = ui.available_rect_before_wrap();
    let clipped = !clip_rect.contains_rect(rect);
    let overflow = !available_rect.contains_rect(rect);
    WidgetLayout {
        desired_size: desired_size.into(),
        actual_size: actual_size.into(),
        clip_rect: clip_rect.into(),
        clipped,
        overflow,
        available_rect: available_rect.into(),
        visible_fraction: sanitize_f32(visible_fraction(response.rect, clip_rect)),
    }
}

#[cfg(test)]
pub fn reset_test_counters() {
    TEST_LAYOUT_CAPTURE_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub fn test_layout_capture_count() -> usize {
    TEST_LAYOUT_CAPTURE_COUNT.with(Cell::get)
}

pub fn record_scroll_area<R>(ui: &egui::Ui, id: impl Into<String>, output: &ScrollAreaOutput<R>) {
    let response = ui.interact(output.inner_rect, output.id, egui::Sense::hover());
    let Some(inner) = active_inner() else {
        return;
    };
    let visible = ui.is_visible() && ui.is_rect_visible(response.rect);
    let layout = Some(capture_layout(ui, &response));
    let id = id.into();
    swallow_panic("record_scroll_area", || {
        let viewport_id = response.ctx.viewport_id();
        let widget_id = response.id.value();
        let override_offset = inner.take_scroll_override(viewport_id, widget_id);
        let recorded_offset = override_offset.unwrap_or(output.state.offset);
        if let Some(offset) = override_offset {
            let mut state = State::load(ui.ctx(), output.id).unwrap_or_default();
            state.offset = offset;
            state.store(ui.ctx(), output.id);
            inner.request_repaint();
        }
        record_widget(
            &inner.widgets,
            id,
            &response,
            WidgetMeta {
                role: WidgetRole::ScrollArea,
                layout,
                role_state: Some(RoleState::ScrollArea {
                    offset: recorded_offset.into(),
                    viewport_size: output.inner_rect.size().into(),
                    content_size: output.content_size.into(),
                }),
                visible,
                ..Default::default()
            },
        );
    });
}

fn visible_fraction(rect: egui::Rect, clip_rect: egui::Rect) -> f32 {
    let area = rect.area();
    if area <= 0.0 {
        return 0.0;
    }
    let intersection = rect.intersect(clip_rect);
    (intersection.area() / area).clamp(0.0, 1.0)
}

#[cfg(test)]
#[allow(deprecated)]
#[allow(clippy::tests_outside_test_module)]
mod tests {
    use std::{any::Any, sync::Arc};

    use egui::{Color32, ColorImage, Context, TextureOptions};

    use super::*;
    use crate::{
        DevMcp, WidgetState,
        devmcp::RuntimeHooks,
        registry::Inner,
        types::WidgetRegistryEntry,
        ui_ext::{ButtonOptions, CheckboxOptions, DevUiExt, ProgressBarOptions, TextEditOptions},
    };

    #[derive(Debug)]
    struct NoopRuntimeHooks;

    impl RuntimeHooks for NoopRuntimeHooks {
        fn as_any(&self) -> &(dyn Any + Send + Sync) {
            self
        }
    }

    fn devmcp_enabled() -> DevMcp {
        DevMcp::new().activate_runtime(Arc::new(Inner::new()), Arc::new(NoopRuntimeHooks))
    }

    fn widget_by_id<'a>(widgets: &'a [WidgetRegistryEntry], id: &str) -> &'a WidgetRegistryEntry {
        widgets
            .iter()
            .find(|entry| entry.id == id)
            .unwrap_or_else(|| panic!("missing widget: {id}"))
    }

    fn run_panel(
        ctx: &Context,
        raw_input: egui::RawInput,
        mut add: impl FnMut(&Context, &mut egui::Ui),
    ) {
        let _output = ctx.run_ui(raw_input, |ui| {
            let ctx = ui.ctx().clone();
            egui::Frame::central_panel(ui.style()).show(ui, |ui| add(&ctx, ui));
        });
    }

    fn assert_combo_widget(widget: &WidgetRegistryEntry) {
        assert_eq!(widget.role, WidgetRole::ComboBox);
        assert!(matches!(widget.value, Some(WidgetValue::Int(1))));
    }

    fn assert_drag_float_widget(widget: &WidgetRegistryEntry) {
        assert_eq!(widget.role, WidgetRole::DragValue);
        assert!(matches!(
            widget.value,
            Some(WidgetValue::Float(value)) if (value - 1.5).abs() < f64::EPSILON
        ));
    }

    fn assert_drag_int_widget(widget: &WidgetRegistryEntry) {
        assert_eq!(widget.role, WidgetRole::DragValue);
        assert!(matches!(widget.value, Some(WidgetValue::Int(7))));
    }

    fn assert_label_widget(widget: &WidgetRegistryEntry) {
        assert_eq!(widget.role, WidgetRole::Label);
        assert!(matches!(
            widget.value,
            Some(WidgetValue::Text(ref value)) if value == "Ready"
        ));
    }

    fn assert_text_edit_widget(widget: &WidgetRegistryEntry) {
        assert_eq!(widget.role, WidgetRole::TextEdit);
        assert!(matches!(
            widget.value,
            Some(WidgetValue::Text(ref value)) if value == "Hello"
        ));
    }

    fn assert_toggle_widget(widget: &WidgetRegistryEntry) {
        assert_eq!(widget.role, WidgetRole::Toggle);
        assert!(matches!(widget.value, Some(WidgetValue::Bool(true))));
    }

    fn assert_radio_widget(widget: &WidgetRegistryEntry) {
        assert_eq!(widget.role, WidgetRole::Radio);
        assert!(matches!(widget.value, Some(WidgetValue::Bool(true))));
    }

    fn assert_selectable_widget(widget: &WidgetRegistryEntry) {
        assert_eq!(widget.role, WidgetRole::Selectable);
        assert!(matches!(widget.value, Some(WidgetValue::Bool(true))));
    }

    fn assert_link_widget(widget: &WidgetRegistryEntry) {
        assert_eq!(widget.role, WidgetRole::Link);
        assert_eq!(widget.label.as_deref(), Some("Docs"));
        assert!(widget.value.is_none());
    }

    fn assert_hyperlink_widget(widget: &WidgetRegistryEntry) {
        assert_eq!(widget.role, WidgetRole::Link);
        assert_eq!(widget.label.as_deref(), Some("Reference"));
        assert!(matches!(
            widget.value,
            Some(WidgetValue::Text(ref value)) if value == "https://example.invalid/reference"
        ));
    }

    fn assert_image_widget(widget: &WidgetRegistryEntry) {
        assert_eq!(widget.role, WidgetRole::Image);
        assert_eq!(widget.label.as_deref(), Some("Preview"));
        assert!(matches!(
            widget.value,
            Some(WidgetValue::Text(ref value)) if value == "Preview"
        ));
    }

    fn assert_separator_widget(widget: &WidgetRegistryEntry) {
        assert_eq!(widget.role, WidgetRole::Separator);
    }

    fn assert_spinner_widget(widget: &WidgetRegistryEntry) {
        assert_eq!(widget.role, WidgetRole::Spinner);
    }

    fn assert_menu_button_widget(widget: &WidgetRegistryEntry) {
        assert_eq!(widget.role, WidgetRole::MenuButton);
        assert_eq!(widget.label.as_deref(), Some("Actions"));
        assert!(matches!(widget.value, Some(WidgetValue::Bool(false))));
    }

    fn assert_collapsing_widget(widget: &WidgetRegistryEntry) {
        assert_eq!(widget.role, WidgetRole::CollapsingHeader);
        assert_eq!(widget.label.as_deref(), Some("Advanced"));
        assert!(matches!(widget.value, Some(WidgetValue::Bool(true))));
    }

    #[test]
    fn registry_records_identified_widget() {
        let devmcp = devmcp_enabled();
        let ctx = Context::default();
        let raw_input = egui::RawInput::default();
        run_panel(&ctx, raw_input, |ctx, ui| {
            devmcp.begin_frame(ctx);
            let response = ui.button("Save");
            track_response_full(
                "save",
                &response,
                WidgetMeta {
                    visible: response.interact_rect.is_positive(),
                    ..Default::default()
                },
            );
            devmcp.end_frame(ctx);
        });
        let widgets = devmcp
            .inner()
            .expect("attached inner")
            .widgets
            .widget_list(egui::ViewportId::ROOT)
            .into_iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>();
        assert!(widgets.contains(&"save".to_string()));
    }

    #[test]
    fn dev_ui_ext_records_extended_widgets() {
        let devmcp = devmcp_enabled();
        let ctx = Context::default();
        let preview_texture = ctx.load_texture(
            "instrument.preview",
            ColorImage::filled([4, 4], Color32::from_rgb(64, 156, 255)),
            TextureOptions::LINEAR,
        );
        let raw_input = egui::RawInput::default();
        let mut selected = 1;
        let options = ["One", "Two", "Three"];
        let mut drag_f = 1.5_f32;
        let mut drag_i = 7_i32;
        let mut text = "Hello".to_string();
        let mut toggle = true;
        let mut radio_current = "a".to_string();
        let mut selectable_current = 2_i32;
        let mut advanced_open = true;
        run_panel(&ctx, raw_input, |ctx, ui| {
            devmcp.begin_frame(ctx);
            ui.dev_link("docs", "Docs");
            ui.dev_hyperlink_to(
                "reference",
                "Reference",
                "https://example.invalid/reference",
            );
            ui.dev_image("preview", "Preview", &preview_texture);
            ui.dev_combo_box("combo", "Choice", &mut selected, &options);
            ui.dev_drag_value("drag_f", &mut drag_f);
            ui.dev_drag_value_i32("drag_i", &mut drag_i);
            ui.dev_label("status", "Ready");
            ui.dev_text_edit_multiline("multi", &mut text);
            ui.dev_toggle_value("toggle", &mut toggle, "Toggle");
            ui.dev_radio_value("radio", &mut radio_current, "a".to_string(), "Radio");
            ui.dev_selectable_value("select", &mut selectable_current, 2, "Select");
            ui.dev_separator("separator");
            ui.dev_spinner("spinner");
            let _menu = ui.dev_menu_button("menu", "Actions", |ui| {
                ui.dev_button("menu.item", "Reset");
            });
            let _advanced = ui.dev_collapsing("advanced", &mut advanced_open, "Advanced", |ui| {
                ui.dev_label("advanced.body", "Shown");
            });
            devmcp.end_frame(ctx);
        });

        let widgets = devmcp
            .inner()
            .expect("attached inner")
            .widgets
            .widget_list(egui::ViewportId::ROOT);
        assert_link_widget(widget_by_id(&widgets, "docs"));
        assert_hyperlink_widget(widget_by_id(&widgets, "reference"));
        assert_image_widget(widget_by_id(&widgets, "preview"));
        assert_combo_widget(widget_by_id(&widgets, "combo"));
        assert_drag_float_widget(widget_by_id(&widgets, "drag_f"));
        assert_drag_int_widget(widget_by_id(&widgets, "drag_i"));
        assert_label_widget(widget_by_id(&widgets, "status"));
        assert_text_edit_widget(widget_by_id(&widgets, "multi"));
        assert_toggle_widget(widget_by_id(&widgets, "toggle"));
        assert_radio_widget(widget_by_id(&widgets, "radio"));
        assert_selectable_widget(widget_by_id(&widgets, "select"));
        assert_separator_widget(widget_by_id(&widgets, "separator"));
        assert_spinner_widget(widget_by_id(&widgets, "spinner"));
        assert_menu_button_widget(widget_by_id(&widgets, "menu"));
        assert_collapsing_widget(widget_by_id(&widgets, "advanced"));
        assert_eq!(
            widget_by_id(&widgets, "advanced.body").parent_id.as_deref(),
            Some("advanced.body")
        );
    }

    #[test]
    fn dev_ui_ext_projects_role_state_and_new_roles() {
        let devmcp = devmcp_enabled();
        let ctx = Context::default();
        let raw_input = egui::RawInput::default();
        let mut toolbar_selected = true;
        let mut mixed = true;
        let mut password = "opensesame".to_string();
        let mut slider = 4.0_f32;
        let mut drag = 2_i32;
        let mut combo = 1_usize;
        let mut color = Color32::from_rgba_unmultiplied(64, 156, 255, 255);
        run_panel(&ctx, raw_input, |ctx, ui| {
            devmcp.begin_frame(ctx);
            if ui
                .dev_button_with(
                    "toolbar",
                    "Toolbar",
                    ButtonOptions {
                        selected: toolbar_selected,
                    },
                )
                .clicked()
            {
                toolbar_selected = !toolbar_selected;
            }
            ui.dev_checkbox_with(
                "mixed",
                &mut mixed,
                "Mixed",
                CheckboxOptions {
                    indeterminate: true,
                },
            );
            ui.dev_text_edit_with(
                "password",
                &mut password,
                TextEditOptions {
                    multiline: false,
                    password: true,
                },
            );
            ui.dev_slider("slider", &mut slider, 0.0..=10.0);
            ui.dev_drag_value_i32_range("drag", &mut drag, -5..=20);
            ui.dev_combo_box("combo", "Choice", &mut combo, &["Alpha", "Beta", "Gamma"]);
            ui.dev_progress_bar_with(
                "progress",
                0.42,
                ProgressBarOptions {
                    text: None,
                    show_percentage: true,
                },
            );
            ui.dev_color_edit("accent", &mut color);
            devmcp.end_frame(ctx);
        });

        let widgets = devmcp
            .inner()
            .expect("attached inner")
            .widgets
            .widget_list(egui::ViewportId::ROOT);
        let toolbar = WidgetState::from(widget_by_id(&widgets, "toolbar"));
        assert_eq!(toolbar.selected, Some(true));

        let mixed = WidgetState::from(widget_by_id(&widgets, "mixed"));
        assert_eq!(mixed.indeterminate, Some(true));

        let password = WidgetState::from(widget_by_id(&widgets, "password"));
        assert_eq!(password.multiline, Some(false));
        assert_eq!(password.password, Some(true));

        let slider = WidgetState::from(widget_by_id(&widgets, "slider"));
        assert_eq!(slider.range.map(|range| range.min), Some(0.0));
        assert_eq!(slider.range.map(|range| range.max), Some(10.0));

        let drag = WidgetState::from(widget_by_id(&widgets, "drag"));
        assert_eq!(drag.range.map(|range| range.min), Some(-5.0));
        assert_eq!(drag.range.map(|range| range.max), Some(20.0));

        let combo = WidgetState::from(widget_by_id(&widgets, "combo"));
        assert_eq!(
            combo.options,
            Some(vec![
                "Alpha".to_string(),
                "Beta".to_string(),
                "Gamma".to_string()
            ])
        );

        let progress = WidgetState::from(widget_by_id(&widgets, "progress"));
        assert_eq!(progress.role, WidgetRole::ProgressBar);
        assert_eq!(progress.label.as_deref(), Some("42%"));

        let accent = WidgetState::from(widget_by_id(&widgets, "accent"));
        assert_eq!(accent.role, WidgetRole::ColorPicker);
        assert_eq!(accent.value_text, "#409CFFFF");
    }

    #[test]
    fn layout_metadata_captures_clipping() {
        let devmcp = devmcp_enabled();
        let ctx = Context::default();
        let raw_input = egui::RawInput::default();
        run_panel(&ctx, raw_input, |ctx, ui| {
            devmcp.begin_frame(ctx);
            egui::ScrollArea::vertical()
                .max_height(40.0)
                .show(ui, |ui| {
                    ui.add_space(200.0);
                    ui.dev_button("clipped", "Clipped");
                });
            devmcp.end_frame(ctx);
        });

        let widgets = devmcp
            .inner()
            .expect("attached inner")
            .widgets
            .widget_list(egui::ViewportId::ROOT);
        let clipped = widgets
            .iter()
            .find(|entry| entry.id == "clipped")
            .expect("clipped widget");
        let layout = clipped.layout.as_ref().expect("layout");
        assert!(layout.desired_size.x >= 0.0);
        assert!(layout.actual_size.x >= 0.0);
        assert!(layout.clipped);
        assert!(layout.overflow);
        assert!(layout.visible_fraction < 1.0);
    }

    #[test]
    fn registry_records_scroll_area() {
        use crate::ui_ext::DevScrollAreaExt;
        let devmcp = devmcp_enabled();
        let ctx = Context::default();
        let raw_input = egui::RawInput::default();
        run_panel(&ctx, raw_input, |ctx, ui| {
            devmcp.begin_frame(ctx);
            egui::ScrollArea::vertical()
                .id_salt("scroll_test")
                .max_height(24.0)
                .dev_show(ui, "scroll", |ui| {
                    ui.add_space(200.0);
                });
            devmcp.end_frame(ctx);
        });

        let widgets = devmcp
            .inner()
            .expect("attached inner")
            .widgets
            .widget_list(egui::ViewportId::ROOT);
        let scroll = widgets
            .iter()
            .find(|entry| entry.id == "scroll")
            .expect("scroll widget");
        assert_eq!(scroll.role, WidgetRole::ScrollArea);
        let scroll_meta = scroll
            .role_state
            .as_ref()
            .and_then(RoleState::scroll_state)
            .expect("scroll metadata");
        assert!(scroll_meta.viewport_size.y > 0.0);
        assert!(scroll_meta.content_size.y >= scroll_meta.viewport_size.y);
    }

    #[test]
    fn scroll_area_metadata_does_not_block_child_clicks() {
        use crate::ui_ext::DevScrollAreaExt;

        let devmcp = devmcp_enabled();
        let ctx = Context::default();
        let mut clicked = false;

        run_panel(&ctx, egui::RawInput::default(), |ctx, ui| {
            devmcp.begin_frame(ctx);
            egui::ScrollArea::vertical()
                .id_salt("scroll_click_test")
                .max_height(48.0)
                .dev_show(ui, "scroll", |ui| {
                    if ui.dev_button("child", "Child").clicked() {
                        clicked = true;
                    }
                    ui.add_space(200.0);
                });
            devmcp.end_frame(ctx);
        });

        let widgets = devmcp
            .inner()
            .expect("attached inner")
            .widgets
            .widget_list(egui::ViewportId::ROOT);
        let pos: egui::Pos2 = widget_by_id(&widgets, "child")
            .interact_rect
            .center()
            .into();
        let click_input = egui::RawInput {
            events: vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                },
            ],
            ..Default::default()
        };

        run_panel(&ctx, click_input, |ctx, ui| {
            devmcp.begin_frame(ctx);
            egui::ScrollArea::vertical()
                .id_salt("scroll_click_test")
                .max_height(48.0)
                .dev_show(ui, "scroll", |ui| {
                    if ui.dev_button("child", "Child").clicked() {
                        clicked = true;
                    }
                    ui.add_space(200.0);
                });
            devmcp.end_frame(ctx);
        });

        assert!(
            clicked,
            "expected child button click to survive scroll-area metadata"
        );
    }

    #[test]
    fn scroll_area_state_reset_requests_origin_jump() {
        let devmcp = devmcp_enabled();
        let ctx = Context::default();
        let mut state = ScrollAreaState::new();

        let render = |ctx: &Context, state: &mut ScrollAreaState| {
            run_panel(ctx, egui::RawInput::default(), |ctx, ui| {
                devmcp.begin_frame(ctx);
                state.show(
                    egui::ScrollArea::vertical()
                        .id_salt("scroll_reset_test")
                        .max_height(24.0),
                    ui,
                    "scroll_reset",
                    |ui| {
                        ui.add_space(200.0);
                    },
                );
                devmcp.end_frame(ctx);
            });
        };

        state.jump_to(egui::vec2(0.0, 100.0));
        render(&ctx, &mut state);
        assert!(
            state.offset().y > 0.0,
            "expected jump_to to move scroll state"
        );

        state.reset();
        render(&ctx, &mut state);
        assert_eq!(
            state.offset().y,
            0.0,
            "expected reset to jump back to origin"
        );
    }

    #[test]
    fn scroll_area_state_reflects_pending_tool_override() {
        let devmcp = devmcp_enabled();
        let ctx = Context::default();
        let mut state = ScrollAreaState::new();

        let render = |ctx: &Context, state: &mut ScrollAreaState| {
            run_panel(ctx, egui::RawInput::default(), |ctx, ui| {
                devmcp.begin_frame(ctx);
                state.show(
                    egui::ScrollArea::vertical()
                        .id_salt("scroll_override_test")
                        .max_height(24.0),
                    ui,
                    "scroll_override",
                    |ui| {
                        ui.add_space(200.0);
                    },
                );
                devmcp.end_frame(ctx);
            });
        };

        render(&ctx, &mut state);
        let widgets = devmcp
            .inner()
            .expect("attached inner")
            .widgets
            .widget_list(egui::ViewportId::ROOT);
        let scroll = widget_by_id(&widgets, "scroll_override");
        let widget_id = scroll.native_id;

        devmcp.inner().expect("attached inner").set_scroll_override(
            egui::ViewportId::ROOT,
            widget_id,
            egui::vec2(0.0, 80.0),
        );
        render(&ctx, &mut state);

        assert_eq!(
            state.offset().y,
            80.0,
            "expected tool-driven override to update tracked offset immediately"
        );
    }

    #[test]
    fn container_scopes_track_parent_id() {
        let devmcp = devmcp_enabled();
        let ctx = Context::default();
        let raw_input = egui::RawInput::default();
        run_panel(&ctx, raw_input, |ctx, ui| {
            devmcp.begin_frame(ctx);
            container(ui, "outer", |ui| {
                container(ui, "inner", |ui| {
                    ui.dev_button("leaf", "Leaf");
                });
            });
            devmcp.end_frame(ctx);
        });

        let widgets = devmcp
            .inner()
            .expect("attached inner")
            .widgets
            .widget_list(egui::ViewportId::ROOT);
        let leaf = widgets
            .iter()
            .find(|entry| entry.id == "leaf")
            .expect("leaf widget");
        assert_eq!(leaf.parent_id.as_deref(), Some("inner"));
    }

    #[test]
    fn metadata_captures_enabled_and_visible() {
        let devmcp = devmcp_enabled();
        let ctx = Context::default();
        let raw_input = egui::RawInput::default();
        run_panel(&ctx, raw_input, |ctx, ui| {
            devmcp.begin_frame(ctx);
            ui.dev_button("enabled_btn", "Enabled");
            ui.add_enabled_ui(false, |ui| {
                ui.dev_button("disabled_btn", "Disabled");
            });
            ui.dev_label("visible_lbl", "Visible");
            egui::ScrollArea::vertical()
                .max_height(10.0)
                .show(ui, |ui| {
                    ui.add_space(100.0);
                    ui.dev_label("clipped_lbl", "Clipped");
                });
            devmcp.end_frame(ctx);
        });

        let widgets = devmcp
            .inner()
            .expect("attached inner")
            .widgets
            .widget_list(egui::ViewportId::ROOT);

        let enabled_btn = widget_by_id(&widgets, "enabled_btn");
        assert!(enabled_btn.enabled, "enabled_btn should be enabled");
        assert!(enabled_btn.visible, "enabled_btn should be visible");

        let disabled_btn = widget_by_id(&widgets, "disabled_btn");
        assert!(!disabled_btn.enabled, "disabled_btn should be disabled");

        let visible_lbl = widget_by_id(&widgets, "visible_lbl");
        assert!(visible_lbl.visible, "visible_lbl should be visible");

        let clipped_lbl = widget_by_id(&widgets, "clipped_lbl");
        assert!(
            !clipped_lbl.visible,
            "clipped_lbl should be invisible (clipped)"
        );
    }
}
