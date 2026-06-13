//! Data types used by DevMCP tooling.

use std::{borrow::Cow, fmt};

use egui::{Rect as EguiRect, Vec2 as EguiVec2};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{
    Deserialize, Serialize,
    de::{self, Deserializer},
    ser::Serializer,
};

use crate::registry::viewport_id_to_string;

fn sanitize_f32(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

/// A logical point in egui coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Pos2 {
    /// X coordinate in points.
    pub x: f32,
    /// Y coordinate in points.
    pub y: f32,
}

impl From<egui::Pos2> for Pos2 {
    fn from(pos: egui::Pos2) -> Self {
        Self {
            x: sanitize_f32(pos.x),
            y: sanitize_f32(pos.y),
        }
    }
}

impl From<Pos2> for egui::Pos2 {
    fn from(pos: Pos2) -> Self {
        egui::pos2(pos.x, pos.y)
    }
}

/// A 2D vector in egui coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Vec2 {
    /// X component in points.
    pub x: f32,
    /// Y component in points.
    pub y: f32,
}

impl From<EguiVec2> for Vec2 {
    fn from(vec: EguiVec2) -> Self {
        Self {
            x: sanitize_f32(vec.x),
            y: sanitize_f32(vec.y),
        }
    }
}

impl From<Vec2> for EguiVec2 {
    fn from(vec: Vec2) -> Self {
        egui::vec2(vec.x, vec.y)
    }
}

/// Axis-aligned rectangle in egui coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Rect {
    /// Minimum point.
    pub min: Pos2,
    /// Maximum point.
    pub max: Pos2,
}

impl Rect {
    /// Return the center point of the rectangle in egui coordinates.
    pub fn center(self) -> Pos2 {
        Pos2 {
            x: (self.min.x + self.max.x) * 0.5,
            y: (self.min.y + self.max.y) * 0.5,
        }
    }
}

impl From<EguiRect> for Rect {
    fn from(rect: EguiRect) -> Self {
        Self {
            min: Pos2::from(rect.min),
            max: Pos2::from(rect.max),
        }
    }
}

impl From<Rect> for EguiRect {
    fn from(rect: Rect) -> Self {
        Self::from_min_max(rect.min.into(), rect.max.into())
    }
}

/// Keyboard modifier state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct Modifiers {
    /// Ctrl key pressed.
    pub ctrl: bool,
    /// Shift key pressed.
    pub shift: bool,
    /// Alt key pressed.
    pub alt: bool,
    /// Command key pressed.
    pub command: bool,
}

impl From<egui::Modifiers> for Modifiers {
    fn from(modifiers: egui::Modifiers) -> Self {
        Self {
            ctrl: modifiers.ctrl,
            shift: modifiers.shift,
            alt: modifiers.alt,
            command: modifiers.command,
        }
    }
}

/// Fixture metadata advertised by an app.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FixtureSpec {
    /// Fixture name.
    pub name: String,
    /// Fixture description.
    pub description: String,
    /// Declarative readiness anchors for the fixture baseline.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<Anchor>,
}

/// A single readiness anchor for a fixture baseline.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Anchor {
    /// Widget id to resolve from the registry.
    pub widget_id: String,
    /// Optional viewport selector (`root` or `vp:...`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport_id: Option<String>,
    /// Readiness condition to evaluate against the widget state.
    pub check: AnchorCheck,
}

/// Declarative readiness checks for fixture anchors.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub enum AnchorCheck {
    /// Widget exists and is visible.
    Visible,
    /// Widget label matches exactly.
    Label(String),
    /// Widget value matches.
    Value(WidgetValue),
    /// Scroll area is initialized and stable across captures.
    ScrollReady,
    /// Scroll area is initialized, stable, and near the requested offset.
    ScrollAt {
        /// Target scroll offset.
        offset: Vec2,
        /// Allowed absolute error per axis.
        tolerance: f32,
    },
}

