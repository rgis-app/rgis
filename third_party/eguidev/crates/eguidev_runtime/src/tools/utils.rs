use std::{
    future::Future,
    time::{Duration, Instant},
};

use egui::PointerButton;
use serde_json::{Value, json};
use tokio::time::{sleep, timeout};

use crate::{
    actions::{ActionTiming, InputAction},
    error::{ErrorCode, ToolError},
    overlay::{OverlayDebugOptions, parse_color},
    registry::{Inner, viewport_id_to_string},
    runtime::Runtime,
    tools::types::OverlayDebugOptionsInput,
    types::{
        Modifiers, Pos2, Rect, RoleState, Vec2, WidgetRef, WidgetRegistryEntry, WidgetRole,
        WidgetState, WidgetValue,
    },
    ui_ext::parse_color_hex,
    viewports::ViewportSnapshot,
};

const FRAME_DURATION_MS: u64 = 16;

pub fn resolve_widget_and_viewport(
    inner: &Inner,
    viewport_id: Option<&str>,
    target: &WidgetRef,
) -> Result<(WidgetRegistryEntry, egui::ViewportId), ToolError> {
    ensure_automation_ready(inner)?;
    let widget = inner
        .widgets
        .resolve_widget(&inner.viewports, viewport_id, target)?;
    let viewport_id = inner
        .viewports
        .resolve_viewport_id(Some(widget.viewport_id.clone()))?;
    Ok((widget, viewport_id))
}

pub fn ensure_automation_ready(inner: &Inner) -> Result<(), ToolError> {
    if let Some(error) = inner.widgets.duplicate_explicit_id_error() {
        return Err(error.into());
    }
    Ok(())
}

pub fn queue_click(
    inner: &Inner,
    viewport_id: egui::ViewportId,
    pos: Pos2,
    button: PointerButton,
    modifiers: Modifiers,
    click_count: u8,
) {
    inner.queue_action(viewport_id, InputAction::PointerMove { pos });
    for _ in 0..click_count {
        inner.queue_action(
            viewport_id,
            InputAction::PointerButton {
                pos,
                button,
                pressed: true,
                modifiers,
            },
        );
        inner.queue_action(
            viewport_id,
            InputAction::PointerButton {
                pos,
                button,
                pressed: false,
                modifiers,
            },
        );
    }
}

pub fn queue_primary_click(inner: &Inner, viewport_id: egui::ViewportId, pos: Pos2) {
    queue_click(
        inner,
        viewport_id,
        pos,
        PointerButton::Primary,
        Modifiers::default(),
        1,
    );
}

pub fn queue_drag(
    inner: &Inner,
    viewport_id: egui::ViewportId,
    start: Pos2,
    end: Pos2,
    modifiers: Modifiers,
) {
    inner.queue_action(viewport_id, InputAction::PointerMove { pos: start });
    inner.queue_action(
        viewport_id,
        InputAction::PointerButton {
            pos: start,
            button: PointerButton::Primary,
            pressed: true,
            modifiers,
        },
    );
    // Add an intermediate frame with ONLY the end movement to ensure egui
    // processes the drag delta before the release.
    inner.queue_action_with_timing(
        viewport_id,
        ActionTiming::Next,
        InputAction::PointerMove { pos: end },
    );
    // Release in the frame after that.
    inner.queue_action_with_timing(
        viewport_id,
        ActionTiming::AfterNext,
        InputAction::PointerButton {
            pos: end,
            button: PointerButton::Primary,
            pressed: false,
            modifiers,
        },
    );
}

pub fn resolve_relative_pos(rect: Rect, relative: Vec2) -> Result<Pos2, ToolError> {
    if !(0.0..=1.0).contains(&relative.x) || !(0.0..=1.0).contains(&relative.y) {
        return Err(ToolError::new(
            ErrorCode::InvalidRef,
            "Relative drag coordinates must be between 0 and 1",
        ));
    }
    let width = rect.max.x - rect.min.x;
    let height = rect.max.y - rect.min.y;
    if width <= 0.0 || height <= 0.0 {
        return Err(ToolError::new(
            ErrorCode::InvalidRef,
            "Widget rect is empty",
        ));
    }
    Ok(Pos2 {
        x: rect.min.x + width * relative.x,
        y: rect.min.y + height * relative.y,
    })
}

