//! Overlay rendering helpers.
#![allow(missing_docs)]

use std::{collections::HashMap, sync::Mutex};

use egui::{Color32, Context, Rect as EguiRect};

use crate::{
    registry::{lock, viewport_id_to_string},
    tree::collect_subtree,
    types::{Pos2, Rect, Vec2, WidgetRef},
    viewports::ViewportState,
    widget_registry::WidgetRegistry,
};

#[derive(Debug, Clone)]
pub struct OverlayEntry {
    pub rect: EguiRect,
    pub color: Color32,
    pub stroke_width: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayDebugMode {
    Bounds,
    Margins,
    Clipping,
    Overlaps,
    Focus,
    Layers,
    Containers,
}

#[derive(Debug, Clone)]
pub struct OverlayDebugOptions {
    pub show_labels: bool,
    pub show_sizes: bool,
    pub label_font_size: f32,
    pub bounds_color: Color32,
    pub clip_color: Color32,
    pub overlap_color: Color32,
}

impl Default for OverlayDebugOptions {
    fn default() -> Self {
        Self {
            show_labels: true,
            show_sizes: true,
            label_font_size: 10.0,
            bounds_color: Color32::from_rgba_premultiplied(0x00, 0xff, 0x00, 0x66),
            clip_color: Color32::from_rgba_premultiplied(0xff, 0x00, 0x00, 0x66),
            overlap_color: Color32::from_rgba_premultiplied(0xff, 0xff, 0x00, 0x66),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OverlayDebugConfig {
    pub enabled: bool,
    pub mode: OverlayDebugMode,
    pub scope: Option<WidgetRef>,
    pub options: OverlayDebugOptions,
}

impl Default for OverlayDebugConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: OverlayDebugMode::Bounds,
            scope: None,
            options: OverlayDebugOptions::default(),
        }
    }
}

pub fn parse_color(value: &str) -> Option<Color32> {
    let value = value.trim();
    let hex = value.strip_prefix('#').unwrap_or(value);
    let bytes = match hex.len() {
        6 => u32::from_str_radix(hex, 16).ok().map(|v| v << 8 | 0xff)?,
        8 => u32::from_str_radix(hex, 16).ok()?,
        _ => return None,
    };
    let r = ((bytes >> 24) & 0xff) as u8;
    let g = ((bytes >> 16) & 0xff) as u8;
    let b = ((bytes >> 8) & 0xff) as u8;
    let a = (bytes & 0xff) as u8;
    Some(Color32::from_rgba_premultiplied(r, g, b, a))
}

pub struct OverlayManager {
    overlays: Mutex<HashMap<String, OverlayEntry>>,
    overlay_debug: Mutex<OverlayDebugConfig>,
}

impl Default for OverlayManager {
    fn default() -> Self {
        Self::new()
    }
}

impl OverlayManager {
    pub fn new() -> Self {
        Self {
            overlays: Mutex::new(HashMap::new()),
            overlay_debug: Mutex::new(OverlayDebugConfig::default()),
        }
    }

    pub fn overlay_debug_config(&self) -> OverlayDebugConfig {
        lock(&self.overlay_debug, "overlay debug lock").clone()
    }

    pub fn set_overlay_debug_config(&self, config: OverlayDebugConfig) {
        let mut stored = lock(&self.overlay_debug, "overlay debug lock");
        *stored = config;
    }

    pub fn set_overlay(&self, key: String, overlay: OverlayEntry) {
        let mut overlays = lock(&self.overlays, "overlay lock");
        overlays.insert(key, overlay);
    }

    pub fn remove_overlay(&self, key: &str) {
        let mut overlays = lock(&self.overlays, "overlay lock");
        overlays.remove(key);
    }

    pub fn clear_overlays(&self) {
        lock(&self.overlays, "overlay lock").clear();
    }

    pub fn clear_transient_state(&self) {
        self.clear_overlays();
        let mut debug = lock(&self.overlay_debug, "overlay debug lock");
        *debug = OverlayDebugConfig::default();
    }

    pub fn paint_overlays(
        &self,
        ctx: &Context,
        widgets: &WidgetRegistry,
        viewports: &ViewportState,
    ) {
        self.paint_highlight_overlays(ctx);
        self.paint_debug_overlay(ctx, widgets, viewports);
    }

    fn paint_highlight_overlays(&self, ctx: &Context) {
        let overlays = lock(&self.overlays, "overlay lock");
        if overlays.is_empty() {
            return;
        }
        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("eguidev.overlay"));
        let painter = ctx.layer_painter(layer);
        for overlay in overlays.values() {
            painter.rect_stroke(
                overlay.rect,
                egui::CornerRadius::ZERO,
                egui::Stroke::new(overlay.stroke_width, overlay.color),
                egui::StrokeKind::Inside,
            );
        }
    }

