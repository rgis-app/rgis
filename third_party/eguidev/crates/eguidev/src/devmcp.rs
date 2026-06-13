//! DevMCP instrumentation helpers and public API.
#![allow(missing_docs)]

use std::{
    any::Any,
    fmt,
    sync::{Arc, atomic::Ordering},
};

use egui::Context;

use crate::{
    actions::InputAction,
    fixtures::FixtureHandler,
    instrument::{ACTIVE, swallow_panic},
    registry::Inner,
    types::FixtureSpec,
};

#[derive(Clone, Debug, Default)]
enum DevMcpState {
    #[default]
    Inactive,
    Active(Arc<Inner>),
}

pub trait RuntimeHooks: Send + Sync {
    fn as_any(&self) -> &(dyn Any + Send + Sync);

    fn on_raw_input(&self, _inner: &Inner, _events: &[egui::Event]) {}

    fn on_frame_end(&self, _inner: &Inner, _ctx: &Context) {}
}

/// DevMCP handle stored in app state.
#[derive(Clone, Default)]
pub struct DevMcp {
    state: DevMcpState,
    fixtures: Vec<FixtureSpec>,
    verbose_logging: bool,
    fixture_handler: Option<FixtureHandler>,
}

impl fmt::Debug for DevMcp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DevMcp")
            .field("state", &self.state)
            .field("fixtures", &self.fixtures)
            .field("verbose_logging", &self.verbose_logging)
            .finish()
    }
}

impl DevMcp {
    /// Create a new inert DevMCP handle.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable or disable verbose internal logging for DevMCP operations.
    pub fn verbose_logging(mut self, verbose_logging: bool) -> Self {
        self.verbose_logging = verbose_logging;
        if let Some(inner) = self.inner() {
            inner.set_verbose_logging(verbose_logging);
        }
        self
    }

    /// Register fixture metadata for discovery and validation.
    pub fn fixtures(mut self, fixtures: impl IntoIterator<Item = FixtureSpec>) -> Self {
        self.fixtures = fixtures.into_iter().collect();
        if let Some(inner) = self.inner() {
            inner.fixtures.set_fixtures(self.fixtures.clone());
        }
        self
    }

    /// Register a callback that applies named fixtures to app state.
    ///
    /// The handler is called directly from the tokio runtime when a fixture
    /// tool call arrives, removing the need for frame-driven polling.
    pub fn on_fixture<F>(mut self, handler: F) -> Self
    where
        F: Fn(&str) -> Result<(), String> + Send + Sync + 'static,
    {
        let handler: FixtureHandler = Arc::new(handler);
        if let Some(inner) = self.inner() {
            inner.fixtures.set_fixture_handler(handler.clone());
        }
        self.fixture_handler = Some(handler);
        self
    }

    /// Returns true if DevMCP automation is attached.
    pub fn is_enabled(&self) -> bool {
        matches!(self.state, DevMcpState::Active(_))
    }

    #[doc(hidden)]
    pub fn inner_arc(&self) -> Option<Arc<Inner>> {
        self.inner().map(Arc::clone)
    }

    #[doc(hidden)]
    pub fn runtime_hooks(&self) -> Option<Arc<dyn RuntimeHooks>> {
        self.inner().and_then(|inner| inner.runtime_hooks())
    }

    fn verbose_logging_enabled(&self) -> bool {
        self.inner()
            .map_or(self.verbose_logging, |inner| inner.verbose_logging())
    }

    #[cfg(test)]
    fn context_for(&self, viewport_id: egui::ViewportId) -> Option<Context> {
        self.inner()
            .and_then(|inner| inner.context_for(viewport_id))
    }

    #[doc(hidden)]
    pub fn activate_runtime(mut self, inner: Arc<Inner>, hooks: Arc<dyn RuntimeHooks>) -> Self {
        inner.set_runtime_hooks(hooks);
        inner.set_verbose_logging(self.verbose_logging);
        if !self.fixtures.is_empty() {
            inner.fixtures.set_fixtures(self.fixtures.clone());
        }
        if let Some(handler) = &self.fixture_handler {
            inner.fixtures.set_fixture_handler(handler.clone());
        }
        self.state = DevMcpState::Active(inner);
        self
    }

    pub(crate) fn inner(&self) -> Option<&Arc<Inner>> {
        match &self.state {
            DevMcpState::Inactive => None,
            DevMcpState::Active(inner) => Some(inner),
        }
    }

    /// Begin a frame, enabling widget tracking for this thread.
    ///
    /// Prefer [`FrameGuard`] over calling this directly.
    pub fn begin_frame(&self, ctx: &Context) {
        let Some(inner) = self.inner() else {
            return;
        };
        swallow_panic("begin_frame", || {
            let viewport_id = ctx.viewport_id();
            inner.capture_context(viewport_id, ctx);
            inner.widgets.clear_registry(viewport_id);
            ACTIVE.with(|active| {
                if let Ok(mut active) = active.try_borrow_mut() {
                    *active = Some(Arc::clone(inner));
                } else {
                    eprintln!("eguidev: begin_frame skipped; active already borrowed");
                }
            });
        });
    }