pub fn ensure_positive_vec2(value: Vec2, field: &str) -> Result<(), ToolError> {
    if !value.x.is_finite() || !value.y.is_finite() || value.x <= 0.0 || value.y <= 0.0 {
        return Err(ToolError::new(
            ErrorCode::InvalidRef,
            format!("{field} must be greater than 0"),
        ));
    }
    Ok(())
}

/// Resolve a key name to an `egui::Key`, case-insensitively for multi-character names.
///
/// Single characters are passed through as-is (case-sensitive: `"a"` ≠ `"A"`).
/// Multi-character names are matched case-insensitively: `"enter"`, `"Enter"`, `"ENTER"` all
/// resolve to `egui::Key::Enter`.
pub fn resolve_key_name(name: &str) -> Option<egui::Key> {
    // Single characters: pass through directly (case-sensitive for letters).
    if name.len() == 1 {
        return egui::Key::from_name(name);
    }
    // Multi-character: try exact match first, then case-insensitive lookup.
    if let Some(key) = egui::Key::from_name(name) {
        return Some(key);
    }
    let lower = name.to_ascii_lowercase();
    LOWERCASE_KEY_MAP
        .iter()
        .find(|(lc, _)| *lc == lower)
        .map(|(_, key)| *key)
}

/// All multi-character egui key names mapped as (lowercase, Key).
const LOWERCASE_KEY_MAP: &[(&str, egui::Key)] = &[
    // Navigation
    ("arrowdown", egui::Key::ArrowDown),
    ("down", egui::Key::ArrowDown),
    ("arrowleft", egui::Key::ArrowLeft),
    ("left", egui::Key::ArrowLeft),
    ("arrowright", egui::Key::ArrowRight),
    ("right", egui::Key::ArrowRight),
    ("arrowup", egui::Key::ArrowUp),
    ("up", egui::Key::ArrowUp),
    // Editing
    ("escape", egui::Key::Escape),
    ("esc", egui::Key::Escape),
    ("tab", egui::Key::Tab),
    ("backspace", egui::Key::Backspace),
    ("enter", egui::Key::Enter),
    ("return", egui::Key::Enter),
    ("space", egui::Key::Space),
    // Insert/Delete/etc
    ("help", egui::Key::Insert),
    ("insert", egui::Key::Insert),
    ("delete", egui::Key::Delete),
    ("home", egui::Key::Home),
    ("end", egui::Key::End),
    ("pageup", egui::Key::PageUp),
    ("pagedown", egui::Key::PageDown),
    // Clipboard
    ("copy", egui::Key::Copy),
    ("cut", egui::Key::Cut),
    ("paste", egui::Key::Paste),
    // Punctuation (named forms)
    ("colon", egui::Key::Colon),
    ("comma", egui::Key::Comma),
    ("minus", egui::Key::Minus),
    ("period", egui::Key::Period),
    ("plus", egui::Key::Plus),
    ("equals", egui::Key::Equals),
    ("equal", egui::Key::Equals),
    ("numpadequal", egui::Key::Equals),
    ("semicolon", egui::Key::Semicolon),
    ("backslash", egui::Key::Backslash),
    ("slash", egui::Key::Slash),
    ("pipe", egui::Key::Pipe),
    ("questionmark", egui::Key::Questionmark),
    ("exclamationmark", egui::Key::Exclamationmark),
    ("openbracket", egui::Key::OpenBracket),
    ("closebracket", egui::Key::CloseBracket),
    ("opencurlybracket", egui::Key::OpenCurlyBracket),
    ("closecurlybracket", egui::Key::CloseCurlyBracket),
    ("backtick", egui::Key::Backtick),
    ("backquote", egui::Key::Backtick),
    ("grave", egui::Key::Backtick),
    ("quote", egui::Key::Quote),
    // Digits (named forms)
    ("num0", egui::Key::Num0),
    ("digit0", egui::Key::Num0),
    ("numpad0", egui::Key::Num0),
    ("num1", egui::Key::Num1),
    ("digit1", egui::Key::Num1),
    ("numpad1", egui::Key::Num1),
    ("num2", egui::Key::Num2),
    ("digit2", egui::Key::Num2),
    ("numpad2", egui::Key::Num2),
    ("num3", egui::Key::Num3),
    ("digit3", egui::Key::Num3),
    ("numpad3", egui::Key::Num3),
    ("num4", egui::Key::Num4),
    ("digit4", egui::Key::Num4),
    ("numpad4", egui::Key::Num4),
    ("num5", egui::Key::Num5),
    ("digit5", egui::Key::Num5),
    ("numpad5", egui::Key::Num5),
    ("num6", egui::Key::Num6),
    ("digit6", egui::Key::Num6),
    ("numpad6", egui::Key::Num6),
    ("num7", egui::Key::Num7),
    ("digit7", egui::Key::Num7),
    ("numpad7", egui::Key::Num7),
    ("num8", egui::Key::Num8),
    ("digit8", egui::Key::Num8),
    ("numpad8", egui::Key::Num8),
    ("num9", egui::Key::Num9),
    ("digit9", egui::Key::Num9),
    ("numpad9", egui::Key::Num9),
    // Function keys
    ("f1", egui::Key::F1),
    ("f2", egui::Key::F2),
    ("f3", egui::Key::F3),
    ("f4", egui::Key::F4),
    ("f5", egui::Key::F5),
    ("f6", egui::Key::F6),
    ("f7", egui::Key::F7),
    ("f8", egui::Key::F8),
    ("f9", egui::Key::F9),
    ("f10", egui::Key::F10),
    ("f11", egui::Key::F11),
    ("f12", egui::Key::F12),
    ("f13", egui::Key::F13),
    ("f14", egui::Key::F14),
    ("f15", egui::Key::F15),
    ("f16", egui::Key::F16),
    ("f17", egui::Key::F17),
    ("f18", egui::Key::F18),
    ("f19", egui::Key::F19),
    ("f20", egui::Key::F20),
    ("f21", egui::Key::F21),
    ("f22", egui::Key::F22),
    ("f23", egui::Key::F23),
    ("f24", egui::Key::F24),
    ("f25", egui::Key::F25),
    ("f26", egui::Key::F26),
    ("f27", egui::Key::F27),
    ("f28", egui::Key::F28),
    ("f29", egui::Key::F29),
    ("f30", egui::Key::F30),
    ("f31", egui::Key::F31),
    ("f32", egui::Key::F32),
    ("f33", egui::Key::F33),
    ("f34", egui::Key::F34),
    ("f35", egui::Key::F35),
    // Other
    ("browserback", egui::Key::BrowserBack),
];