impl FixtureSpec {
    /// Create a new fixture specification.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            anchors: Vec::new(),
        }
    }

    /// Add a visible-widget readiness anchor.
    pub fn anchor(self, widget_id: impl Into<String>) -> Self {
        self.push_anchor(widget_id.into(), None, AnchorCheck::Visible)
    }

    /// Add a visible-widget readiness anchor scoped to a viewport.
    pub fn anchor_in(self, widget_id: impl Into<String>, viewport: egui::ViewportId) -> Self {
        self.push_anchor(
            widget_id.into(),
            Some(viewport_id_to_string(viewport)),
            AnchorCheck::Visible,
        )
    }

    /// Add an exact-label readiness anchor.
    pub fn anchor_label(self, widget_id: impl Into<String>, text: impl Into<String>) -> Self {
        self.push_anchor(widget_id.into(), None, AnchorCheck::Label(text.into()))
    }

    /// Add an exact-label readiness anchor scoped to a viewport.
    pub fn anchor_label_in(
        self,
        widget_id: impl Into<String>,
        text: impl Into<String>,
        viewport: egui::ViewportId,
    ) -> Self {
        self.push_anchor(
            widget_id.into(),
            Some(viewport_id_to_string(viewport)),
            AnchorCheck::Label(text.into()),
        )
    }

    /// Add an exact-value readiness anchor.
    pub fn anchor_value(self, widget_id: impl Into<String>, value: WidgetValue) -> Self {
        self.push_anchor(widget_id.into(), None, AnchorCheck::Value(value))
    }

    /// Add an exact-value readiness anchor scoped to a viewport.
    pub fn anchor_value_in(
        self,
        widget_id: impl Into<String>,
        value: WidgetValue,
        viewport: egui::ViewportId,
    ) -> Self {
        self.push_anchor(
            widget_id.into(),
            Some(viewport_id_to_string(viewport)),
            AnchorCheck::Value(value),
        )
    }

    /// Add a scroll-readiness anchor.
    pub fn anchor_scroll(self, widget_id: impl Into<String>) -> Self {
        self.push_anchor(widget_id.into(), None, AnchorCheck::ScrollReady)
    }

    /// Add a scroll-readiness anchor scoped to a viewport.
    pub fn anchor_scroll_in(
        self,
        widget_id: impl Into<String>,
        viewport: egui::ViewportId,
    ) -> Self {
        self.push_anchor(
            widget_id.into(),
            Some(viewport_id_to_string(viewport)),
            AnchorCheck::ScrollReady,
        )
    }

    /// Add a scroll-position readiness anchor.
    pub fn anchor_scroll_at(
        self,
        widget_id: impl Into<String>,
        offset: impl Into<Vec2>,
        tolerance: f32,
    ) -> Self {
        self.push_anchor(
            widget_id.into(),
            None,
            AnchorCheck::ScrollAt {
                offset: offset.into(),
                tolerance,
            },
        )
    }

    /// Add a scroll-position readiness anchor scoped to a viewport.
    pub fn anchor_scroll_at_in(
        self,
        widget_id: impl Into<String>,
        offset: impl Into<Vec2>,
        tolerance: f32,
        viewport: egui::ViewportId,
    ) -> Self {
        self.push_anchor(
            widget_id.into(),
            Some(viewport_id_to_string(viewport)),
            AnchorCheck::ScrollAt {
                offset: offset.into(),
                tolerance,
            },
        )
    }

    /// Validate fixture metadata and readiness anchors.
    pub fn validate(&self, require_anchors: bool) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("fixture name must not be empty".to_string());
        }
        for (index, anchor) in self.anchors.iter().enumerate() {
            anchor
                .validate()
                .map_err(|error| format!("fixture {} anchor {}: {error}", self.name, index + 1))?;
        }
        if require_anchors && self.anchors.is_empty() {
            return Err(format!(
                "fixture {} must declare at least one readiness anchor",
                self.name
            ));
        }
        Ok(())
    }

    /// Return a human-readable summary of the readiness contract.
    pub fn describe_readiness(&self) -> String {
        if self.anchors.is_empty() {
            return "No readiness anchors declared.".to_string();
        }
        self.anchors
            .iter()
            .map(Anchor::describe)
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn push_anchor(
        mut self,
        widget_id: String,
        viewport_id: Option<String>,
        check: AnchorCheck,
    ) -> Self {
        self.anchors.push(Anchor {
            widget_id,
            viewport_id,
            check,
        });
        self
    }
}

impl Anchor {
    /// Return a human-readable description of the anchor.
    pub fn describe(&self) -> String {
        let target = match &self.viewport_id {
            Some(viewport_id) => format!("{} in {}", self.widget_id, viewport_id),
            None => self.widget_id.clone(),
        };
        format!("{target} {}", self.check)
    }