    /// End a frame, finalizing widget registry and handling automation state.
    ///
    /// Prefer [`FrameGuard`] over calling this directly.
    pub fn end_frame(&self, ctx: &Context) {
        let Some(inner) = self.inner() else {
            return;
        };
        swallow_panic("end_frame", || {
            self.finish_frame(inner, ctx);
            ACTIVE.with(|active| {
                if let Ok(mut active) = active.try_borrow_mut() {
                    *active = None;
                } else {
                    eprintln!("eguidev: end_frame skipped; active already borrowed");
                }
            });
        });
    }

    fn finish_frame(&self, inner: &Arc<Inner>, ctx: &Context) {
        inner.widgets.finalize_registry(ctx.viewport_id());
        let next_frame = inner.frame_count() + 1;
        inner
            .viewports
            .capture_input_snapshot(ctx, inner.fixture_epoch(), next_frame);
        inner.advance_frame();
        if let Some(hooks) = inner.runtime_hooks() {
            hooks.on_frame_end(inner, ctx);
        }
    }

    /// Inject queued raw input during the raw input hook.
    pub(crate) fn raw_input_hook(&self, ctx: &Context, raw_input: &mut egui::RawInput) {
        let Some(inner) = self.inner() else {
            return;
        };
        swallow_panic("raw_input_hook", || {
            let viewport_id = raw_input.viewport_id;
            inner.capture_context(viewport_id, ctx);
            if let Some(hooks) = inner.runtime_hooks() {
                hooks.on_raw_input(inner, &raw_input.events);
            }
            let actions = inner.actions.drain_actions(viewport_id);
            if !actions.is_empty() {
                inner
                    .last_action_frame
                    .store(inner.frame_count(), Ordering::Relaxed);
                if self.verbose_logging_enabled() {
                    eprintln!(
                        "eguidev: raw_input_hook viewport={:?} actions={}",
                        viewport_id,
                        actions.len()
                    );
                }
            }
            let base_modifiers = raw_input.modifiers;
            let mut current_modifiers = base_modifiers;
            let mut force_focus = false;
            for action in &actions {
                if let InputAction::Key {
                    pressed, modifiers, ..
                } = action
                {
                    current_modifiers = if *pressed {
                        base_modifiers.plus((*modifiers).into())
                    } else {
                        base_modifiers
                    };
                }
                if matches!(
                    action,
                    InputAction::Key { .. } | InputAction::Text { .. } | InputAction::Paste { .. }
                ) {
                    force_focus = true;
                }
            }
            raw_input.modifiers = current_modifiers;
            if force_focus {
                raw_input.focused = true;
            }
            for action in actions {
                action.apply(raw_input);
            }
        });
    }
}

/// RAII guard that calls `begin_frame` and `end_frame` automatically.
#[must_use = "FrameGuard must be held for the duration of the frame"]
pub struct FrameGuard<'a> {
    /// DevMcp handle for the active frame.
    devmcp: &'a DevMcp,
    /// Egui context for the current frame.
    ctx: &'a egui::Context,
}

impl<'a> FrameGuard<'a> {
    /// Create a new frame guard for the provided DevMcp.
    pub fn new(devmcp: &'a DevMcp, ctx: &'a Context) -> Self {
        devmcp.begin_frame(ctx);
        Self { devmcp, ctx }
    }
}

impl Drop for FrameGuard<'_> {
    fn drop(&mut self) {
        self.devmcp.end_frame(self.ctx);
    }
}

/// Forward the raw input hook into the DevMcp handler.
pub fn raw_input_hook(devmcp: &DevMcp, ctx: &Context, raw_input: &mut egui::RawInput) {
    devmcp.raw_input_hook(ctx, raw_input);
}

#[cfg(test)]
#[allow(deprecated)]
#[allow(clippy::tests_outside_test_module)]
mod inactive_tests {
    use egui::Context;

    use super::*;
    use crate::{instrument, ui_ext::DevUiExt};

    #[test]
    fn inactive_raw_input_hook_is_a_noop() {
        let devmcp = DevMcp::new();
        let ctx = Context::default();
        let mut raw_input = egui::RawInput {
            viewport_id: egui::ViewportId::ROOT,
            focused: false,
            ..Default::default()
        };

        devmcp.raw_input_hook(&ctx, &mut raw_input);

        assert!(!raw_input.focused);
        assert!(raw_input.events.is_empty());
    }

    #[test]
    fn inactive_frame_guard_does_not_capture_context() {
        let devmcp = DevMcp::new();
        let ctx = Context::default();
        instrument::reset_test_counters();

        let _output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let ctx = ui.ctx().clone();
            let _guard = FrameGuard::new(&devmcp, &ctx);
            ui.dev_button("inactive.button", "Inactive");
        });

        assert!(devmcp.context_for(egui::ViewportId::ROOT).is_none());
        assert_eq!(instrument::test_layout_capture_count(), 0);
    }
}