/// Parse a key combo string into an egui key and modifiers.
///
/// Format: `[modifier-]...[modifier-]keyname`
///
/// Modifiers (case-insensitive): `ctrl`, `shift`, `alt`, `cmd` (alias: `command`).
/// The last segment after splitting on `-` is the key name; all preceding segments are modifiers.
///
/// Examples: `"enter"`, `"ctrl-a"`, `"shift-tab"`, `"ctrl-shift-z"`, `"cmd-s"`, `"-"` (minus).
/// Returns `(key, modifiers, key_name_str)` where `key_name_str` is the raw key name segment
/// from the combo (preserving original case for single characters).
pub fn parse_key_combo(combo: &str) -> Result<(egui::Key, Modifiers, String), String> {
    if combo.is_empty() {
        return Err("empty key combo".to_string());
    }

    // Split on '-'. The last segment is the key name. But we need to handle edge cases:
    // - bare "-" → segments = ["", ""], key is "-"
    // - "ctrl--" → segments = ["ctrl", "", ""], key is "-"
    // - "ctrl-a" → segments = ["ctrl", "a"]
    let segments: Vec<&str> = combo.split('-').collect();

    // Find the key name: it's the last segment, except when the last segment is empty
    // (meaning the combo ended with '-', so the key is '-' itself).
    let (modifier_segments, key_name) = if segments.len() >= 2 && segments.last() == Some(&"") {
        // Ends with '-', so key is the minus character.
        (&segments[..segments.len() - 2], "-")
    } else {
        (&segments[..segments.len() - 1], *segments.last().unwrap())
    };

    let mut modifiers = Modifiers::default();
    for seg in modifier_segments {
        if seg.is_empty() {
            // Skip empty segments from consecutive dashes.
            continue;
        }
        match seg.to_ascii_lowercase().as_str() {
            "ctrl" => modifiers.ctrl = true,
            "shift" => modifiers.shift = true,
            "alt" => modifiers.alt = true,
            "cmd" | "command" => modifiers.command = true,
            _ => return Err(format!("unknown modifier: {seg}")),
        }
    }

    let key = resolve_key_name(key_name).ok_or_else(|| format!("unknown key: {key_name}"))?;

    Ok((key, modifiers, key_name.to_string()))
}

