//! Embedded runtime attachment for DevMCP automation.

use std::{any::Any, sync::Arc, time::Duration};

use egui::Context;
use eguidev::internal::{devmcp::RuntimeHooks, registry::Inner};
use tokio::{runtime::Handle, sync::Notify};

use crate::{
    DevMcp, ScriptErrorInfo, ScriptEvalOptions, ScriptEvalOutcome,
    screenshots::{ScreenshotDebugSnapshot, ScreenshotKind, ScreenshotManager, ScreenshotState},
    server::start_server,
    tools::{DEFAULT_POLL_INTERVAL_MS, DEFAULT_SCRIPT_EVAL_TIMEOUT_MS, script::run_script_eval},
};

#[derive(Debug)]
pub struct Runtime {
    screenshots: ScreenshotManager,
    frame_notify: Notify,
}

#[derive(Debug)]
struct RuntimeHooksImpl {
    runtime: Arc<Runtime>,
}

impl Runtime {
    fn new() -> Self {
        Self {
            screenshots: ScreenshotManager::new(),
            frame_notify: Notify::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn ensure_for_inner(inner: &Arc<Inner>) -> Arc<Self> {
        if let Some(runtime) = Self::from_inner(inner) {
            return runtime;
        }
        let runtime = Arc::new(Self::new());
        inner.set_runtime_hooks(Arc::new(RuntimeHooksImpl {
            runtime: Arc::clone(&runtime),
        }));
        runtime
    }

    pub(crate) fn from_inner(inner: &Inner) -> Option<Arc<Self>> {
        let hooks = inner.runtime_hooks()?;
        let hooks = hooks.as_any().downcast_ref::<RuntimeHooksImpl>()?;
        Some(Arc::clone(&hooks.runtime))
    }

    pub(crate) fn for_devmcp(devmcp: &DevMcp) -> Option<Arc<Self>> {
        let hooks = devmcp.runtime_hooks()?;
        let hooks = hooks.as_any().downcast_ref::<RuntimeHooksImpl>()?;
        Some(Arc::clone(&hooks.runtime))
    }

    pub(crate) fn frame_notify(&self) -> &Notify {
        &self.frame_notify
    }

    pub(crate) fn screenshot_state(&self, request_id: u64) -> Option<ScreenshotState> {
        self.screenshots.screenshot_state(request_id)
    }

    pub(crate) fn insert_screenshot(&self, request_id: u64, state: ScreenshotState) {
        self.screenshots.insert_screenshot(request_id, state);
    }

    pub(crate) fn take_screenshot(&self, request_id: u64) -> Option<ScreenshotState> {
        self.screenshots.take_screenshot(request_id)
    }

    pub(crate) fn record_screenshot_request(
        &self,
        inner: &Inner,
        request_id: u64,
        viewport_id: egui::ViewportId,
        kind: &ScreenshotKind,
    ) {
        self.screenshots.record_screenshot_request(
            request_id,
            viewport_id,
            kind,
            inner.verbose_logging(),
            inner.frame_count(),
        );
    }

    pub(crate) fn record_screenshot_command_sent(
        &self,
        inner: &Inner,
        viewport_id: egui::ViewportId,
        request_id: Option<u64>,
    ) {
        self.screenshots.record_screenshot_command_sent(
            viewport_id,
            request_id,
            inner.verbose_logging(),
            inner.frame_count(),
        );
    }

    pub(crate) fn screenshot_debug_snapshot(&self, inner: &Inner) -> ScreenshotDebugSnapshot {
        self.screenshots
            .screenshot_debug_snapshot(true, inner.frame_count())
    }

    pub(crate) fn log_screenshot(&self, inner: &Inner, message: String) {
        self.screenshots
            .log_screenshot(inner.verbose_logging(), message);
    }

    fn capture_screenshot_events(&self, inner: &Inner, events: &[egui::Event]) {
        self.screenshots.capture_screenshot_events(
            events,
            inner.verbose_logging(),
            inner.frame_count(),
        );
    }

    fn finish_frame(&self, inner: &Inner, ctx: &Context) {
        let mut sent_viewport_command = false;
        for (viewport_id, commands) in inner.actions.drain_all_commands() {
            for command in commands {
                sent_viewport_command = true;
                if let egui::ViewportCommand::Screenshot(user_data) = &command {
                    let request_id = user_data
                        .data
                        .as_ref()
                        .and_then(|data| data.downcast_ref::<u64>())
                        .copied();
                    self.record_screenshot_command_sent(inner, viewport_id, request_id);
                }
                ctx.send_viewport_cmd_to(viewport_id, command);
            }
        }
        if sent_viewport_command {
            ctx.request_repaint();
        }
        inner.viewports.update_viewports(ctx);
        inner.paint_overlays(ctx);
        self.frame_notify.notify_waiters();
        ctx.request_repaint_after(Duration::from_millis(DEFAULT_POLL_INTERVAL_MS));
    }
}

impl RuntimeHooks for RuntimeHooksImpl {
    fn as_any(&self) -> &(dyn Any + Send + Sync) {
        self
    }

    fn on_raw_input(&self, inner: &Inner, events: &[egui::Event]) {
        self.runtime.capture_screenshot_events(inner, events);
    }

    fn on_frame_end(&self, inner: &Inner, ctx: &Context) {
        self.runtime.finish_frame(inner, ctx);
    }
}

/// Attach the embedded runtime to an inert `DevMcp` handle.
pub fn attach(devmcp: DevMcp) -> DevMcp {
    attach_internal(devmcp, true)
}

fn attach_internal(devmcp: DevMcp, should_start_server: bool) -> DevMcp {
    if devmcp.is_enabled() {
        return devmcp;
    }

    let inner = Arc::new(Inner::new());
    let runtime = Arc::new(Runtime::new());
    let hooks = Arc::new(RuntimeHooksImpl {
        runtime: Arc::clone(&runtime),
    });
    let devmcp = devmcp.activate_runtime(Arc::clone(&inner), hooks);
    if should_start_server {
        start_server(inner, runtime);
    }
    devmcp
}

#[cfg(test)]
pub fn attach_for_tests(devmcp: DevMcp) -> DevMcp {
    attach_internal(devmcp, false)
}

/// Evaluate a Luau script directly against this attached `DevMcp` instance.
pub fn eval_script(
    devmcp: &DevMcp,
    handle: Handle,
    script_source: &str,
    timeout_ms: Option<u64>,
    options: ScriptEvalOptions,
) -> ScriptEvalOutcome {
    let Some(inner) = devmcp.inner_arc() else {
        return ScriptEvalOutcome::error_only(ScriptErrorInfo {
            error_type: "runtime".to_string(),
            message: "DevMCP runtime is not attached".to_string(),
            location: None,
            backtrace: None,
            code: None,
            details: None,
        });
    };
    let runtime = Runtime::for_devmcp(devmcp).expect("runtime attached");
    let timeout_ms = timeout_ms.unwrap_or(DEFAULT_SCRIPT_EVAL_TIMEOUT_MS);
    let source_name = options
        .source_name
        .unwrap_or_else(|| "script.luau".to_string());
    run_script_eval(
        inner,
        runtime,
        handle,
        script_source,
        timeout_ms,
        source_name,
        options.args,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::{reset_start_server_calls, start_server_calls};

    #[test]
    fn inactive_handle_does_not_start_server() {
        reset_start_server_calls();
        let _devmcp = DevMcp::new();
        assert_eq!(start_server_calls(), 0);
    }

    #[test]
    fn attach_starts_server_once() {
        reset_start_server_calls();
        let _devmcp = attach(DevMcp::new());
        assert_eq!(start_server_calls(), 1);
    }
}
