//! Thin wrappers over egui widgets that, when the `eguidev` feature is enabled
//! (native only), register the widget with eguidev under a stable `id` for
//! automation. With the feature off (e.g. wasm), they fall back to plain egui
//! calls and the `id` is ignored. This keeps call sites identical across
//! targets.

use bevy_egui::egui;

#[cfg(all(feature = "eguidev", not(target_arch = "wasm32")))]
use eguidev::DevUiExt;

/// A button labelled `text`, registered for automation as `id`.
pub fn button(ui: &mut egui::Ui, id: &str, text: impl Into<egui::WidgetText>) -> egui::Response {
    #[cfg(all(feature = "eguidev", not(target_arch = "wasm32")))]
    {
        let _ = id;
        return ui.dev_button(id, text);
    }
    #[cfg(not(all(feature = "eguidev", not(target_arch = "wasm32"))))]
    {
        let _ = id;
        ui.button(text)
    }
}

/// A label showing `text`, registered for automation as `id`.
pub fn label(ui: &mut egui::Ui, id: &str, text: impl Into<egui::WidgetText>) -> egui::Response {
    #[cfg(all(feature = "eguidev", not(target_arch = "wasm32")))]
    {
        let _ = id;
        return ui.dev_label(id, text);
    }
    #[cfg(not(all(feature = "eguidev", not(target_arch = "wasm32"))))]
    {
        let _ = id;
        ui.label(text)
    }
}

/// Register a hand-painted region (one that doesn't go through a standard egui
/// widget) as an automation widget covering `response`'s rect. No-op without
/// the `eguidev` feature.
#[allow(unused_variables)]
pub fn track(id: &str, response: &egui::Response, role_label: &str) {
    #[cfg(all(feature = "eguidev", not(target_arch = "wasm32")))]
    {
        eguidev::track_response_full(
            id,
            response,
            eguidev::WidgetMeta {
                role: eguidev::WidgetRole::Unknown,
                label: Some(role_label.to_string()),
                visible: true,
                ..Default::default()
            },
        );
    }
}