    /// Validate the anchor contents.
    pub fn validate(&self) -> Result<(), String> {
        if self.widget_id.trim().is_empty() {
            return Err("widget_id must not be empty".to_string());
        }
        if let Some(viewport_id) = &self.viewport_id
            && viewport_id.trim().is_empty()
        {
            return Err("viewport_id must not be empty when provided".to_string());
        }
        match &self.check {
            AnchorCheck::Label(text) if text.is_empty() => {
                Err("label anchors must not be empty".to_string())
            }
            AnchorCheck::ScrollAt { tolerance, .. }
                if !tolerance.is_finite() || *tolerance <= 0.0 =>
            {
                Err("scroll_at tolerance must be finite and greater than 0".to_string())
            }
            _ => Ok(()),
        }
    }
}

impl fmt::Display for AnchorCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Visible => f.write_str("visible"),
            Self::Label(text) => write!(f, "label == \"{text}\""),
            Self::Value(value) => match value {
                WidgetValue::Text(text) => write!(f, "value == \"{text}\""),
                _ => write!(f, "value == {}", value.to_text()),
            },
            Self::ScrollReady => f.write_str("scroll_ready"),
            Self::ScrollAt { offset, tolerance } => write!(
                f,
                "scroll_at ({:.1}, {:.1}) ± {:.2}",
                offset.x, offset.y, tolerance
            ),
        }
    }
}

impl From<Modifiers> for egui::Modifiers {
    fn from(modifiers: Modifiers) -> Self {
        Self {
            ctrl: modifiers.ctrl,
            shift: modifiers.shift,
            alt: modifiers.alt,
            command: modifiers.command,
            mac_cmd: modifiers.command,
        }
    }
}

/// Widget reference used in tool calls.
///
/// Matching rules:
/// - `id` is the canonical widget selector.
/// - `viewport_id` acts as an additional selector to narrow matches.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WidgetRef {
    /// Canonical widget id.
    ///
    /// If instrumentation provides an explicit id, eguidev uses it verbatim.
    /// Otherwise eguidev generates an opaque hex id that is best-effort stable
    /// within the current app session.
    #[serde(default)]
    pub id: Option<String>,
    /// Optional viewport selector (`root` or `vp:...`).
    #[serde(default)]
    pub viewport_id: Option<String>,
}

/// Widget role taxonomy for automation and scripting filters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum WidgetRole {
    Button,
    Link,
    Image,
    Label,
    TextEdit,
    Slider,
    Checkbox,
    ComboBox,
    Radio,
    DragValue,
    Toggle,
    Selectable,
    Separator,
    Spinner,
    ScrollArea,
    MenuButton,
    CollapsingHeader,
    Window,
    ProgressBar,
    ColorPicker,
    #[default]
    Unknown,
}

/// Captured widget value for stateful controls.
#[derive(Debug, Clone, PartialEq)]
pub enum WidgetValue {
    /// Boolean value from checkboxes/toggles.
    Bool(bool),
    /// Floating-point value from sliders/drag values.
    Float(f64),
    /// Integer value from drag values/combos.
    Int(i64),
    /// Text value from text edits.
    Text(String),
}

impl WidgetValue {
    /// String representation matching Luau `tostring()` semantics.
    pub fn to_text(&self) -> String {
        match self {
            Self::Bool(v) => v.to_string(),
            Self::Float(v) => v.to_string(),
            Self::Int(v) => v.to_string(),
            Self::Text(v) => v.clone(),
        }
    }
}

impl<'de> Deserialize<'de> for WidgetValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        widget_value_from_json(value).map_err(de::Error::custom)
    }
}

impl Serialize for WidgetValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Float(value) => serializer.serialize_f64(*value),
            Self::Int(value) => serializer.serialize_i64(*value),
            Self::Text(value) => serializer.serialize_str(value),
        }
    }
}

#[doc(hidden)]
impl JsonSchema for WidgetRole {
    fn schema_name() -> Cow<'static, str> {
        "WidgetRole".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        schemars::json_schema!({
            "type": "string",
            "enum": [
                "button",
                "link",
                "image",
                "label",
                "text_edit",
                "slider",
                "checkbox",
                "combo_box",
                "radio",
                "drag_value",
                "toggle",
                "selectable",
                "separator",
                "spinner",
                "scroll_area",
                "menu_button",
                "collapsing_header",
                "window",
                "progress_bar",
                "color_picker",
                "unknown"
            ]
        })
    }
}

#[doc(hidden)]
impl JsonSchema for WidgetValue {
    fn schema_name() -> Cow<'static, str> {
        "WidgetValue".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        schemars::json_schema!({
            "oneOf": [
                { "type": "boolean" },
                { "type": "integer" },
                { "type": "number" },
                { "type": "string" }
            ]
        })
    }
}