pub fn printable_key_text(key: &str) -> Option<String> {
    if key == "Space" {
        return Some(" ".to_string());
    }
    let mut chars = key.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    if ch.is_control() {
        return None;
    }
    Some(ch.to_string())
}

pub fn frames_for_duration(duration_ms: u64) -> u64 {
    duration_ms.div_ceil(FRAME_DURATION_MS)
}

pub async fn wait_for_frames(
    inner: &Inner,
    frames: u64,
    start: Instant,
    timeout_ms: u64,
) -> Result<(), ToolError> {
    let mut completed = 0u64;
    let runtime =
        Runtime::from_inner(inner).expect("runtime wait helpers require an attached runtime");
    while completed < frames {
        let elapsed_ms = start.elapsed().as_millis() as u64;
        if elapsed_ms >= timeout_ms {
            return Err(ToolError::new(
                ErrorCode::Internal,
                "Timed out waiting for frame notifications",
            ));
        }
        // Request a repaint and wait for the frame to complete. Use a short
        // poll interval so we re-request repaint if the event loop stalls
        // (e.g. macOS throttles a background window).
        let notified = runtime.frame_notify().notified();
        inner.request_repaint();
        let remaining = timeout_ms.saturating_sub(elapsed_ms).max(1);
        let poll = Duration::from_millis(FRAME_DURATION_MS).min(Duration::from_millis(remaining));
        if timeout(poll, notified).await.is_ok() {
            completed += 1;
        }
    }
    Ok(())
}

/// Generic utility for polling a condition that requires UI interaction or state updates.
///
/// This function handles the boilerplate of checking a condition, tracking elapsed time
/// against a timeout, and efficiently waiting for `egui` frame updates.
///
/// `condition` should return `Ok((matched, state))` where `state` is some snapshot or
/// context to return to the caller (e.g., the last seen widget state or viewports).
///
/// `deadline` is an optional hard cutoff (useful for script timeouts). If the deadline
/// is exceeded while waiting, `wait_until_condition` will immediately return the last
/// known state as unmatched.
pub async fn wait_until_condition<F, Fut, T, E>(
    inner: &Inner,
    timeout_ms: u64,
    poll_interval_ms: u64,
    deadline: Option<Instant>,
    mut condition: F,
) -> Result<(bool, Option<T>, u64), E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<(bool, Option<T>), E>>,
{
    let poll_interval_ms = poll_interval_ms.max(1);
    let start = Instant::now();
    let mut last_state = None;
    let runtime =
        Runtime::from_inner(inner).expect("runtime wait helpers require an attached runtime");

    loop {
        if let Some(dl) = deadline
            && Instant::now() >= dl
        {
            let elapsed_ms = start.elapsed().as_millis() as u64;
            return Ok((false, last_state, elapsed_ms));
        }

        match condition().await {
            Ok((true, state)) => {
                let elapsed_ms = start.elapsed().as_millis() as u64;
                return Ok((true, state, elapsed_ms));
            }
            Ok((false, state)) => {
                last_state = state;
            }
            Err(e) => return Err(e),
        }

        let elapsed_ms = start.elapsed().as_millis() as u64;
        if elapsed_ms >= timeout_ms {
            return Ok((false, last_state, elapsed_ms));
        }

        // Keep requesting repaints while also allowing plain state changes to
        // satisfy the wait, even when no frames are being produced.
        let poll_deadline = Instant::now()
            .checked_add(Duration::from_millis(poll_interval_ms))
            .unwrap_or_else(Instant::now);
        while Instant::now() < poll_deadline {
            let elapsed_ms = start.elapsed().as_millis() as u64;
            if elapsed_ms >= timeout_ms {
                return Ok((false, last_state, elapsed_ms));
            }
            if let Some(dl) = deadline
                && Instant::now() >= dl
            {
                return Ok((false, last_state, elapsed_ms));
            }

            let notified = runtime.frame_notify().notified();
            inner.request_repaint();

            let remaining_poll = poll_deadline.saturating_duration_since(Instant::now());
            let remaining_timeout = Duration::from_millis(timeout_ms.saturating_sub(elapsed_ms));
            let step = Duration::from_millis(FRAME_DURATION_MS)
                .min(remaining_poll)
                .min(remaining_timeout)
                .max(Duration::from_millis(1));
            tokio::select! {
                _ = notified => {}
                _ = sleep(step) => {}
            }
        }
    }
}