    fn paint_debug_overlay(
        &self,
        ctx: &Context,
        widget_registry: &WidgetRegistry,
        viewport_state: &ViewportState,
    ) {
        let config = self.overlay_debug_config();
        if !config.enabled {
            return;
        }
        let viewport_id = ctx.viewport_id();
        let viewport_id_str = viewport_id_to_string(viewport_id);
        if let Some(scope) = config.scope.as_ref()
            && let Some(scope_viewport) = scope.viewport_id.as_deref()
            && scope_viewport != viewport_id_str
        {
            return;
        }
        let mut widget_list = widget_registry.widget_list(viewport_id);
        if let Some(scope) = config.scope.as_ref()
            && let Ok(root) =
                widget_registry.resolve_widget(viewport_state, Some(&viewport_id_str), scope)
        {
            widget_list = collect_subtree(&widget_list, &root);
        }
        if widget_list.is_empty() {
            return;
        }
        let layer = egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("eguidev.overlay.debug"),
        );
        let painter = ctx.layer_painter(layer);
        let options = config.options.clone();
        let mode = config.mode;
        let font_id = egui::FontId::proportional(options.label_font_size);
        let mut overlap_rects = Vec::new();

        for widget in &widget_list {
            let rect = widget.rect.into();
            let draw_bounds = match mode {
                OverlayDebugMode::Focus => widget.focused,
                OverlayDebugMode::Containers => widget_list
                    .iter()
                    .any(|entry| entry.parent_id.as_deref() == Some(&widget.id)),
                _ => true,
            };
            if mode == OverlayDebugMode::Margins
                && let Some(layout) = widget.layout.as_ref()
            {
                painter.rect_stroke(
                    layout.available_rect.into(),
                    egui::CornerRadius::ZERO,
                    egui::Stroke::new(1.0, options.clip_color),
                    egui::StrokeKind::Inside,
                );
            }
            if draw_bounds {
                painter.rect_stroke(
                    rect,
                    egui::CornerRadius::ZERO,
                    egui::Stroke::new(1.0, options.bounds_color),
                    egui::StrokeKind::Inside,
                );
            }
            if mode == OverlayDebugMode::Clipping
                && let Some(layout) = widget.layout.as_ref()
                && let Some(clipped_rect) = rect_intersection(widget.rect, layout.clip_rect)
            {
                painter.rect_stroke(
                    clipped_rect.into(),
                    egui::CornerRadius::ZERO,
                    egui::Stroke::new(1.0, options.clip_color),
                    egui::StrokeKind::Inside,
                );
            }
            if mode == OverlayDebugMode::Overlaps {
                for other in &widget_list {
                    if widget.id == other.id {
                        continue;
                    }
                    if widget.parent_id != other.parent_id {
                        continue;
                    }
                    if let Some(intersection) = rect_intersection(widget.rect, other.rect) {
                        overlap_rects.push(intersection);
                    }
                }
            }
            if options.show_labels {
                let mut label = widget.id.clone();
                if mode == OverlayDebugMode::Layers {
                    label = format!("{label} [{}]", widget.layer_id);
                }
                if options.show_sizes {
                    let size = rect_size(widget.rect);
                    label = format!("{label} ({:.0}x{:.0})", size.x, size.y);
                }
                painter.text(
                    rect.min,
                    egui::Align2::LEFT_TOP,
                    label,
                    font_id.clone(),
                    egui::Color32::WHITE,
                );
            }
        }

        if mode == OverlayDebugMode::Overlaps {
            for rect in overlap_rects {
                painter.rect_stroke(
                    rect.into(),
                    egui::CornerRadius::ZERO,
                    egui::Stroke::new(1.0, options.overlap_color),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }
}

pub fn rect_intersection(first: Rect, second: Rect) -> Option<Rect> {
    let min_x = first.min.x.max(second.min.x);
    let min_y = first.min.y.max(second.min.y);
    let max_x = first.max.x.min(second.max.x);
    let max_y = first.max.y.min(second.max.y);
    if max_x <= min_x || max_y <= min_y {
        return None;
    }
    Some(Rect {
        min: Pos2 { x: min_x, y: min_y },
        max: Pos2 { x: max_x, y: max_y },
    })
}

pub fn rect_size(rect: Rect) -> Vec2 {
    Vec2 {
        x: rect.max.x - rect.min.x,
        y: rect.max.y - rect.min.y,
    }
}