fn widget_value_from_json(value: serde_json::Value) -> Result<WidgetValue, String> {
    match value {
        serde_json::Value::Object(map) => {
            if map.len() != 1 {
                return Err("WidgetValue must include exactly one field".to_string());
            }
            let (key, value) = map.into_iter().next().expect("map entry");
            match key.as_str() {
                "bool" => match value {
                    serde_json::Value::Bool(value) => Ok(WidgetValue::Bool(value)),
                    _ => Err("WidgetValue bool must be a boolean".to_string()),
                },
                "float" => match value {
                    serde_json::Value::Number(number) => number
                        .as_f64()
                        .map(WidgetValue::Float)
                        .ok_or_else(|| "WidgetValue float must be a number".to_string()),
                    _ => Err("WidgetValue float must be a number".to_string()),
                },
                "int" => match value {
                    serde_json::Value::Number(number) => number
                        .as_i64()
                        .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok()))
                        .map(WidgetValue::Int)
                        .ok_or_else(|| "WidgetValue int must be an integer".to_string()),
                    _ => Err("WidgetValue int must be an integer".to_string()),
                },
                "text" => match value {
                    serde_json::Value::String(value) => Ok(WidgetValue::Text(value)),
                    _ => Err("WidgetValue text must be a string".to_string()),
                },
                _ => Err("WidgetValue field must be one of bool, float, int, text".to_string()),
            }
        }
        serde_json::Value::Bool(value) => Ok(WidgetValue::Bool(value)),
        serde_json::Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                Ok(WidgetValue::Int(value))
            } else if let Some(value) = number.as_u64() {
                i64::try_from(value)
                    .map(WidgetValue::Int)
                    .map_err(|_| "WidgetValue int is out of range".to_string())
            } else if let Some(value) = number.as_f64() {
                Ok(WidgetValue::Float(value))
            } else {
                Err("WidgetValue number must be int or float".to_string())
            }
        }
        serde_json::Value::String(value) => {
            let trimmed = value.trim();
            if trimmed.starts_with('{')
                && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed)
            {
                return widget_value_from_json(parsed);
            }
            Ok(WidgetValue::Text(value))
        }
        _ => Err("WidgetValue must be bool, number, string, or tagged object".to_string()),
    }
}

/// Layout metadata captured for a widget when available.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WidgetLayout {
    /// Desired size of the widget before layout constraints.
    pub desired_size: Vec2,
    /// Actual size assigned to the widget.
    pub actual_size: Vec2,
    /// Clip rect for the widget at layout time.
    pub clip_rect: Rect,
    /// Whether any part of the widget is clipped.
    pub clipped: bool,
    /// Whether the widget extends beyond its allocated layout slot.
    pub overflow: bool,
    /// Available rect before the widget was laid out.
    pub available_rect: Rect,
    /// Visible fraction of the widget within the clip rect.
    pub visible_fraction: f32,
}

/// Scroll metadata captured for a scroll area.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ScrollAreaMeta {
    /// Current scroll offset.
    pub offset: Vec2,
    /// Viewport size available to the scroll contents.
    pub viewport_size: Vec2,
    /// Total content size within the scroll area.
    pub content_size: Vec2,
    /// Maximum reachable scroll offset after clamping.
    pub max_offset: Vec2,
}

impl ScrollAreaMeta {
    /// Build scroll metadata and derive the clamped maximum offset.
    pub fn new(offset: Vec2, viewport_size: Vec2, content_size: Vec2) -> Self {
        Self {
            offset,
            viewport_size,
            content_size,
            max_offset: Vec2 {
                x: (content_size.x - viewport_size.x).max(0.0),
                y: (content_size.y - viewport_size.y).max(0.0),
            },
        }
    }
}

/// Min/max bounds for a numeric widget.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WidgetRange {
    /// Minimum allowed value.
    pub min: f64,
    /// Maximum allowed value.
    pub max: f64,
}

impl WidgetRange {
    /// Check whether the range contains the provided value.
    pub fn contains(self, value: f64) -> bool {
        self.min <= value && value <= self.max
    }
}

