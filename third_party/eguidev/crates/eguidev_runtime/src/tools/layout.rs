use egui::{Color32, TextStyle};

use crate::{
    overlay::{rect_intersection, rect_size},
    tools::{
        ErrorCode, ToolError,
        results::{LayoutIssue, TextMeasure, TextMeasureLine},
        types::LayoutIssueKind,
    },
    types::{Pos2, Rect, WidgetRegistryEntry, WidgetRole, WidgetValue},
};

pub fn widget_text(widget: &WidgetRegistryEntry) -> Option<String> {
    match widget.value.as_ref() {
        Some(WidgetValue::Text(text)) => Some(text.clone()),
        _ => widget.label.clone(),
    }
}

pub fn measure_text(
    ctx: &egui::Context,
    widget: &WidgetRegistryEntry,
) -> Result<TextMeasure, ToolError> {
    let text = widget_text(widget)
        .ok_or_else(|| ToolError::new(ErrorCode::InvalidRef, "Widget does not contain text"))?;
    let style = ctx.style();
    let font_id = TextStyle::Body.resolve(style.as_ref());
    let desired_galley =
        ctx.fonts_mut(|fonts| fonts.layout_no_wrap(text.clone(), font_id.clone(), Color32::WHITE));
    let desired_size = desired_galley.rect.size();
    let actual_size = rect_size(widget.rect);
    let wrap_width = actual_size.x.max(1.0);
    let wrapped_galley = ctx
        .fonts_mut(|fonts| fonts.layout(text.clone(), font_id.clone(), Color32::WHITE, wrap_width));
    let line_height = wrapped_galley
        .rows
        .first()
        .map(|row| row.size.y)
        .unwrap_or(0.0);
    let lines = wrapped_galley
        .rows
        .iter()
        .map(|row| TextMeasureLine {
            text: row.glyphs.iter().map(|glyph| glyph.chr).collect(),
            width: row.size.x,
        })
        .collect::<Vec<_>>();
    Ok(TextMeasure {
        text: text.clone(),
        visible_text: text,
        desired_size: desired_size.into(),
        actual_size,
        line_height,
        lines,
        ellipsis: false,
    })
}

pub fn check_text_truncation(
    ctx: &egui::Context,
    widgets: &[WidgetRegistryEntry],
) -> Result<Vec<LayoutIssue>, ToolError> {
    let mut issues = Vec::new();
    for widget in widgets {
        if widget_text(widget).is_none() {
            continue;
        }
        let measurement = measure_text(ctx, widget)?;
        if measurement.desired_size.x > measurement.actual_size.x && measurement.lines.len() <= 1 {
            issues.push(LayoutIssue {
                kind: LayoutIssueKind::TextTruncation,
                widgets: vec![widget.id.clone()],
                message: format!(
                    "Text truncated (needs {:.1}px, has {:.1}px)",
                    measurement.desired_size.x, measurement.actual_size.x
                ),
                rect: Some(widget.rect),
            });
        }
    }
    Ok(issues)
}

pub fn check_zero_size(widgets: &[WidgetRegistryEntry]) -> Vec<LayoutIssue> {
    widgets
        .iter()
        .filter_map(|widget| {
            let size = rect_size(widget.rect);
            if size.x <= 0.0 || size.y <= 0.0 {
                Some(LayoutIssue {
                    kind: LayoutIssueKind::ZeroSize,
                    widgets: vec![widget.id.clone()],
                    message: "Widget has zero size".to_string(),
                    rect: Some(widget.rect),
                })
            } else {
                None
            }
        })
        .collect()
}

pub fn check_clipping(
    widgets: &[WidgetRegistryEntry],
    viewport_rect: Option<Rect>,
) -> Vec<LayoutIssue> {
    widgets
        .iter()
        .filter_map(|widget| {
            let layout = widget.layout.as_ref()?;
            let clipped = layout.clipped || layout.visible_fraction < 1.0;
            if clipped && !has_nested_clip_region(widget, viewport_rect) {
                Some(LayoutIssue {
                    kind: LayoutIssueKind::Clipping,
                    widgets: vec![widget.id.clone()],
                    message: "Widget is clipped".to_string(),
                    rect: Some(widget.rect),
                })
            } else {
                None
            }
        })
        .collect()
}

pub fn check_overflow(
    widgets: &[WidgetRegistryEntry],
    viewport_rect: Option<Rect>,
) -> Vec<LayoutIssue> {
    widgets
        .iter()
        .filter_map(|widget| {
            let layout = widget.layout.as_ref()?;
            let starts_within_slot = point_in_rect(widget.rect.min, layout.available_rect);
            if layout.overflow
                && starts_within_slot
                && !has_nested_clip_region(widget, viewport_rect)
            {
                Some(LayoutIssue {
                    kind: LayoutIssueKind::Overflow,
                    widgets: vec![widget.id.clone()],
                    message: "Widget overflows its clip rect".to_string(),
                    rect: Some(widget.rect),
                })
            } else {
                None
            }
        })
        .collect()
}

pub fn check_overlaps(widgets: &[WidgetRegistryEntry]) -> Vec<LayoutIssue> {
    let mut issues = Vec::new();
    for (i, first) in widgets.iter().enumerate() {
        for second in widgets.iter().skip(i + 1) {
            if !first.visible || !second.visible {
                continue;
            }
            if first.role == WidgetRole::ScrollArea || second.role == WidgetRole::ScrollArea {
                continue;
            }
            if let Some(overlap_rect) = rect_intersection(first.rect, second.rect) {
                issues.push(LayoutIssue {
                    kind: LayoutIssueKind::Overlap,
                    widgets: vec![first.id.clone(), second.id.clone()],
                    message: "Widgets overlap".to_string(),
                    rect: Some(overlap_rect),
                });
            }
        }
    }
    issues
}