pub fn validate_widget_value(
    widget: &WidgetRegistryEntry,
    value: &WidgetValue,
) -> Result<(), ToolError> {
    let matches_role = match widget.role {
        WidgetRole::TextEdit => matches!(value, WidgetValue::Text(_)),
        WidgetRole::Checkbox
        | WidgetRole::Toggle
        | WidgetRole::Selectable
        | WidgetRole::Radio
        | WidgetRole::CollapsingHeader => {
            matches!(value, WidgetValue::Bool(_))
        }
        WidgetRole::Slider => match value {
            WidgetValue::Float(v) => slider_accepts(widget, *v),
            WidgetValue::Int(v) => slider_accepts(widget, *v as f64),
            _ => false,
        },
        WidgetRole::ComboBox => {
            matches!(value, WidgetValue::Int(index) if combo_box_accepts(widget, *index))
        }
        WidgetRole::DragValue => match (widget.value.as_ref(), value) {
            (Some(WidgetValue::Int(_)), WidgetValue::Int(v)) => {
                drag_value_accepts(widget, *v as f64)
            }
            (Some(WidgetValue::Float(_)), WidgetValue::Float(v)) => drag_value_accepts(widget, *v),
            (Some(_), _) => false,
            (None, WidgetValue::Int(v)) => drag_value_accepts(widget, *v as f64),
            (None, WidgetValue::Float(v)) => drag_value_accepts(widget, *v),
            (None, _) => false,
        },
        WidgetRole::ColorPicker => {
            matches!(value, WidgetValue::Text(text) if parse_color_hex(text).is_some())
        }
        _ => false,
    };
    if !matches_role {
        return Err(ToolError::new(
            ErrorCode::InvalidRef,
            "Value type does not match widget role",
        ));
    }
    Ok(())
}

fn slider_accepts(widget: &WidgetRegistryEntry, value: f64) -> bool {
    widget
        .role_state
        .as_ref()
        .and_then(RoleState::range)
        .is_none_or(|range| range.contains(value))
}

fn drag_value_accepts(widget: &WidgetRegistryEntry, value: f64) -> bool {
    widget
        .role_state
        .as_ref()
        .and_then(RoleState::range)
        .is_none_or(|range| range.contains(value))
}

fn combo_box_accepts(widget: &WidgetRegistryEntry, index: i64) -> bool {
    if index < 0 {
        return false;
    }
    widget
        .role_state
        .as_ref()
        .and_then(RoleState::options)
        .map(|options| index < options.len() as i64)
        .unwrap_or(true)
}

pub fn viewport_rect(inner: &Inner, viewport_id: egui::ViewportId) -> Option<Rect> {
    let snapshot = viewport_snapshot_for(inner, viewport_id)?;
    Some(Rect {
        min: Pos2 { x: 0.0, y: 0.0 },
        max: Pos2 {
            x: snapshot.inner_size.x,
            y: snapshot.inner_size.y,
        },
    })
}

pub fn viewport_snapshot_for(
    inner: &Inner,
    viewport_id: egui::ViewportId,
) -> Option<ViewportSnapshot> {
    let viewports = inner.viewports.viewports_snapshot();
    let id_str = viewport_id_to_string(viewport_id);
    viewports.into_iter().find(|v| v.viewport_id == id_str)
}

pub fn viewport_snapshot_json(snapshot: &ViewportSnapshot) -> Value {
    json!({
        "title": snapshot.title,
        "outer_pos": Value::Null,
        "outer_size": snapshot.outer_size,
        "inner_size": snapshot.inner_size,
        "focused": snapshot.focused,
        "minimized": snapshot.minimized,
        "maximized": snapshot.maximized,
        "fullscreen": snapshot.fullscreen,
    })
}