/// Role-specific widget metadata kept on internal registry entries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoleState {
    /// Scroll area metadata.
    ScrollArea {
        /// Current scroll offset.
        offset: Vec2,
        /// Viewport size available to the scroll contents.
        viewport_size: Vec2,
        /// Total content size within the scroll area.
        content_size: Vec2,
    },
    /// Slider range metadata.
    Slider {
        /// Allowed numeric range.
        range: WidgetRange,
    },
    /// Drag value range metadata.
    DragValue {
        /// Allowed numeric range when constrained by the app.
        range: Option<WidgetRange>,
    },
    /// Combo box option labels.
    ComboBox {
        /// Available option labels.
        options: Vec<String>,
    },
    /// Selected/toggled button state.
    Button {
        /// Whether the button is in a selected state.
        selected: bool,
    },
    /// Checkbox third-state metadata.
    Checkbox {
        /// Whether the checkbox is visually indeterminate.
        indeterminate: bool,
    },
    /// Text edit configuration metadata.
    TextEdit {
        /// Whether the edit is multiline.
        multiline: bool,
        /// Whether the edit masks its input.
        password: bool,
    },
}

impl RoleState {
    /// Project scroll-area metadata into the flat scripting shape.
    pub fn scroll_state(&self) -> Option<ScrollAreaMeta> {
        match self {
            Self::ScrollArea {
                offset,
                viewport_size,
                content_size,
            } => Some(ScrollAreaMeta::new(*offset, *viewport_size, *content_size)),
            _ => None,
        }
    }

    /// Project numeric range metadata into the flat scripting shape.
    pub fn range(&self) -> Option<WidgetRange> {
        match self {
            Self::Slider { range } => Some(*range),
            Self::DragValue { range } => *range,
            _ => None,
        }
    }

    /// Return combo-box options when present.
    pub fn options(&self) -> Option<&[String]> {
        match self {
            Self::ComboBox { options } => Some(options),
            _ => None,
        }
    }

    /// Return button selected metadata when present.
    pub fn selected(&self) -> Option<bool> {
        match self {
            Self::Button { selected } => Some(*selected),
            _ => None,
        }
    }

    /// Return checkbox indeterminate metadata when present.
    pub fn indeterminate(&self) -> Option<bool> {
        match self {
            Self::Checkbox { indeterminate } => Some(*indeterminate),
            _ => None,
        }
    }