pub fn check_offscreen(widgets: &[WidgetRegistryEntry], viewport_rect: Rect) -> Vec<LayoutIssue> {
    widgets
        .iter()
        .filter_map(|widget| {
            if has_nested_clip_region(widget, Some(viewport_rect)) {
                return None;
            }
            if rect_contains_rect(viewport_rect, widget.rect) {
                return None;
            }
            Some(LayoutIssue {
                kind: LayoutIssueKind::Offscreen,
                widgets: vec![widget.id.clone()],
                message: "Widget is outside the viewport".to_string(),
                rect: Some(widget.rect),
            })
        })
        .collect()
}

fn has_nested_clip_region(widget: &WidgetRegistryEntry, viewport_rect: Option<Rect>) -> bool {
    let Some(layout) = widget.layout.as_ref() else {
        return false;
    };
    let Some(viewport_rect) = viewport_rect else {
        return false;
    };
    !rect_approx_eq(layout.clip_rect, viewport_rect)
}

fn rect_approx_eq(left: Rect, right: Rect) -> bool {
    approx_eq(left.min.x, right.min.x)
        && approx_eq(left.min.y, right.min.y)
        && approx_eq(left.max.x, right.max.x)
        && approx_eq(left.max.y, right.max.y)
}

fn approx_eq(left: f32, right: f32) -> bool {
    (left - right).abs() <= 0.5
}

pub fn point_in_rect(pos: Pos2, rect: Rect) -> bool {
    pos.x >= rect.min.x && pos.x <= rect.max.x && pos.y >= rect.min.y && pos.y <= rect.max.y
}

pub fn rect_contains_rect(outer: Rect, inner: Rect) -> bool {
    inner.min.x >= outer.min.x
        && inner.min.y >= outer.min.y
        && inner.max.x <= outer.max.x
        && inner.max.y <= outer.max.y
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{WidgetLayout, WidgetRole};

    fn rect(min_x: f32, min_y: f32, max_x: f32, max_y: f32) -> Rect {
        Rect {
            min: Pos2 { x: min_x, y: min_y },
            max: Pos2 { x: max_x, y: max_y },
        }
    }

    fn entry(id: &str, role: WidgetRole, rect: Rect, visible: bool) -> WidgetRegistryEntry {
        WidgetRegistryEntry {
            id: id.to_string(),
            explicit_id: true,
            native_id: 1,
            viewport_id: "root".to_string(),
            layer_id: "layer".to_string(),
            rect,
            interact_rect: rect,
            role,
            label: None,
            value: None,
            layout: None,
            role_state: None,
            parent_id: Some("panel".to_string()),
            enabled: true,
            visible,
            focused: false,
        }
    }

    #[test]
    fn layout_checks_ignore_nested_scroll_clipping() {
        let viewport_rect = rect(0.0, 0.0, 100.0, 100.0);
        let scroll_rect = rect(0.0, 40.0, 100.0, 80.0);

        let mut row = entry(
            "row",
            WidgetRole::Label,
            rect(0.0, 70.0, 100.0, 120.0),
            false,
        );
        row.layout = Some(WidgetLayout {
            desired_size: egui::vec2(100.0, 50.0).into(),
            actual_size: egui::vec2(100.0, 50.0).into(),
            clip_rect: scroll_rect,
            clipped: true,
            overflow: true,
            available_rect: viewport_rect,
            visible_fraction: 0.2,
        });

        assert!(check_clipping(&[row.clone()], Some(viewport_rect)).is_empty());
        assert!(check_overflow(&[row.clone()], Some(viewport_rect)).is_empty());
        assert!(check_offscreen(&[row], viewport_rect).is_empty());
    }

    #[test]
    fn overlap_checks_ignore_scroll_areas_and_invisible_widgets() {
        let scroll = entry(
            "scroll",
            WidgetRole::ScrollArea,
            rect(0.0, 0.0, 100.0, 40.0),
            true,
        );
        let row = entry("row", WidgetRole::Label, rect(0.0, 0.0, 100.0, 20.0), true);
        let hidden = entry(
            "hidden",
            WidgetRole::Label,
            rect(0.0, 0.0, 100.0, 20.0),
            false,
        );

        assert!(check_overlaps(&[scroll, row, hidden]).is_empty());
    }

    #[test]
    fn overflow_checks_ignore_post_layout_available_rects() {
        let viewport_rect = rect(0.0, 0.0, 100.0, 100.0);
        let mut button = entry(
            "button",
            WidgetRole::Button,
            rect(0.0, 0.0, 40.0, 20.0),
            true,
        );
        button.layout = Some(WidgetLayout {
            desired_size: egui::vec2(40.0, 20.0).into(),
            actual_size: egui::vec2(40.0, 20.0).into(),
            clip_rect: viewport_rect,
            clipped: false,
            overflow: true,
            available_rect: rect(50.0, 0.0, 100.0, 100.0),
            visible_fraction: 1.0,
        });

        assert!(check_overflow(&[button], Some(viewport_rect)).is_empty());
    }
}
