use tmcp::tool_result;

use super::types::LayoutIssueKind;
use crate::types::{Rect, Vec2, WidgetRegistryEntry};

#[derive(Debug, Clone)]
#[tool_result]
pub struct WidgetGetResult {
    pub widget: WidgetRegistryEntry,
}

#[derive(Debug, Clone)]
#[tool_result]
pub struct OverlayHighlightResult {
    pub rect: Rect,
}

/// A single semantic layout problem detected in a viewport or widget subtree.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct LayoutIssue {
    pub kind: LayoutIssueKind,
    pub widgets: Vec<String>,
    pub message: String,
    pub rect: Option<Rect>,
}

#[derive(Debug, Clone)]
#[tool_result]
pub struct TextMeasure {
    pub text: String,
    pub visible_text: String,
    pub desired_size: Vec2,
    pub actual_size: Vec2,
    pub line_height: f32,
    pub lines: Vec<TextMeasureLine>,
    pub ellipsis: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct TextMeasureLine {
    pub text: String,
    pub width: f32,
}

#[derive(Debug, Clone)]
#[tool_result]
pub struct WidgetAtPointResult {
    pub widgets: Vec<WidgetRegistryEntry>,
}
