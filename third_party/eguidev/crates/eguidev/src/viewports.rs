//! Viewport and input snapshot state.
#![allow(missing_docs)]

use std::{collections::HashMap, sync::Mutex};

use egui::{Context, Vec2 as EguiVec2};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    error::{ErrorCode, ToolError},
    registry::{lock, viewport_id_to_string},
    types::{Pos2, Vec2},
};

#[derive(Debug, Clone)]
pub struct InputSnapshot {
    pub pixels_per_point: f32,
    pub pointer_pos: Option<Pos2>,
}

#[derive(Debug, Clone, Copy)]
pub struct CaptureSnapshot {
    pub fixture_epoch: u64,
    pub frame_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ViewportSnapshot {
    pub viewport_id: String,
    pub inner_size: Vec2,
    pub outer_size: Option<Vec2>,
    pub pixels_per_point: f32,
    pub focused: bool,
    pub title: Option<String>,
    pub parent_viewport_id: Option<String>,
    pub minimized: Option<bool>,
    pub maximized: Option<bool>,
    pub fullscreen: Option<bool>,
}

pub struct ViewportState {
    viewports_snapshot: Mutex<Vec<ViewportSnapshot>>,
    viewport_lookup: Mutex<HashMap<String, egui::ViewportId>>,
    input_snapshot: Mutex<HashMap<egui::ViewportId, InputSnapshot>>,
    capture_snapshot: Mutex<HashMap<egui::ViewportId, CaptureSnapshot>>,
}

impl Default for ViewportState {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewportState {
    pub fn new() -> Self {
        Self {
            viewports_snapshot: Mutex::new(Vec::new()),
            viewport_lookup: Mutex::new(HashMap::new()),
            input_snapshot: Mutex::new(HashMap::new()),
            capture_snapshot: Mutex::new(HashMap::new()),
        }
    }

    pub fn update_viewports(&self, ctx: &Context) {
        let (viewports, pixels_per_point, focused) =
            ctx.input(|i| (i.raw.viewports.clone(), i.pixels_per_point(), i.focused));
        let mut stored = lock(&self.viewports_snapshot, "viewports snapshot lock");
        let mut snapshots = stored
            .iter()
            .cloned()
            .map(|snapshot| (snapshot.viewport_id.clone(), snapshot))
            .collect::<HashMap<_, _>>();
        let mut lookup = lock(&self.viewport_lookup, "viewport lookup lock");
        for (viewport_id, info) in viewports {
            let viewport_id_str = viewport_id_to_string(viewport_id);
            let inner_size = info
                .inner_rect
                .map(|rect| rect.size())
                .unwrap_or_else(|| EguiVec2::ZERO);
            let outer_size = info.outer_rect.map(|rect| Vec2::from(rect.size()));
            let ppp = info.native_pixels_per_point.unwrap_or(pixels_per_point);
            let focused = info.focused.unwrap_or(focused);
            lookup.insert(viewport_id_str.clone(), viewport_id);
            snapshots.insert(
                viewport_id_str.clone(),
                ViewportSnapshot {
                    viewport_id: viewport_id_str,
                    inner_size: Vec2::from(inner_size),
                    outer_size,
                    pixels_per_point: ppp,
                    focused,
                    title: info.title.clone(),
                    parent_viewport_id: info.parent.map(viewport_id_to_string),
                    minimized: info.minimized,
                    maximized: info.maximized,
                    fullscreen: info.fullscreen,
                },
            );
        }
        let mut ordered = snapshots.into_values().collect::<Vec<_>>();
        ordered.sort_by(|left, right| left.viewport_id.cmp(&right.viewport_id));
        *stored = ordered;
    }

    pub fn remember_viewport_id(&self, viewport_id: egui::ViewportId) {
        lock(&self.viewport_lookup, "viewport lookup lock")
            .insert(viewport_id_to_string(viewport_id), viewport_id);
    }