pub fn wait_timeout_details(
    kind: &str,
    elapsed_ms: u64,
    widget: Option<&WidgetRegistryEntry>,
    viewport: Option<&ViewportSnapshot>,
    start_frame: Option<u64>,
    end_frame: Option<u64>,
) -> Value {
    json!({
        "kind": kind,
        "elapsed_ms": elapsed_ms,
        "widget": widget.map(WidgetState::from),
        "viewport": viewport.map(viewport_snapshot_json),
        "start_frame": start_frame,
        "end_frame": end_frame,
    })
}

pub fn apply_overlay_debug_options(
    options: &mut OverlayDebugOptions,
    input: OverlayDebugOptionsInput,
) -> Result<(), ToolError> {
    if let Some(show_labels) = input.show_labels {
        options.show_labels = show_labels;
    }
    if let Some(show_sizes) = input.show_sizes {
        options.show_sizes = show_sizes;
    }
    if let Some(label_font_size) = input.label_font_size {
        options.label_font_size = label_font_size;
    }
    if let Some(color) = input.bounds_color {
        options.bounds_color = parse_color(&color)
            .ok_or_else(|| ToolError::new(ErrorCode::InvalidRef, "Invalid bounds_color"))?;
    }
    if let Some(color) = input.clip_color {
        options.clip_color = parse_color(&color)
            .ok_or_else(|| ToolError::new(ErrorCode::InvalidRef, "Invalid clip_color"))?;
    }
    if let Some(color) = input.overlap_color {
        options.overlap_color = parse_color(&color)
            .ok_or_else(|| ToolError::new(ErrorCode::InvalidRef, "Invalid overlap_color"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{RoleState, WidgetRange};

    fn widget(role: WidgetRole, value: Option<WidgetValue>) -> WidgetRegistryEntry {
        WidgetRegistryEntry {
            id: "widget".to_string(),
            explicit_id: true,
            native_id: 1,
            viewport_id: "root".to_string(),
            layer_id: "layer".to_string(),
            rect: Rect {
                min: Pos2 { x: 0.0, y: 0.0 },
                max: Pos2 { x: 10.0, y: 10.0 },
            },
            interact_rect: Rect {
                min: Pos2 { x: 0.0, y: 0.0 },
                max: Pos2 { x: 10.0, y: 10.0 },
            },
            role,
            label: None,
            value,
            layout: None,
            role_state: None,
            parent_id: None,
            enabled: true,
            visible: true,
            focused: false,
        }
    }

    #[test]
    fn resolve_key_name_single_char() {
        assert_eq!(resolve_key_name("a"), Some(egui::Key::A));
        assert_eq!(resolve_key_name("A"), Some(egui::Key::A));
        assert_eq!(resolve_key_name("-"), Some(egui::Key::Minus));
        assert_eq!(resolve_key_name("+"), Some(egui::Key::Plus));
        assert_eq!(resolve_key_name("0"), Some(egui::Key::Num0));
    }

    #[test]
    fn resolve_key_name_case_insensitive() {
        assert_eq!(resolve_key_name("enter"), Some(egui::Key::Enter));
        assert_eq!(resolve_key_name("Enter"), Some(egui::Key::Enter));
        assert_eq!(resolve_key_name("ENTER"), Some(egui::Key::Enter));
        assert_eq!(resolve_key_name("arrowup"), Some(egui::Key::ArrowUp));
        assert_eq!(resolve_key_name("ArrowUp"), Some(egui::Key::ArrowUp));
        assert_eq!(resolve_key_name("ARROWUP"), Some(egui::Key::ArrowUp));
        assert_eq!(resolve_key_name("f5"), Some(egui::Key::F5));
        assert_eq!(resolve_key_name("F5"), Some(egui::Key::F5));
        assert_eq!(resolve_key_name("escape"), Some(egui::Key::Escape));
        assert_eq!(resolve_key_name("esc"), Some(egui::Key::Escape));
        assert_eq!(resolve_key_name("tab"), Some(egui::Key::Tab));
        assert_eq!(resolve_key_name("pagedown"), Some(egui::Key::PageDown));
    }

    #[test]
    fn resolve_key_name_unknown() {
        assert_eq!(resolve_key_name("foobar"), None);
        assert_eq!(resolve_key_name(""), None);
    }

    #[test]
    fn parse_combo_simple_key() {
        let (key, mods, name) = parse_key_combo("enter").unwrap();
        assert_eq!(key, egui::Key::Enter);
        assert_eq!(name, "enter");
        assert!(!mods.ctrl && !mods.shift && !mods.alt && !mods.command);
    }

    #[test]
    fn parse_combo_with_modifiers() {
        let (key, mods, name) = parse_key_combo("ctrl-a").unwrap();
        assert_eq!(key, egui::Key::A);
        assert_eq!(name, "a");
        assert!(mods.ctrl);
        assert!(!mods.shift && !mods.alt && !mods.command);

        let (key, mods, _) = parse_key_combo("ctrl-shift-z").unwrap();
        assert_eq!(key, egui::Key::Z);
        assert!(mods.ctrl && mods.shift);

        let (key, mods, _) = parse_key_combo("cmd-s").unwrap();
        assert_eq!(key, egui::Key::S);
        assert!(mods.command);

        let (key, mods, _) = parse_key_combo("alt-f4").unwrap();
        assert_eq!(key, egui::Key::F4);
        assert!(mods.alt);
    }

    #[test]
    fn parse_combo_case_insensitive_modifiers() {
        let (key, mods, _) = parse_key_combo("CTRL-A").unwrap();
        assert_eq!(key, egui::Key::A);
        assert!(mods.ctrl);

        let (key, mods, _) = parse_key_combo("Shift-Tab").unwrap();
        assert_eq!(key, egui::Key::Tab);
        assert!(mods.shift);

        let (_, mods, _) = parse_key_combo("COMMAND-s").unwrap();
        assert!(mods.command);
    }

    #[test]
    fn parse_combo_minus_key() {
        let (key, mods, name) = parse_key_combo("-").unwrap();
        assert_eq!(key, egui::Key::Minus);
        assert_eq!(name, "-");
        assert!(!mods.ctrl);

        let (key, mods, name) = parse_key_combo("ctrl--").unwrap();
        assert_eq!(key, egui::Key::Minus);
        assert_eq!(name, "-");
        assert!(mods.ctrl);
    }

    #[test]
    fn parse_combo_plus_key() {
        let (key, mods, _) = parse_key_combo("ctrl-+").unwrap();
        assert_eq!(key, egui::Key::Plus);
        assert!(mods.ctrl);
    }

    #[test]
    fn parse_combo_errors() {
        assert!(parse_key_combo("").is_err());
        assert!(parse_key_combo("foobar").is_err());
        assert!(parse_key_combo("ctrl-foobar").is_err());
        assert!(parse_key_combo("notamod-a").is_err());
    }

    #[test]
    fn validate_widget_value_rejects_out_of_range_slider() {
        let mut slider = widget(WidgetRole::Slider, Some(WidgetValue::Float(5.0)));
        slider.role_state = Some(RoleState::Slider {
            range: WidgetRange {
                min: 0.0,
                max: 10.0,
            },
        });

        assert!(validate_widget_value(&slider, &WidgetValue::Float(8.0)).is_ok());
        assert!(validate_widget_value(&slider, &WidgetValue::Int(8)).is_ok());
        assert!(validate_widget_value(&slider, &WidgetValue::Float(12.0)).is_err());
        assert!(validate_widget_value(&slider, &WidgetValue::Int(12)).is_err());
    }

    #[test]
    fn validate_widget_value_rejects_out_of_range_combo_box_index() {
        let mut combo = widget(WidgetRole::ComboBox, Some(WidgetValue::Int(1)));
        combo.role_state = Some(RoleState::ComboBox {
            options: vec!["Alpha".to_string(), "Beta".to_string()],
        });

        assert!(validate_widget_value(&combo, &WidgetValue::Int(1)).is_ok());
        assert!(validate_widget_value(&combo, &WidgetValue::Int(2)).is_err());
        assert!(validate_widget_value(&combo, &WidgetValue::Int(-1)).is_err());
    }

    #[test]
    fn validate_widget_value_accepts_and_rejects_color_hex() {
        let color = widget(
            WidgetRole::ColorPicker,
            Some(WidgetValue::Text("#409CFFFF".to_string())),
        );

        assert!(validate_widget_value(&color, &WidgetValue::Text("#11223344".to_string())).is_ok());
        assert!(validate_widget_value(&color, &WidgetValue::Text("#1234".to_string())).is_err());
        assert!(validate_widget_value(&color, &WidgetValue::Text("11223344".to_string())).is_err());
    }
}
