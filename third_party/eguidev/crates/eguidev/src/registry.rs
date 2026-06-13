//! Internal state and registry capture.
#![allow(missing_docs)]

use std::{
    collections::HashMap,
    fmt,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use egui::{Context, Vec2 as EguiVec2};

use crate::{
    actions::{ActionQueue, ActionTiming, InputAction},
    devmcp::RuntimeHooks,
    fixtures::FixtureManager,
    overlay::{OverlayDebugConfig, OverlayEntry, OverlayManager},
    types::WidgetValue,
    viewports::ViewportState,
    widget_registry::WidgetRegistry,
};

pub fn lock<'a, T>(mutex: &'a Mutex<T>, label: &'static str) -> MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            eprintln!("eguidev: recovering poisoned lock: {label}");
            poisoned.into_inner()
        }
    }
}

pub struct Inner {
    pub actions: ActionQueue,
    pub viewports: ViewportState,
    pub widgets: WidgetRegistry,
    pub overlays: OverlayManager,
    contexts: Mutex<HashMap<egui::ViewportId, Context>>,
    widget_value_updates: Mutex<HashMap<WidgetValueKey, WidgetValue>>,
    scroll_overrides: Mutex<HashMap<ScrollAreaKey, EguiVec2>>,
    next_request_id: AtomicU64,
    frame_count: AtomicU64,
    fixture_epoch: AtomicU64,
    pub last_action_frame: AtomicU64,
    verbose_logging: AtomicBool,
    pub fixtures: FixtureManager,
    runtime_hooks: Mutex<Option<Arc<dyn RuntimeHooks>>>,
}

