use egui::PointerButton;

use crate::overlay::OverlayDebugMode;

#[derive(
    Debug, Clone, Copy, serde::Serialize, serde::Deserialize, schemars::JsonSchema, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum PointerButtonName {
    #[default]
    Primary,
    Secondary,
    Middle,
}

impl PointerButtonName {
    pub fn to_pointer_button(self) -> Option<PointerButton> {
        match self {
            Self::Primary => Some(PointerButton::Primary),
            Self::Secondary => Some(PointerButton::Secondary),
            Self::Middle => Some(PointerButton::Middle),
        }
    }
}

/// Closed set of issue kinds returned by layout checking.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum LayoutIssueKind {
    Overlap,
    Clipping,
    Overflow,
    ZeroSize,
    TextTruncation,
    Offscreen,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ScrollAlign {
    Top,
    Center,
    Bottom,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum OverlayDebugModeName {
    Bounds,
    Margins,
    Clipping,
    Overlaps,
    Focus,
    Layers,
    Containers,
}

impl From<OverlayDebugModeName> for OverlayDebugMode {
    fn from(mode: OverlayDebugModeName) -> Self {
        match mode {
            OverlayDebugModeName::Bounds => Self::Bounds,
            OverlayDebugModeName::Margins => Self::Margins,
            OverlayDebugModeName::Clipping => Self::Clipping,
            OverlayDebugModeName::Overlaps => Self::Overlaps,
            OverlayDebugModeName::Focus => Self::Focus,
            OverlayDebugModeName::Layers => Self::Layers,
            OverlayDebugModeName::Containers => Self::Containers,
        }
    }
}

impl From<OverlayDebugMode> for OverlayDebugModeName {
    fn from(mode: OverlayDebugMode) -> Self {
        match mode {
            OverlayDebugMode::Bounds => Self::Bounds,
            OverlayDebugMode::Margins => Self::Margins,
            OverlayDebugMode::Clipping => Self::Clipping,
            OverlayDebugMode::Overlaps => Self::Overlaps,
            OverlayDebugMode::Focus => Self::Focus,
            OverlayDebugMode::Layers => Self::Layers,
            OverlayDebugMode::Containers => Self::Containers,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema, Default)]
pub struct OverlayDebugOptionsInput {
    pub show_labels: Option<bool>,
    pub show_sizes: Option<bool>,
    pub label_font_size: Option<f32>,
    pub bounds_color: Option<String>,
    pub clip_color: Option<String>,
    pub overlap_color: Option<String>,
}
