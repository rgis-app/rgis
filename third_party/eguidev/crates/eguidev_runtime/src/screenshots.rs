//! Screenshot state tracking and management.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::{
    registry::{lock, viewport_id_to_string},
    types::Rect,
};

#[derive(Debug, Clone)]
pub struct ScreenshotState {
    pub kind: ScreenshotKind,
    image: Option<Arc<egui::ColorImage>>,
    notify: Arc<Notify>,
}

impl ScreenshotState {
    pub fn pending(kind: ScreenshotKind) -> Self {
        Self {
            kind,
            image: None,
            notify: Arc::new(Notify::new()),
        }
    }

    pub fn mark_ready(&mut self, image: Arc<egui::ColorImage>) {
        self.image = Some(image);
        self.notify.notify_waiters();
    }

    pub fn is_ready(&self) -> bool {
        self.image.is_some()
    }

    pub fn image(&self) -> Option<Arc<egui::ColorImage>> {
        self.image.clone()
    }

    pub fn notify(&self) -> Arc<Notify> {
        Arc::clone(&self.notify)
    }
}

#[derive(Debug, Clone)]
pub enum ScreenshotKind {
    Viewport,
    Widget { rect: Rect, pixels_per_point: f32 },
}

pub fn screenshot_kind_label(kind: &ScreenshotKind) -> String {
    match kind {
        ScreenshotKind::Viewport => "viewport".to_string(),
        ScreenshotKind::Widget { .. } => "widget".to_string(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ScreenshotDebugRequest {
    pub request_id: u64,
    pub viewport_id: String,
    pub kind: String,
    pub frame_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ScreenshotDebugCommand {
    pub request_id: Option<u64>,
    pub viewport_id: String,
    pub frame_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ScreenshotDebugEvent {
    pub request_id: Option<u64>,
    pub viewport_id: String,
    pub frame_count: u64,
    pub matched: bool,
    pub has_user_data: bool,
    pub image_size: Option<[usize; 2]>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ScreenshotDebugState {
    pub requests_queued: u64,
    pub commands_sent: u64,
    pub events_seen: u64,
    pub events_matched: u64,
    pub events_unknown_request: u64,
    pub events_missing_user_data: u64,
    pub last_request: Option<ScreenshotDebugRequest>,
    pub last_command: Option<ScreenshotDebugCommand>,
    pub last_event: Option<ScreenshotDebugEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ScreenshotDebugRequestInfo {
    pub request_id: u64,
    pub kind: String,
    pub ready: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ScreenshotDebugSnapshot {
    pub enabled: bool,
    pub frame_count: u64,
    pub pending_requests: Vec<ScreenshotDebugRequestInfo>,
    pub debug: ScreenshotDebugState,
}

#[derive(Debug)]
pub struct ScreenshotManager {
    screenshots: Mutex<HashMap<u64, ScreenshotState>>,
    screenshot_debug: Mutex<ScreenshotDebugState>,
}

impl ScreenshotManager {
    pub(crate) fn new() -> Self {
        Self {
            screenshots: Mutex::new(HashMap::new()),
            screenshot_debug: Mutex::new(ScreenshotDebugState::default()),
        }
    }

    pub(crate) fn screenshot_state(&self, request_id: u64) -> Option<ScreenshotState> {
        lock(&self.screenshots, "screenshots lock")
            .get(&request_id)
            .cloned()
    }

    pub(crate) fn insert_screenshot(&self, request_id: u64, state: ScreenshotState) {
        let mut screenshots = lock(&self.screenshots, "screenshots lock");
        screenshots.insert(request_id, state);
    }

    pub(crate) fn take_screenshot(&self, request_id: u64) -> Option<ScreenshotState> {
        let mut screenshots = lock(&self.screenshots, "screenshots lock");
        screenshots.remove(&request_id)
    }

    pub(crate) fn mark_screenshot_ready(
        &self,
        request_id: u64,
        image: Arc<egui::ColorImage>,
    ) -> bool {
        let mut screenshots = lock(&self.screenshots, "screenshots lock");
        if let Some(state) = screenshots.get_mut(&request_id) {
            state.mark_ready(image);
            true
        } else {
            false
        }
    }

    pub(crate) fn capture_screenshot_events(
        &self,
        events: &[egui::Event],
        enabled: bool,
        frame_count: u64,
    ) {
        for event in events {
            let egui::Event::Screenshot {
                viewport_id,
                user_data,
                image,
            } = event
            else {
                continue;
            };
            let request_id = user_data
                .data
                .as_ref()
                .and_then(|data| data.downcast_ref::<u64>())
                .copied();
            let matched = request_id
                .map(|request_id| self.mark_screenshot_ready(request_id, Arc::clone(image)))
                .unwrap_or(false);
            let has_user_data = request_id.is_some();
            let mut debug = lock(&self.screenshot_debug, "screenshot debug lock");
            debug.events_seen += 1;
            if has_user_data {
                if matched {
                    debug.events_matched += 1;
                } else {
                    debug.events_unknown_request += 1;
                }
            } else {
                debug.events_missing_user_data += 1;
            }
            self.log_screenshot(
                enabled,
                format!(
                    "event received request_id={request_id:?} viewport={} matched={matched} \
                     user_data={has_user_data} size={:?} frame={frame_count}",
                    viewport_id_to_string(*viewport_id),
                    image.size,
                ),
            );
            debug.last_event = Some(ScreenshotDebugEvent {
                request_id,
                viewport_id: viewport_id_to_string(*viewport_id),
                frame_count,
                matched,
                has_user_data,
                image_size: Some(image.size),
            });
        }
    }

    pub(crate) fn record_screenshot_request(
        &self,
        request_id: u64,
        viewport_id: egui::ViewportId,
        kind: &ScreenshotKind,
        enabled: bool,
        frame_count: u64,
    ) {
        let mut debug = lock(&self.screenshot_debug, "screenshot debug lock");
        debug.requests_queued += 1;
        debug.last_request = Some(ScreenshotDebugRequest {
            request_id,
            viewport_id: viewport_id_to_string(viewport_id),
            kind: screenshot_kind_label(kind),
            frame_count,
        });
        self.log_screenshot(
            enabled,
            format!(
                "request queued id={request_id} viewport={} kind={} frame={frame_count}",
                viewport_id_to_string(viewport_id),
                screenshot_kind_label(kind),
            ),
        );
    }

    pub(crate) fn record_screenshot_command_sent(
        &self,
        viewport_id: egui::ViewportId,
        request_id: Option<u64>,
        enabled: bool,
        frame_count: u64,
    ) {
        let mut debug = lock(&self.screenshot_debug, "screenshot debug lock");
        debug.commands_sent += 1;
        debug.last_command = Some(ScreenshotDebugCommand {
            request_id,
            viewport_id: viewport_id_to_string(viewport_id),
            frame_count,
        });
        self.log_screenshot(
            enabled,
            format!(
                "command sent request_id={request_id:?} viewport={} frame={frame_count}",
                viewport_id_to_string(viewport_id),
            ),
        );
    }

    pub(crate) fn screenshot_debug_snapshot(
        &self,
        enabled: bool,
        frame_count: u64,
    ) -> ScreenshotDebugSnapshot {
        let screenshots = lock(&self.screenshots, "screenshots lock");
        let pending_requests = screenshots
            .iter()
            .map(|(request_id, state)| ScreenshotDebugRequestInfo {
                request_id: *request_id,
                kind: screenshot_kind_label(&state.kind),
                ready: state.is_ready(),
            })
            .collect::<Vec<_>>();
        let debug = lock(&self.screenshot_debug, "screenshot debug lock").clone();
        ScreenshotDebugSnapshot {
            enabled,
            frame_count,
            pending_requests,
            debug,
        }
    }

    pub(crate) fn log_screenshot(&self, enabled: bool, line: impl AsRef<str>) {
        if !enabled {
            return;
        }
        eprintln!("eguidev: screenshot {}", line.as_ref());
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn capture_screenshot_events_marks_ready_from_raw_input() {
        let manager = ScreenshotManager::new();
        let request_id = 42_u64;
        manager.insert_screenshot(
            request_id,
            ScreenshotState::pending(ScreenshotKind::Viewport),
        );

        let image = Arc::new(egui::ColorImage::new([1, 1], vec![egui::Color32::WHITE]));
        let event = egui::Event::Screenshot {
            viewport_id: egui::ViewportId::ROOT,
            user_data: egui::UserData::new(request_id),
            image: Arc::clone(&image),
        };
        let events = [event];
        manager.capture_screenshot_events(&events, true, 0);

        let state = manager
            .screenshot_state(request_id)
            .expect("screenshot state");
        assert!(state.is_ready());
        assert!(state.image().is_some());

        let snapshot = manager.screenshot_debug_snapshot(true, 0);
        assert_eq!(snapshot.debug.events_seen, 1);
        assert_eq!(snapshot.debug.events_matched, 1);
        assert_eq!(snapshot.debug.events_unknown_request, 0);
        assert_eq!(snapshot.debug.events_missing_user_data, 0);
        let last_event = snapshot.debug.last_event.expect("last event");
        assert_eq!(last_event.request_id, Some(request_id));
        assert!(last_event.matched);
    }

    #[test]
    fn capture_screenshot_events_records_missing_user_data() {
        let manager = ScreenshotManager::new();

        let image = Arc::new(egui::ColorImage::new([1, 1], vec![egui::Color32::WHITE]));
        let event = egui::Event::Screenshot {
            viewport_id: egui::ViewportId::ROOT,
            user_data: egui::UserData::default(),
            image,
        };
        let events = [event];
        manager.capture_screenshot_events(&events, true, 0);

        let snapshot = manager.screenshot_debug_snapshot(true, 0);
        assert_eq!(snapshot.debug.events_seen, 1);
        assert_eq!(snapshot.debug.events_missing_user_data, 1);
        assert_eq!(snapshot.debug.events_matched, 0);
        assert_eq!(snapshot.debug.events_unknown_request, 0);
        let last_event = snapshot.debug.last_event.expect("last event");
        assert!(last_event.request_id.is_none());
        assert!(!last_event.matched);
    }
}