impl Default for Inner {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Inner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Inner")
            .field("fixtures", &self.fixtures.fixtures().len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct WidgetValueKey {
    viewport_id: egui::ViewportId,
    id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ScrollAreaKey {
    viewport_id: egui::ViewportId,
    widget_id: u64,
}

impl WidgetValueKey {
    fn new(viewport_id: egui::ViewportId, id: impl Into<String>) -> Self {
        Self {
            viewport_id,
            id: id.into(),
        }
    }
}

impl ScrollAreaKey {
    fn new(viewport_id: egui::ViewportId, widget_id: u64) -> Self {
        Self {
            viewport_id,
            widget_id,
        }
    }
}

impl Inner {
    pub fn new() -> Self {
        Self {
            actions: ActionQueue::new(),
            viewports: ViewportState::new(),
            widgets: WidgetRegistry::new(),
            overlays: OverlayManager::new(),
            contexts: Mutex::new(HashMap::new()),
            widget_value_updates: Mutex::new(HashMap::new()),
            scroll_overrides: Mutex::new(HashMap::new()),
            next_request_id: AtomicU64::new(1),
            frame_count: AtomicU64::new(0),
            fixture_epoch: AtomicU64::new(0),
            last_action_frame: AtomicU64::new(0),
            verbose_logging: AtomicBool::new(false),
            fixtures: FixtureManager::new(),
            runtime_hooks: Mutex::new(None),
        }
    }

    pub fn set_runtime_hooks(&self, hooks: Arc<dyn RuntimeHooks>) {
        *lock(&self.runtime_hooks, "runtime hooks lock") = Some(hooks);
    }

    pub fn runtime_hooks(&self) -> Option<Arc<dyn RuntimeHooks>> {
        lock(&self.runtime_hooks, "runtime hooks lock").clone()
    }

    /// Apply a named fixture by calling the registered handler.
    pub fn apply_fixture(&self, name: &str) -> Result<(), String> {
        self.fixtures.apply_fixture(name)
    }

    pub fn reset_fixture_transient_state(&self) {
        self.actions.clear_all();
        lock(&self.widget_value_updates, "widget value update lock").clear();
        lock(&self.scroll_overrides, "scroll overrides lock").clear();
        self.overlays.clear_transient_state();
        self.request_repaint_of(egui::ViewportId::ROOT);
    }

    pub fn set_verbose_logging(&self, verbose_logging: bool) {
        self.verbose_logging
            .store(verbose_logging, Ordering::Relaxed);
    }

    pub fn verbose_logging(&self) -> bool {
        self.verbose_logging.load(Ordering::Relaxed)
    }

    pub fn capture_context(&self, viewport_id: egui::ViewportId, ctx: &Context) {
        let mut stored = lock(&self.contexts, "contexts lock");
        stored.insert(viewport_id, ctx.clone());
        self.viewports.remember_viewport_id(viewport_id);
    }

    pub fn context_for(&self, viewport_id: egui::ViewportId) -> Option<Context> {
        let contexts = lock(&self.contexts, "contexts lock");
        contexts.get(&viewport_id).cloned()
    }

    pub fn has_context(&self) -> bool {
        !lock(&self.contexts, "contexts lock").is_empty()
    }

    pub fn request_repaint(&self) {
        self.request_repaint_of(egui::ViewportId::ROOT);
    }

    pub fn request_repaint_all(&self) {
        let contexts = {
            let contexts = lock(&self.contexts, "contexts lock");
            contexts.values().cloned().collect::<Vec<_>>()
        };
        for ctx in contexts {
            ctx.request_repaint();
        }
    }

    pub fn request_repaint_of(&self, viewport_id: egui::ViewportId) {
        let ctx = {
            let contexts = lock(&self.contexts, "contexts lock");
            contexts.get(&viewport_id).cloned()
        };
        if let Some(ctx) = ctx {
            ctx.request_repaint();
        }
    }

    pub fn queue_widget_value_update(
        &self,
        viewport_id: egui::ViewportId,
        id: String,
        value: WidgetValue,
    ) {
        let mut updates = lock(&self.widget_value_updates, "widget value update lock");
        updates.insert(WidgetValueKey::new(viewport_id, id), value);
        self.request_repaint_of(viewport_id);
    }

    pub fn take_widget_value_update(
        &self,
        viewport_id: egui::ViewportId,
        id: &str,
    ) -> Option<WidgetValue> {
        let mut updates = lock(&self.widget_value_updates, "widget value update lock");
        updates.remove(&WidgetValueKey::new(viewport_id, id))
    }

    pub fn set_overlay_debug_config(&self, config: OverlayDebugConfig) {
        self.overlays.set_overlay_debug_config(config);
        self.request_repaint();
    }

    pub fn set_overlay(&self, key: String, overlay: OverlayEntry) {
        self.overlays.set_overlay(key, overlay);
        self.request_repaint();
    }

    pub fn remove_overlay(&self, key: &str) {
        self.overlays.remove_overlay(key);
        self.request_repaint();
    }

    pub fn clear_overlays(&self) {
        self.overlays.clear_overlays();
        self.request_repaint();
    }

    pub fn paint_overlays(&self, ctx: &Context) {
        self.overlays
            .paint_overlays(ctx, &self.widgets, &self.viewports);
    }

    pub fn set_scroll_override(
        &self,
        viewport_id: egui::ViewportId,
        widget_id: u64,
        offset: egui::Vec2,
    ) {
        let mut overrides = lock(&self.scroll_overrides, "scroll overrides lock");
        overrides.insert(ScrollAreaKey::new(viewport_id, widget_id), offset);
        self.request_repaint_of(viewport_id);
    }

    pub fn take_scroll_override(
        &self,
        viewport_id: egui::ViewportId,
        widget_id: u64,
    ) -> Option<egui::Vec2> {
        let mut overrides = lock(&self.scroll_overrides, "scroll overrides lock");
        overrides.remove(&ScrollAreaKey::new(viewport_id, widget_id))
    }

    pub fn queue_action(&self, viewport_id: egui::ViewportId, action: InputAction) {
        self.queue_action_with_timing(viewport_id, ActionTiming::Current, action);
    }

    pub fn queue_action_with_timing(
        &self,
        viewport_id: egui::ViewportId,
        timing: ActionTiming,
        action: InputAction,
    ) {
        self.actions
            .queue_action_with_timing(viewport_id, timing, action);
        self.request_repaint_of(viewport_id);
    }

    pub fn queue_command(&self, viewport_id: egui::ViewportId, command: egui::ViewportCommand) {
        self.actions.queue_command(viewport_id, command);
        self.request_repaint_of(viewport_id);
    }

    pub fn next_request_id(&self) -> u64 {
        self.next_request_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn clear_all(&self) {
        lock(&self.widget_value_updates, "widget values lock").clear();
        lock(&self.scroll_overrides, "scroll overrides lock").clear();
        self.actions.clear_all();
    }

    pub fn advance_frame(&self) {
        self.frame_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count.load(Ordering::Relaxed)
    }

    pub fn begin_fixture_epoch(&self) -> u64 {
        self.fixture_epoch.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn fixture_epoch(&self) -> u64 {
        self.fixture_epoch.load(Ordering::Relaxed)
    }
}

pub fn viewport_id_to_string(viewport_id: egui::ViewportId) -> String {
    if viewport_id == egui::ViewportId::ROOT {
        "root".to_string()
    } else {
        format!("vp:{:x}", viewport_id.0.value())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, mpsc},
        time::Duration,
    };

    use super::*;

    #[test]
    fn request_repaint_does_not_hold_contexts_lock_across_callback() {
        let inner = Arc::new(new_test_inner());
        let ctx = Context::default();
        inner.capture_context(egui::ViewportId::ROOT, &ctx);

        let inner_for_callback = Arc::clone(&inner);
        let (sender, receiver) = mpsc::channel();
        ctx.set_request_repaint_callback(move |_| {
            assert!(
                inner_for_callback
                    .context_for(egui::ViewportId::ROOT)
                    .is_some()
            );
            sender.send(()).expect("notify repaint callback");
        });

        inner.request_repaint_of(egui::ViewportId::ROOT);
        receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("repaint callback");
    }

    fn new_test_inner() -> Inner {
        Inner::new()
    }
}