    /// Return text-edit metadata when present: `(multiline, password)`.
    pub fn text_edit(&self) -> Option<(bool, bool)> {
        match self {
            Self::TextEdit {
                multiline,
                password,
            } => Some((*multiline, *password)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RoleState, Vec2, WidgetValue};

    #[test]
    fn widget_value_deserializes_tagged_object() {
        let value: WidgetValue = serde_json::from_value(serde_json::json!({"bool": false}))
            .expect("deserialize tagged bool");
        assert_eq!(value, WidgetValue::Bool(false));
    }

    #[test]
    fn widget_value_deserializes_stringified_object() {
        let value: WidgetValue = serde_json::from_value(serde_json::json!("{\"bool\": false}"))
            .expect("deserialize stringified bool");
        assert_eq!(value, WidgetValue::Bool(false));
    }

    #[test]
    fn widget_value_deserializes_plain_text() {
        let value: WidgetValue =
            serde_json::from_value(serde_json::json!("hello")).expect("deserialize text");
        assert_eq!(value, WidgetValue::Text("hello".to_string()));
    }

    #[test]
    fn widget_value_serialization() {
        let v = WidgetValue::Bool(true);
        assert_eq!(serde_json::to_string(&v).unwrap(), "true");
    }

    #[test]
    fn scroll_area_meta_computes_max_offset() {
        let scroll = RoleState::ScrollArea {
            offset: Vec2 { x: 2.0, y: 3.0 },
            viewport_size: Vec2 { x: 100.0, y: 40.0 },
            content_size: Vec2 { x: 180.0, y: 150.0 },
        }
        .scroll_state()
        .expect("scroll metadata");

        assert_eq!(scroll.max_offset.x, 80.0);
        assert_eq!(scroll.max_offset.y, 110.0);
    }

    #[test]
    fn scroll_area_meta_clamps_negative_max_offset() {
        let scroll = RoleState::ScrollArea {
            offset: Vec2 { x: 0.0, y: 0.0 },
            viewport_size: Vec2 { x: 100.0, y: 40.0 },
            content_size: Vec2 { x: 80.0, y: 20.0 },
        }
        .scroll_state()
        .expect("scroll metadata");

        assert_eq!(scroll.max_offset.x, 0.0);
        assert_eq!(scroll.max_offset.y, 0.0);
    }
}

/// Widget registry entry captured per frame.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WidgetRegistryEntry {
    /// Canonical widget id.
    pub id: String,
    /// Whether the id was explicitly provided by instrumentation.
    #[serde(skip_serializing, skip_deserializing)]
    #[schemars(skip)]
    pub explicit_id: bool,
    /// Raw egui widget id used for low-level engine interactions.
    #[serde(skip_serializing, skip_deserializing)]
    #[schemars(skip)]
    pub native_id: u64,
    /// Viewport id string.
    pub viewport_id: String,
    /// Layer id rendered as a stable string (internal use only, e.g. debug overlay).
    #[serde(skip_serializing, skip_deserializing)]
    #[schemars(skip)]
    pub layer_id: String,
    /// Widget rect.
    pub rect: Rect,
    /// Widget interaction rect.
    pub interact_rect: Rect,
    /// Role taxonomy entry.
    pub role: WidgetRole,
    /// Optional label.
    pub label: Option<String>,
    /// Optional widget value for stateful controls.
    pub value: Option<WidgetValue>,
    /// Optional layout metadata.
    pub layout: Option<WidgetLayout>,
    /// Optional role-specific metadata encoded as a nested enum.
    #[serde(default)]
    pub role_state: Option<RoleState>,
    /// Optional parent id for container scoping.
    pub parent_id: Option<String>,
    /// Whether the widget is enabled.
    pub enabled: bool,
    /// Whether the widget is visible.
    pub visible: bool,
    /// Whether the widget reported egui focus in the captured frame (may lag keyboard focus).
    pub focused: bool,
}

/// Live widget snapshot exposed to scripting surfaces.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WidgetState {
    /// Widget rect.
    pub rect: Rect,
    /// Widget interaction rect.
    pub interact_rect: Rect,
    /// Role taxonomy entry.
    pub role: WidgetRole,
    /// Optional label.
    pub label: Option<String>,
    /// Optional widget value for stateful controls.
    pub value: Option<WidgetValue>,
    /// String representation of the widget value. Empty string when value is
    /// `None`. For `Bool` → `"true"`/`"false"`, `Float` → decimal, `Int` →
    /// decimal, `Text` → verbatim.
    pub value_text: String,
    /// Optional layout metadata.
    pub layout: Option<WidgetLayout>,
    /// Optional scroll metadata for scroll areas.
    #[serde(rename = "scroll_state")]
    pub scroll: Option<ScrollAreaMeta>,
    /// Optional numeric range for sliders and ranged drag values.
    pub range: Option<WidgetRange>,
    /// Optional option labels for combo boxes.
    pub options: Option<Vec<String>>,
    /// Optional selected/toggled state for selected-aware buttons.
    pub selected: Option<bool>,
    /// Optional third visual state for indeterminate checkboxes.
    pub indeterminate: Option<bool>,
    /// Optional multiline flag for text edits.
    pub multiline: Option<bool>,
    /// Optional password-masking flag for text edits.
    pub password: Option<bool>,
    /// Whether the widget is enabled.
    pub enabled: bool,
    /// Whether the widget is visible.
    pub visible: bool,
    /// Whether the widget reported egui focus in the captured frame (may lag keyboard focus).
    pub focused: bool,
}

impl From<&WidgetRegistryEntry> for WidgetState {
    fn from(entry: &WidgetRegistryEntry) -> Self {
        let value_text = entry
            .value
            .as_ref()
            .map(|v| v.to_text())
            .unwrap_or_default();
        let scroll = entry.role_state.as_ref().and_then(RoleState::scroll_state);
        let range = entry.role_state.as_ref().and_then(RoleState::range);
        let options = entry
            .role_state
            .as_ref()
            .and_then(RoleState::options)
            .map(<[String]>::to_vec);
        let selected = entry.role_state.as_ref().and_then(RoleState::selected);
        let indeterminate = entry.role_state.as_ref().and_then(RoleState::indeterminate);
        let (multiline, password) = entry
            .role_state
            .as_ref()
            .and_then(RoleState::text_edit)
            .map_or((None, None), |(multiline, password)| {
                (Some(multiline), Some(password))
            });
        Self {
            rect: entry.rect,
            interact_rect: entry.interact_rect,
            role: entry.role.clone(),
            label: entry.label.clone(),
            value: entry.value.clone(),
            value_text,
            layout: entry.layout.clone(),
            scroll,
            range,
            options,
            selected,
            indeterminate,
            multiline,
            password,
            enabled: entry.enabled,
            visible: entry.visible,
            focused: entry.focused,
        }
    }
}