    pub fn capture_input_snapshot(&self, ctx: &Context, fixture_epoch: u64, frame_count: u64) {
        let viewport_id = ctx.viewport_id();
        self.remember_viewport_id(viewport_id);
        let snapshot = ctx.input(|i| InputSnapshot {
            pixels_per_point: i.pixels_per_point(),
            pointer_pos: i.pointer.latest_pos().map(Pos2::from),
        });
        self.record_input_snapshot(viewport_id, snapshot, fixture_epoch, frame_count);
    }

    pub fn record_input_snapshot(
        &self,
        viewport_id: egui::ViewportId,
        snapshot: InputSnapshot,
        fixture_epoch: u64,
        frame_count: u64,
    ) {
        let mut map = lock(&self.input_snapshot, "input snapshot lock");
        map.insert(viewport_id, snapshot);
        let mut capture_map = lock(&self.capture_snapshot, "capture snapshot lock");
        capture_map.insert(
            viewport_id,
            CaptureSnapshot {
                fixture_epoch,
                frame_count,
            },
        );
    }

    pub fn viewports_snapshot(&self) -> Vec<ViewportSnapshot> {
        lock(&self.viewports_snapshot, "viewports snapshot lock").clone()
    }

    pub fn has_viewport_snapshot(&self, viewport_id: egui::ViewportId) -> bool {
        let id = viewport_id_to_string(viewport_id);
        lock(&self.viewports_snapshot, "viewports snapshot lock")
            .iter()
            .any(|snapshot| snapshot.viewport_id == id)
    }

    pub fn input_snapshot(&self, viewport_id: egui::ViewportId) -> Option<InputSnapshot> {
        lock(&self.input_snapshot, "input snapshot lock")
            .get(&viewport_id)
            .cloned()
    }

    pub fn capture_snapshot(&self, viewport_id: egui::ViewportId) -> Option<CaptureSnapshot> {
        lock(&self.capture_snapshot, "capture snapshot lock")
            .get(&viewport_id)
            .copied()
    }

    pub fn resolve_viewport_id(
        &self,
        viewport_id: Option<String>,
    ) -> Result<egui::ViewportId, ToolError> {
        match viewport_id {
            None => Ok(egui::ViewportId::ROOT),
            Some(value) if value == "root" => Ok(egui::ViewportId::ROOT),
            Some(value) => {
                let lookup = lock(&self.viewport_lookup, "viewport lookup lock");
                lookup.get(&value).copied().ok_or_else(|| {
                    ToolError::new(ErrorCode::InvalidRef, "Unknown viewport").with_details(json!({
                        "selectors": {
                            "viewport_id": value,
                        }
                    }))
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_viewports_retains_known_secondary_viewports() {
        let state = ViewportState::new();
        let ctx = Context::default();
        let secondary = egui::ViewportId::from_hash_of("secondary");

        let mut raw_input = egui::RawInput {
            viewport_id: egui::ViewportId::ROOT,
            ..Default::default()
        };
        raw_input
            .viewports
            .insert(egui::ViewportId::ROOT, Default::default());
        raw_input.viewports.insert(secondary, Default::default());
        drop(ctx.run_ui(raw_input, |_| {}));
        state.update_viewports(&ctx);

        let secondary_id = viewport_id_to_string(secondary);
        assert_eq!(
            state
                .resolve_viewport_id(Some(secondary_id.clone()))
                .expect("secondary viewport"),
            secondary
        );

        let mut root_only = egui::RawInput {
            viewport_id: egui::ViewportId::ROOT,
            ..Default::default()
        };
        root_only
            .viewports
            .insert(egui::ViewportId::ROOT, Default::default());
        drop(ctx.run_ui(root_only, |_| {}));
        state.update_viewports(&ctx);

        assert_eq!(
            state
                .resolve_viewport_id(Some(secondary_id))
                .expect("retained secondary viewport"),
            secondary
        );
    }
}
