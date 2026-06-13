#![allow(clippy::needless_pass_by_value, clippy::result_large_err)]

use std::{
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use serde::Serialize;
use serde_json::{Map, Value};
use tmcp::ToolResult;
use tokio::{runtime::Handle, time::timeout};

use super::{
    super::{
        DEFAULT_POLL_INTERVAL_MS, DEFAULT_WAIT_TIMEOUT_MS, DevMcpServer, ErrorCode,
        OverlayDebugOptionsInput, ToolError, capture_screenshot, collect_widget_list,
        parse_key_combo, resolve_screenshot_viewport, resolve_widget_and_viewport,
        viewport_snapshot_for, wait_timeout_details,
    },
    parse::{
        map_has_any, map_value, parse_modifiers, parse_optional_bool, parse_optional_f32,
        parse_optional_string, parse_optional_u8, parse_optional_u32, parse_optional_u64,
        parse_optional_u64_val, parse_optional_vec2, parse_overlay_mode, parse_pointer_button,
        parse_pos2, parse_scroll_align, parse_vec2, parse_widget_ref, parse_widget_role,
        widget_value_from_dynamic,
    },
    types::{
        ImageCapture, ScriptAssertion, ScriptErrorInfo, ScriptImageKind, ScriptLocation,
        ScriptPosition, ScriptResult,
    },
};
use crate::{
    registry::{Inner, viewport_id_to_string},
    screenshots::ScreenshotKind,
    types::{Modifiers, Rect, WidgetRef, WidgetRegistryEntry, WidgetState},
    viewports::ViewportSnapshot,
};

pub(super) struct ScriptRuntime {
    pub(super) server: DevMcpServer,
    pub(super) handle: Handle,
    logs: Mutex<Vec<String>>,
    assertions: Mutex<Vec<ScriptAssertion>>,
    images: Mutex<Vec<ImageCapture>>,
    image_counter: AtomicUsize,
    source_name: String,
    deadline: Instant,
    script_timeout_ms: u64,
    config_timeout_ms: Mutex<Option<u64>>,
    config_poll_interval_ms: Mutex<Option<u64>>,
    config_settle: Mutex<Option<bool>>,
}

impl ScriptRuntime {
    pub(super) fn new(
        inner: Arc<Inner>,
        handle: Handle,
        source_name: String,
        timeout_ms: u64,
    ) -> Self {
        let deadline = Instant::now()
            .checked_add(Duration::from_millis(timeout_ms))
            .unwrap_or_else(Instant::now);
        Self {
            server: DevMcpServer::new(inner),
            handle,
            logs: Mutex::new(Vec::new()),
            assertions: Mutex::new(Vec::new()),
            images: Mutex::new(Vec::new()),
            image_counter: AtomicUsize::new(0),
            source_name,
            deadline,
            script_timeout_ms: timeout_ms,
            config_timeout_ms: Mutex::new(None),
            config_poll_interval_ms: Mutex::new(None),
            config_settle: Mutex::new(None),
        }
    }

    pub(super) fn configure(
        &self,
        pos: ScriptPosition,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<()> {
        if let Some(timeout_ms) = parse_optional_u64(options, "timeout_ms")
            .map_err(|error| self.type_error(pos, error.message))?
        {
            *self
                .config_timeout_ms
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = Some(timeout_ms);
        }
        if let Some(poll_interval_ms) = parse_optional_u64(options, "poll_interval_ms")
            .map_err(|error| self.type_error(pos, error.message))?
        {
            *self
                .config_poll_interval_ms
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = Some(poll_interval_ms);
        }
        if let Some(settle) = parse_optional_bool(options, "settle")
            .map_err(|error| self.type_error(pos, error.message))?
        {
            *self.config_settle.lock().unwrap_or_else(|p| p.into_inner()) = Some(settle);
        }
        Ok(())
    }

    fn configured_timeout_ms(&self) -> Option<u64> {
        *self
            .config_timeout_ms
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    fn configured_poll_interval_ms(&self) -> Option<u64> {
        *self
            .config_poll_interval_ms
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    fn configured_settle(&self) -> bool {
        self.config_settle
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .unwrap_or(true)
    }

    pub(super) fn log(&self, line: impl Into<String>) {
        let mut logs = self
            .logs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        logs.push(line.into());
    }

    pub(super) fn logs(&self) -> Vec<String> {
        self.logs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(super) fn source_name(&self) -> &str {
        &self.source_name
    }

    pub(super) fn assertions(&self) -> Vec<ScriptAssertion> {
        self.assertions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn record_assertion(&self, passed: bool, message: String, pos: ScriptPosition) {
        let location = self.format_location(pos);
        let assertion = ScriptAssertion {
            passed,
            message,
            location,
        };
        let mut assertions = self
            .assertions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assertions.push(assertion);
    }

    pub(super) fn images(&self) -> Vec<ImageCapture> {
        self.images
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn next_image_id(&self) -> String {
        let id = self.image_counter.fetch_add(1, Ordering::Relaxed);
        format!("img_{id}")
    }

    fn store_image(&self, image: ImageCapture) {
        let mut images = self
            .images
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        images.push(image);
    }

    fn format_location(&self, pos: ScriptPosition) -> String {
        let source = self.source_name.as_str();
        match (pos.line, pos.column) {
            (Some(line), Some(column)) => format!("{source}:{line}:{column}"),
            (Some(line), None) => format!("{source}:{line}"),
            (None, _) => source.to_string(),
        }
    }

    pub(super) fn error_location(&self, pos: ScriptPosition) -> Option<ScriptLocation> {
        let line = pos.line?;
        Some(ScriptLocation {
            line,
            column: pos.column,
        })
    }

    pub(super) fn type_error(
        &self,
        pos: ScriptPosition,
        message: impl Into<String>,
    ) -> ScriptErrorInfo {
        ScriptErrorInfo {
            error_type: "type_error".to_string(),
            message: message.into(),
            location: self.error_location(pos),
            backtrace: None,
            code: None,
            details: None,
        }
    }

    fn assertion_error(&self, pos: ScriptPosition, message: impl Into<String>) -> ScriptErrorInfo {
        ScriptErrorInfo {
            error_type: "assertion".to_string(),
            message: message.into(),
            location: self.error_location(pos),
            backtrace: None,
            code: None,
            details: None,
        }
    }

    fn runtime_error(&self, pos: ScriptPosition, message: impl Into<String>) -> ScriptErrorInfo {
        ScriptErrorInfo {
            error_type: "runtime".to_string(),
            message: message.into(),
            location: self.error_location(pos),
            backtrace: None,
            code: None,
            details: None,
        }
    }

    fn tool_error(&self, pos: ScriptPosition, error: tmcp::ToolError) -> ScriptErrorInfo {
        let details = error
            .structured
            .as_ref()
            .and_then(|structured| structured.get("error"))
            .and_then(|structured| structured.get("details"))
            .cloned();
        ScriptErrorInfo {
            error_type: if error.code == "timeout" {
                "timeout".to_string()
            } else {
                "tool".to_string()
            },
            message: error.message,
            location: self.error_location(pos),
            backtrace: None,
            code: Some(error.code.to_string()),
            details,
        }
    }

    pub(super) fn script_timeout_error(&self, pos: ScriptPosition) -> ScriptErrorInfo {
        ScriptErrorInfo {
            error_type: "timeout".to_string(),
            message: format!("Script timed out after {}ms", self.script_timeout_ms),
            location: self.error_location(pos),
            backtrace: None,
            code: Some("timeout".to_string()),
            details: None,
        }
    }

    fn remaining_script_duration(&self, pos: ScriptPosition) -> ScriptResult<Duration> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .unwrap_or_default();
        if remaining.is_zero() {
            return Err(self.script_timeout_error(pos));
        }
        Ok(remaining)
    }

    fn block_on_tool<T>(
        &self,
        pos: ScriptPosition,
        fut: impl Future<Output = ToolResult<T>>,
    ) -> ScriptResult<T> {
        let remaining = self.remaining_script_duration(pos)?;
        self.handle
            .block_on(async move {
                timeout(remaining, fut).await.map_err(|_| {
                    ToolError::new(
                        ErrorCode::Timeout,
                        "Script deadline exceeded while waiting for tool call",
                    )
                    .into_tmcp()
                })?
            })
            .map_err(|error| self.tool_error(pos, error))
    }

    fn to_json<T: Serialize>(&self, pos: ScriptPosition, value: T) -> ScriptResult<Value> {
        serde_json::to_value(value).map_err(|error| {
            self.runtime_error(pos, format!("Failed to serialize result: {error}"))
        })
    }

    fn widget_handle_json(
        &self,
        pos: ScriptPosition,
        widget: &WidgetRegistryEntry,
    ) -> ScriptResult<Value> {
        self.to_json(
            pos,
            serde_json::json!({
                "id": widget.id,
                "viewport_id": widget.viewport_id,
                "__viewport_id": widget.viewport_id,
            }),
        )
    }

    fn widget_state_json(
        &self,
        pos: ScriptPosition,
        widget: &WidgetRegistryEntry,
    ) -> ScriptResult<Value> {
        self.to_json(pos, WidgetState::from(widget))
    }

    fn widget_handle_list_json(
        &self,
        pos: ScriptPosition,
        widgets: &[WidgetRegistryEntry],
    ) -> ScriptResult<Value> {
        self.to_json(
            pos,
            widgets
                .iter()
                .map(|widget| {
                    serde_json::json!({
                        "id": widget.id,
                        "viewport_id": widget.viewport_id,
                        "__viewport_id": widget.viewport_id,
                    })
                })
                .collect::<Vec<_>>(),
        )
    }

    fn viewport_handle_json(&self, pos: ScriptPosition, viewport_id: &str) -> ScriptResult<Value> {
        self.to_json(
            pos,
            serde_json::json!({
                "id": viewport_id,
            }),
        )
    }

    fn viewport_state_json(
        &self,
        pos: ScriptPosition,
        snapshot: &ViewportSnapshot,
    ) -> ScriptResult<Value> {
        let input = self.server.inner.viewports.input_snapshot(
            self.server
                .inner
                .viewports
                .resolve_viewport_id(Some(snapshot.viewport_id.clone()))
                .unwrap_or_default(),
        );
        self.to_json(
            pos,
            serde_json::json!({
                "title": snapshot.title,
                "outer_pos": Value::Null,
                "outer_size": snapshot.outer_size,
                "inner_size": snapshot.inner_size,
                "focused": snapshot.focused,
                "minimized": snapshot.minimized,
                "maximized": snapshot.maximized,
                "fullscreen": snapshot.fullscreen,
                "frame_count": self.server.inner.frame_count(),
                "pixels_per_point": input.as_ref().map(|i| i.pixels_per_point).unwrap_or(1.0),
                "pointer_pos": input.as_ref().and_then(|i| i.pointer_pos),
            }),
        )
    }

    fn viewport_handle_list_json(
        &self,
        pos: ScriptPosition,
        snapshots: &[ViewportSnapshot],
    ) -> ScriptResult<Value> {
        self.to_json(
            pos,
            snapshots
                .iter()
                .map(|snapshot| serde_json::json!({ "id": snapshot.viewport_id }))
                .collect::<Vec<_>>(),
        )
    }

    fn modifiers_from_options(
        &self,
        options: Option<&Map<String, Value>>,
    ) -> Result<Option<Modifiers>, ScriptErrorInfo> {
        match options {
            Some(map) => Ok(Some(parse_modifiers(Some(map))?)),
            None => Ok(None),
        }
    }

    pub(super) fn parse_wait_options(
        &self,
        pos: ScriptPosition,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<(Option<String>, Option<u64>, Option<u64>)> {
        let viewport_id = parse_optional_string(options, "viewport_id")
            .map_err(|error| self.type_error(pos, error.message))?;
        let timeout_ms = self.configured_timeout_ms();
        let poll_interval_ms = self.configured_poll_interval_ms();
        Ok((viewport_id, timeout_ms, poll_interval_ms))
    }

    fn resolve_target_viewport(
        &self,
        pos: ScriptPosition,
        viewport_id: Option<&str>,
        target: &WidgetRef,
    ) -> ScriptResult<String> {
        let (_, resolved_viewport_id) =
            resolve_widget_and_viewport(&self.server.inner, viewport_id, target)
                .map_err(|error| self.tool_error(pos, error.into_tmcp()))?;
        Ok(viewport_id_to_string(resolved_viewport_id))
    }

    fn settle_after_action(
        &self,
        pos: ScriptPosition,
        options: Option<&Map<String, Value>>,
        viewport_id: Option<String>,
    ) -> ScriptResult<()> {
        // Per-call settle override takes priority, then global config, then default (true).
        let settle_enabled = parse_optional_bool(options, "settle")
            .map_err(|error| self.type_error(pos, error.message))?
            .unwrap_or_else(|| self.configured_settle());
        if !settle_enabled {
            return Ok(());
        }
        let timeout_ms = self.configured_timeout_ms();
        let poll_interval_ms = self.configured_poll_interval_ms();
        self.block_on_tool(
            pos,
            self.server
                .wait_for_settle(viewport_id, timeout_ms, poll_interval_ms),
        )?;
        Ok(())
    }

    fn parse_action_target(
        &self,
        pos: ScriptPosition,
        target: &Value,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<(WidgetRef, String)> {
        let target =
            parse_widget_ref(target).map_err(|error| self.type_error(pos, error.message))?;
        let viewport_id = parse_optional_string(options, "viewport_id")
            .map_err(|error| self.type_error(pos, error.message))?;
        let action_viewport_id =
            self.resolve_target_viewport(pos, viewport_id.as_deref(), &target)?;
        Ok((target, action_viewport_id))
    }

    fn finish_action<T: Serialize>(
        &self,
        pos: ScriptPosition,
        options: Option<&Map<String, Value>>,
        viewport_id: Option<String>,
        result: T,
        _target: Option<&WidgetRef>,
    ) -> ScriptResult<Value> {
        self.settle_after_action(pos, options, viewport_id)?;
        self.to_json(pos, result)
    }

    pub(super) fn widget_list(
        &self,
        pos: ScriptPosition,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let viewport_id = parse_optional_string(options, "viewport_id")
            .map_err(|error| self.type_error(pos, error.message))?;
        let include_invisible = parse_optional_bool(options, "include_invisible")
            .map_err(|error| self.type_error(pos, error.message))?;
        if map_value(options, "offset").is_some() || map_value(options, "limit").is_some() {
            return Err(self.type_error(pos, "widget_list no longer accepts offset or limit"));
        }
        let role = match map_value(options, "role") {
            None => None,
            Some(value) => Some(
                parse_widget_role(value).map_err(|error| self.type_error(pos, error.message))?,
            ),
        };
        let id_prefix = parse_optional_string(options, "id_prefix")
            .map_err(|error| self.type_error(pos, error.message))?;
        let widgets = collect_widget_list(
            &self.server.inner,
            viewport_id,
            include_invisible,
            role,
            id_prefix.as_deref(),
        )
        .map_err(|error| self.tool_error(pos, error))?;
        self.widget_handle_list_json(pos, &widgets)
    }

    pub(super) fn widget_get(
        &self,
        pos: ScriptPosition,
        target: &Value,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let target =
            parse_widget_ref(target).map_err(|error| self.type_error(pos, error.message))?;
        let viewport_id = parse_optional_string(options, "viewport_id")
            .map_err(|error| self.type_error(pos, error.message))?;
        let result = self.block_on_tool(pos, self.server.widget_get(viewport_id, target))?;
        self.widget_handle_json(pos, &result.widget)
    }

    pub(super) fn widget_set_value(
        &self,
        pos: ScriptPosition,
        target: &Value,
        value: &Value,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let (target, action_viewport_id) = self.parse_action_target(pos, target, options)?;
        let value = widget_value_from_dynamic(value)
            .map_err(|error| self.type_error(pos, error.message))?;
        self.block_on_tool(
            pos,
            self.server
                .widget_set_value(Some(action_viewport_id.clone()), target, value),
        )?;
        self.finish_action(pos, options, Some(action_viewport_id), (), None)
    }

    pub(super) fn widget_at_point(
        &self,
        pos: ScriptPosition,
        pos_arg: &Value,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let point = parse_pos2(pos_arg).map_err(|error| self.type_error(pos, error.message))?;
        let all_layers = parse_optional_bool(options, "all_layers")
            .map_err(|error| self.type_error(pos, error.message))?;
        let viewport_id = parse_optional_string(options, "viewport_id")
            .map_err(|error| self.type_error(pos, error.message))?;
        let result = self.block_on_tool(
            pos,
            self.server.widget_at_point(point, all_layers, viewport_id),
        )?;
        self.widget_handle_list_json(pos, &result.widgets)
    }

    pub(super) fn text_measure(&self, pos: ScriptPosition, target: &Value) -> ScriptResult<Value> {
        let target =
            parse_widget_ref(target).map_err(|error| self.type_error(pos, error.message))?;
        let result = self.block_on_tool(pos, self.server.text_measure(target))?;
        self.to_json(pos, result)
    }

    pub(super) fn action_click(
        &self,
        pos: ScriptPosition,
        target: &Value,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let (target, action_viewport_id) = self.parse_action_target(pos, target, options)?;
        let button = match map_value(options, "button") {
            None => None,
            Some(value) => Some(
                parse_pointer_button(value).map_err(|error| self.type_error(pos, error.message))?,
            ),
        };
        let modifiers = self
            .modifiers_from_options(options)
            .map_err(|error| self.type_error(pos, error.message))?;
        let click_count = parse_optional_u8(options, "click_count")
            .map_err(|error| self.type_error(pos, error.message))?;
        self.block_on_tool(
            pos,
            self.server.action_click(
                Some(action_viewport_id.clone()),
                target.clone(),
                button,
                modifiers,
                click_count,
            ),
        )?;
        self.finish_action(pos, options, Some(action_viewport_id), (), Some(&target))
    }

    pub(super) fn action_hover(
        &self,
        pos: ScriptPosition,
        target: &Value,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let (target, action_viewport_id) = self.parse_action_target(pos, target, options)?;
        let position = parse_optional_vec2(options, "position")
            .map_err(|error| self.type_error(pos, error.message))?;
        let duration_ms = parse_optional_u64(options, "duration_ms")
            .map_err(|error| self.type_error(pos, error.message))?;
        self.block_on_tool(
            pos,
            self.server.action_hover(
                Some(action_viewport_id.clone()),
                target,
                position,
                duration_ms,
            ),
        )?;
        self.finish_action(pos, options, Some(action_viewport_id), (), None)
    }

    pub(super) fn action_type(
        &self,
        pos: ScriptPosition,
        target: &Value,
        text: String,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let (target, action_viewport_id) = self.parse_action_target(pos, target, options)?;
        let clear = parse_optional_bool(options, "clear")
            .map_err(|error| self.type_error(pos, error.message))?;
        let enter = parse_optional_bool(options, "enter")
            .map_err(|error| self.type_error(pos, error.message))?;
        let focus_timeout_ms = parse_optional_u64(options, "focus_timeout_ms")
            .map_err(|error| self.type_error(pos, error.message))?;
        if focus_timeout_ms.is_some() {
            self.block_on_tool(pos, async {
                self.server
                    .focus_widget_for_keyboard(
                        Some(action_viewport_id.clone()),
                        &target,
                        focus_timeout_ms,
                    )
                    .await
                    .map_err(tmcp::ToolError::from)
            })?;
        }
        self.block_on_tool(
            pos,
            self.server.action_type(
                Some(action_viewport_id.clone()),
                target.clone(),
                text,
                enter,
                clear,
            ),
        )?;
        self.finish_action(pos, options, Some(action_viewport_id), (), Some(&target))
    }

    pub(super) fn action_focus(&self, pos: ScriptPosition, target: &Value) -> ScriptResult<Value> {
        let target =
            parse_widget_ref(target).map_err(|error| self.type_error(pos, error.message))?;
        let action_viewport_id = self.resolve_target_viewport(pos, None, &target)?;
        self.block_on_tool(
            pos,
            self.server.action_focus(Some(action_viewport_id), target),
        )?;
        self.to_json(pos, ())
    }

    pub(super) fn action_drag(
        &self,
        pos: ScriptPosition,
        target: &Value,
        to: &Value,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let (target, action_viewport_id) = self.parse_action_target(pos, target, options)?;
        let to = parse_pos2(to).map_err(|error| self.type_error(pos, error.message))?;
        let modifiers = self
            .modifiers_from_options(options)
            .map_err(|error| self.type_error(pos, error.message))?;
        self.block_on_tool(
            pos,
            self.server
                .action_drag(Some(action_viewport_id.clone()), target, to, modifiers),
        )?;
        self.finish_action(pos, options, Some(action_viewport_id), (), None)
    }

    pub(super) fn action_drag_relative(
        &self,
        pos: ScriptPosition,
        target: &Value,
        to: &Value,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let (target, action_viewport_id) = self.parse_action_target(pos, target, options)?;
        let to = parse_vec2(to).map_err(|error| self.type_error(pos, error.message))?;
        let from = parse_optional_vec2(options, "from")
            .map_err(|error| self.type_error(pos, error.message))?;
        let modifiers = self
            .modifiers_from_options(options)
            .map_err(|error| self.type_error(pos, error.message))?;
        self.block_on_tool(
            pos,
            self.server.action_drag_relative(
                Some(action_viewport_id.clone()),
                target,
                from,
                to,
                modifiers,
            ),
        )?;
        self.finish_action(pos, options, Some(action_viewport_id), (), None)
    }

    pub(super) fn action_drag_to_widget(
        &self,
        pos: ScriptPosition,
        from: &Value,
        to: &Value,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let (from, action_viewport_id) = self.parse_action_target(pos, from, options)?;
        let to = parse_widget_ref(to).map_err(|error| self.type_error(pos, error.message))?;
        let modifiers = self
            .modifiers_from_options(options)
            .map_err(|error| self.type_error(pos, error.message))?;
        self.block_on_tool(
            pos,
            self.server.action_drag_to_widget(
                Some(action_viewport_id.clone()),
                from,
                to,
                modifiers,
            ),
        )?;
        self.finish_action(pos, options, Some(action_viewport_id), (), None)
    }

    pub(super) fn action_scroll(
        &self,
        pos: ScriptPosition,
        target: &Value,
        delta: &Value,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let (target, action_viewport_id) = self.parse_action_target(pos, target, options)?;
        let delta = parse_vec2(delta).map_err(|error| self.type_error(pos, error.message))?;
        let modifiers = self
            .modifiers_from_options(options)
            .map_err(|error| self.type_error(pos, error.message))?;
        self.block_on_tool(
            pos,
            self.server
                .action_scroll(Some(action_viewport_id.clone()), target, delta, modifiers),
        )?;
        self.finish_action(pos, options, Some(action_viewport_id), (), None)
    }

    pub(super) fn action_scroll_to(
        &self,
        pos: ScriptPosition,
        target: &Value,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let (target, action_viewport_id) = self.parse_action_target(pos, target, options)?;
        let offset = parse_optional_vec2(options, "offset")
            .map_err(|error| self.type_error(pos, error.message))?;
        let mut align = match map_value(options, "align") {
            None => None,
            Some(value) => Some(
                parse_scroll_align(value).map_err(|error| self.type_error(pos, error.message))?,
            ),
        };
        if offset.is_some() {
            align = None;
        }
        if offset.is_none() && align.is_none() {
            return Err(self.type_error(pos, "scroll_to requires either offset or align"));
        }
        self.block_on_tool(
            pos,
            self.server
                .action_scroll_to(Some(action_viewport_id.clone()), target, offset, align),
        )?;
        self.finish_action(pos, options, Some(action_viewport_id), (), None)
    }

    pub(super) fn action_scroll_into_view(
        &self,
        pos: ScriptPosition,
        target: &Value,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let (target, action_viewport_id) = self.parse_action_target(pos, target, options)?;
        self.block_on_tool(
            pos,
            self.server
                .action_scroll_into_view(Some(action_viewport_id.clone()), target),
        )?;
        self.finish_action(pos, options, Some(action_viewport_id), (), None)
    }

    pub(super) fn action_key(
        &self,
        pos: ScriptPosition,
        combo: String,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let (key, modifiers, key_name) =
            parse_key_combo(&combo).map_err(|msg| self.type_error(pos, msg))?;
        let viewport_id = parse_optional_string(options, "viewport_id")
            .map_err(|error| self.type_error(pos, error.message))?;
        let repeat = parse_optional_u32(options, "repeat_count")
            .map_err(|error| self.type_error(pos, error.message))?;
        let target = match options.and_then(|map| map.get("target")) {
            Some(value) => {
                Some(parse_widget_ref(value).map_err(|error| self.type_error(pos, error.message))?)
            }
            None => None,
        };
        let focus_timeout_ms = parse_optional_u64(options, "focus_timeout_ms")
            .map_err(|error| self.type_error(pos, error.message))?;
        let action_viewport_id = if let Some(target) = &target {
            Some(self.resolve_target_viewport(pos, viewport_id.as_deref(), target)?)
        } else {
            viewport_id
        };
        if let Some(target) = &target {
            let focus_timeout_ms = focus_timeout_ms.or(Some(5_000));
            self.block_on_tool(pos, async {
                self.server
                    .focus_widget_for_keyboard(action_viewport_id.clone(), target, focus_timeout_ms)
                    .await
                    .map_err(tmcp::ToolError::from)
            })?;
        }
        self.block_on_tool(
            pos,
            self.server.action_key(
                action_viewport_id.clone(),
                key,
                modifiers,
                &key_name,
                repeat,
            ),
        )?;
        self.finish_action(pos, options, action_viewport_id, (), target.as_ref())
    }

    pub(super) fn action_paste(
        &self,
        pos: ScriptPosition,
        text: String,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let viewport_id = parse_optional_string(options, "viewport_id")
            .map_err(|error| self.type_error(pos, error.message))?;
        self.block_on_tool(pos, self.server.action_paste(viewport_id.clone(), text))?;
        self.finish_action(pos, options, viewport_id, (), None)
    }

    fn parse_optional_viewport_id_arg(
        &self,
        pos: ScriptPosition,
        arg: Option<&Value>,
    ) -> ScriptResult<Option<String>> {
        match arg {
            None => Ok(None),
            Some(Value::String(value)) => Ok(Some(value.clone())),
            Some(_) => Err(self.type_error(pos, "viewport_id must be a string")),
        }
    }

    pub(super) fn viewport_handle(
        &self,
        pos: ScriptPosition,
        viewport_id: &str,
    ) -> ScriptResult<Value> {
        self.viewport_handle_json(pos, viewport_id)
    }

    pub(super) fn root_viewport(&self, pos: ScriptPosition) -> ScriptResult<Value> {
        self.viewport_handle_json(pos, "root")
    }

    pub(super) fn viewports_list(
        &self,
        pos: ScriptPosition,
        arg: Option<&Value>,
    ) -> ScriptResult<Value> {
        let viewport_id = self.parse_optional_viewport_id_arg(pos, arg)?;
        let result = self.block_on_tool(pos, self.server.viewports_list(viewport_id))?;
        self.viewport_handle_list_json(pos, &result)
    }

    pub(super) fn viewport_state(
        &self,
        pos: ScriptPosition,
        viewport_id: String,
    ) -> ScriptResult<Value> {
        let snapshot = self
            .block_on_tool(pos, self.server.viewports_list(Some(viewport_id.clone())))?
            .into_iter()
            .find(|snapshot| snapshot.viewport_id == viewport_id)
            .ok_or_else(|| {
                self.runtime_error(pos, format!("Viewport `{viewport_id}` is not available"))
            })?;
        self.viewport_state_json(pos, &snapshot)
    }

    pub(super) fn widget_state(&self, pos: ScriptPosition, target: &Value) -> ScriptResult<Value> {
        let target =
            parse_widget_ref(target).map_err(|error| self.type_error(pos, error.message))?;
        let result = self.block_on_tool(pos, self.server.widget_get(None, target))?;
        self.widget_state_json(pos, &result.widget)
    }

    pub(super) fn widget_parent(&self, pos: ScriptPosition, target: &Value) -> ScriptResult<Value> {
        let target =
            parse_widget_ref(target).map_err(|error| self.type_error(pos, error.message))?;
        let widget = self
            .server
            .inner
            .widgets
            .resolve_widget(&self.server.inner.viewports, None, &target)
            .map_err(|error| self.tool_error(pos, error.into_tmcp()))?;
        let Some(parent_id) = widget.parent_id.as_deref() else {
            return Ok(Value::Null);
        };
        let viewport_id = self
            .server
            .inner
            .viewports
            .resolve_viewport_id(Some(widget.viewport_id.clone()))
            .map_err(|error| self.tool_error(pos, error.into_tmcp()))?;
        let parent = self
            .server
            .inner
            .widgets
            .widget_list(viewport_id)
            .into_iter()
            .find(|candidate| {
                candidate.id == parent_id
                    && candidate.rect.min.x <= widget.rect.min.x
                    && candidate.rect.min.y <= widget.rect.min.y
                    && candidate.rect.max.x >= widget.rect.max.x
                    && candidate.rect.max.y >= widget.rect.max.y
            })
            .or_else(|| {
                self.server
                    .inner
                    .widgets
                    .widget_list(viewport_id)
                    .into_iter()
                    .rev()
                    .find(|candidate| candidate.id == parent_id)
            });
        match parent {
            Some(parent) => self.widget_handle_json(pos, &parent),
            None => Ok(Value::Null),
        }
    }

    pub(super) fn widget_children(
        &self,
        pos: ScriptPosition,
        target: &Value,
    ) -> ScriptResult<Value> {
        let target =
            parse_widget_ref(target).map_err(|error| self.type_error(pos, error.message))?;
        let widget = self
            .server
            .inner
            .widgets
            .resolve_widget(&self.server.inner.viewports, None, &target)
            .map_err(|error| self.tool_error(pos, error.into_tmcp()))?;
        let viewport_id = self
            .server
            .inner
            .viewports
            .resolve_viewport_id(Some(widget.viewport_id.clone()))
            .map_err(|error| self.tool_error(pos, error.into_tmcp()))?;
        let widgets = self.server.inner.widgets.widget_list(viewport_id);
        let children = widgets
            .into_iter()
            .filter(|candidate| candidate.parent_id.as_deref() == Some(widget.id.as_str()))
            .collect::<Vec<_>>();
        self.widget_handle_list_json(pos, &children)
    }

    pub(super) fn raw_pointer_move(
        &self,
        pos: ScriptPosition,
        pos_arg: &Value,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let point = parse_pos2(pos_arg).map_err(|error| self.type_error(pos, error.message))?;
        let viewport_id = parse_optional_string(options, "viewport_id")
            .map_err(|error| self.type_error(pos, error.message))?;
        self.block_on_tool(pos, self.server.input_pointer_move(viewport_id, point))?;
        self.to_json(pos, ())
    }

    pub(super) fn raw_pointer_button(
        &self,
        pos: ScriptPosition,
        pos_arg: &Value,
        button: &Value,
        pressed: bool,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let point = parse_pos2(pos_arg).map_err(|error| self.type_error(pos, error.message))?;
        let button =
            parse_pointer_button(button).map_err(|error| self.type_error(pos, error.message))?;
        let viewport_id = parse_optional_string(options, "viewport_id")
            .map_err(|error| self.type_error(pos, error.message))?;
        let modifiers = self
            .modifiers_from_options(options)
            .map_err(|error| self.type_error(pos, error.message))?;
        self.block_on_tool(
            pos,
            self.server
                .input_pointer_button(viewport_id, point, button, pressed, modifiers),
        )?;
        self.to_json(pos, ())
    }

    pub(super) fn raw_key(
        &self,
        pos: ScriptPosition,
        key: String,
        pressed: bool,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let viewport_id = parse_optional_string(options, "viewport_id")
            .map_err(|error| self.type_error(pos, error.message))?;
        let modifiers = self
            .modifiers_from_options(options)
            .map_err(|error| self.type_error(pos, error.message))?;
        self.block_on_tool(
            pos,
            self.server.input_key(viewport_id, key, pressed, modifiers),
        )?;
        self.to_json(pos, ())
    }

    pub(super) fn raw_text(
        &self,
        pos: ScriptPosition,
        text: String,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let viewport_id = parse_optional_string(options, "viewport_id")
            .map_err(|error| self.type_error(pos, error.message))?;
        self.block_on_tool(pos, self.server.input_text(viewport_id, text))?;
        self.to_json(pos, ())
    }

    pub(super) fn raw_scroll(
        &self,
        pos: ScriptPosition,
        delta: &Value,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let delta = parse_vec2(delta).map_err(|error| self.type_error(pos, error.message))?;
        let viewport_id = parse_optional_string(options, "viewport_id")
            .map_err(|error| self.type_error(pos, error.message))?;
        let modifiers = self
            .modifiers_from_options(options)
            .map_err(|error| self.type_error(pos, error.message))?;
        self.block_on_tool(pos, self.server.input_scroll(viewport_id, delta, modifiers))?;
        self.to_json(pos, ())
    }

    pub(super) fn wait_for_widget_predicate<F>(
        &self,
        pos: ScriptPosition,
        target: &Value,
        options: Option<&Map<String, Value>>,
        mut predicate: F,
    ) -> ScriptResult<Value>
    where
        F: FnMut(Value) -> ScriptResult<bool>,
    {
        let target =
            parse_widget_ref(target).map_err(|error| self.type_error(pos, error.message))?;
        let (viewport_id, timeout_ms, poll_interval_ms) = self.parse_wait_options(pos, options)?;
        let timeout_ms = timeout_ms.unwrap_or(DEFAULT_WAIT_TIMEOUT_MS);
        let poll_interval_ms = poll_interval_ms.unwrap_or(DEFAULT_POLL_INTERVAL_MS);

        let result = self.handle.block_on(async {
            super::super::utils::wait_until_condition(
                &self.server.inner,
                timeout_ms,
                poll_interval_ms,
                Some(self.deadline),
                || {
                    let result = match self.server.inner.widgets.resolve_widget(
                        &self.server.inner.viewports,
                        viewport_id.as_deref(),
                        &target,
                    ) {
                        Ok(widget) => match self.widget_state_json(pos, &widget) {
                            Ok(widget_json) => match predicate(widget_json) {
                                Ok(matched) => Ok::<_, ScriptErrorInfo>((matched, Some(widget))),
                                Err(error) => Err(error),
                            },
                            Err(error) => Err(self.runtime_error(
                                pos,
                                format!("Failed to prepare widget state for predicate: {error:?}"),
                            )),
                        },
                        Err(error) if error.code == ErrorCode::NotFound => Ok((false, None)),
                        Err(error) => Err(self.tool_error(pos, error.into_tmcp())),
                    };
                    async move { result }
                },
            )
            .await
        });

        match result {
            Ok((matched, widget, elapsed_ms)) => {
                if !matched && self.deadline <= Instant::now() {
                    return Err(self.script_timeout_error(pos));
                }
                if matched {
                    self.to_json(pos, widget.as_ref().map(WidgetState::from))
                } else {
                    Err(self.tool_error(
                        pos,
                        ToolError::new(
                            ErrorCode::Timeout,
                            format!("Timed out waiting for widget predicate after {timeout_ms}ms"),
                        )
                        .with_details(wait_timeout_details(
                            "widget",
                            elapsed_ms,
                            widget.as_ref(),
                            None,
                            None,
                            None,
                        ))
                        .into_tmcp(),
                    ))
                }
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn wait_for_widget_visible(
        &self,
        pos: ScriptPosition,
        target: &Value,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let target =
            parse_widget_ref(target).map_err(|error| self.type_error(pos, error.message))?;
        let (viewport_id, timeout_ms, poll_interval_ms) = self.parse_wait_options(pos, options)?;
        let timeout_ms = timeout_ms.unwrap_or(DEFAULT_WAIT_TIMEOUT_MS);
        let poll_interval_ms = poll_interval_ms.unwrap_or(DEFAULT_POLL_INTERVAL_MS);

        let result = self.handle.block_on(async {
            super::super::utils::wait_until_condition(
                &self.server.inner,
                timeout_ms,
                poll_interval_ms,
                Some(self.deadline),
                || {
                    let result = match self.server.inner.widgets.resolve_widget(
                        &self.server.inner.viewports,
                        viewport_id.as_deref(),
                        &target,
                    ) {
                        Ok(widget) => Ok::<_, ScriptErrorInfo>((widget.visible, Some(widget))),
                        Err(error) if error.code == ErrorCode::NotFound => Ok((false, None)),
                        Err(error) => Err(self.tool_error(pos, error.into_tmcp())),
                    };
                    async move { result }
                },
            )
            .await
        });

        match result {
            Ok((matched, widget, elapsed_ms)) => {
                if !matched && self.deadline <= Instant::now() {
                    return Err(self.script_timeout_error(pos));
                }
                if matched {
                    if let Some(widget) = widget.as_ref() {
                        self.widget_state_json(pos, widget)
                    } else {
                        Err(self.runtime_error(
                            pos,
                            "wait_for_widget_visible matched without a widget snapshot",
                        ))
                    }
                } else {
                    Err(self.tool_error(
                        pos,
                        ToolError::new(
                            ErrorCode::Timeout,
                            format!(
                                "Timed out waiting for widget visibility predicate after {timeout_ms}ms"
                            ),
                        )
                        .with_details(wait_timeout_details(
                            "widget_visible",
                            elapsed_ms,
                            widget.as_ref(),
                            None,
                            None,
                            None,
                        ))
                        .into_tmcp(),
                    ))
                }
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn wait_for_widget_absent(
        &self,
        pos: ScriptPosition,
        target: &Value,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let target =
            parse_widget_ref(target).map_err(|error| self.type_error(pos, error.message))?;
        let (viewport_id, timeout_ms, poll_interval_ms) = self.parse_wait_options(pos, options)?;
        let timeout_ms = timeout_ms.unwrap_or(DEFAULT_WAIT_TIMEOUT_MS);
        let poll_interval_ms = poll_interval_ms.unwrap_or(DEFAULT_POLL_INTERVAL_MS);

        let result = self.handle.block_on(async {
            super::super::utils::wait_until_condition(
                &self.server.inner,
                timeout_ms,
                poll_interval_ms,
                Some(self.deadline),
                || {
                    let result = match self.server.inner.widgets.resolve_widget(
                        &self.server.inner.viewports,
                        viewport_id.as_deref(),
                        &target,
                    ) {
                        Ok(widget) => Ok::<_, ScriptErrorInfo>((false, Some(widget))),
                        Err(error) if error.code == ErrorCode::NotFound => Ok((true, None)),
                        Err(error) => Err(self.tool_error(pos, error.into_tmcp())),
                    };
                    async move { result }
                },
            )
            .await
        });

        match result {
            Ok((matched, _widget, elapsed_ms)) => {
                if !matched && self.deadline <= Instant::now() {
                    return Err(self.script_timeout_error(pos));
                }
                if matched {
                    Ok(Value::Null)
                } else {
                    Err(self.tool_error(
                        pos,
                        ToolError::new(
                            ErrorCode::Timeout,
                            format!("Timed out waiting for widget absence after {timeout_ms}ms"),
                        )
                        .with_details(wait_timeout_details(
                            "widget_absent",
                            elapsed_ms,
                            None,
                            None,
                            None,
                            None,
                        ))
                        .into_tmcp(),
                    ))
                }
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn wait_for_frames(
        &self,
        pos: ScriptPosition,
        count: &Value,
    ) -> ScriptResult<Value> {
        let count =
            parse_optional_u64_val(count).map_err(|error| self.type_error(pos, error.message))?;
        let timeout_ms = self.configured_timeout_ms();
        let result =
            self.block_on_tool(pos, self.server.wait_for_frame_count(count, timeout_ms))?;
        self.to_json(pos, result)
    }

    pub(super) fn wait_for_settle(
        &self,
        pos: ScriptPosition,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<Value> {
        let (viewport_id, timeout_ms, poll_interval_ms) = self.parse_wait_options(pos, options)?;
        self.block_on_tool(
            pos,
            self.server
                .wait_for_settle(viewport_id, timeout_ms, poll_interval_ms),
        )?;
        self.to_json(pos, ())
    }

    pub(super) fn wait_for_viewport_predicate<F>(
        &self,
        pos: ScriptPosition,
        options: Option<&Map<String, Value>>,
        mut predicate: F,
    ) -> ScriptResult<Value>
    where
        F: FnMut(Value) -> ScriptResult<bool>,
    {
        let (viewport_id, timeout_ms, poll_interval_ms) = self.parse_wait_options(pos, options)?;
        let timeout_ms = timeout_ms.unwrap_or(DEFAULT_WAIT_TIMEOUT_MS);
        let poll_interval_ms = poll_interval_ms.unwrap_or(DEFAULT_POLL_INTERVAL_MS);
        let viewport_id = self
            .server
            .inner
            .viewports
            .resolve_viewport_id(viewport_id)
            .map_err(|error| self.tool_error(pos, error.into_tmcp()))?;

        let result = self.handle.block_on(async {
            super::super::utils::wait_until_condition(
                &self.server.inner,
                timeout_ms,
                poll_interval_ms,
                Some(self.deadline),
                || {
                    let result = match viewport_snapshot_for(&self.server.inner, viewport_id) {
                        Some(snapshot) => match self.viewport_state_json(pos, &snapshot) {
                            Ok(viewport_json) => match predicate(viewport_json) {
                                Ok(matched) => Ok::<_, ScriptErrorInfo>((matched, Some(snapshot))),
                                Err(error) => Err(error),
                            },
                            Err(error) => Err(self.runtime_error(
                                pos,
                                format!(
                                    "Failed to prepare viewport state for predicate: {error:?}"
                                ),
                            )),
                        },
                        None => Err(self.tool_error(
                            pos,
                            ToolError::new(ErrorCode::InvalidRef, "Viewport not ready for wait")
                                .into_tmcp(),
                        )),
                    };
                    async move { result }
                },
            )
            .await
        });

        match result {
            Ok((matched, viewport, elapsed_ms)) => {
                if !matched && self.deadline <= Instant::now() {
                    return Err(self.script_timeout_error(pos));
                }
                let viewport = viewport
                    .ok_or_else(|| self.runtime_error(pos, "Viewport not ready for wait"))?;
                if matched {
                    self.viewport_state_json(pos, &viewport)
                } else {
                    Err(self.tool_error(
                        pos,
                        ToolError::new(
                            ErrorCode::Timeout,
                            format!(
                                "Timed out waiting for viewport predicate after {timeout_ms}ms"
                            ),
                        )
                        .with_details(wait_timeout_details(
                            "viewport",
                            elapsed_ms,
                            None,
                            Some(&viewport),
                            None,
                            None,
                        ))
                        .into_tmcp(),
                    ))
                }
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn screenshot(
        &self,
        pos: ScriptPosition,
        target: Option<&Value>,
    ) -> ScriptResult<Value> {
        let mut viewport_id = None;
        let mut widget_target: Option<WidgetRef> = None;
        if let Some(target) = target {
            if let Some(map) = target.as_object() {
                let has_target = map_has_any(map, &["id"]);
                if has_target {
                    widget_target = Some(
                        parse_widget_ref(target)
                            .map_err(|error| self.type_error(pos, error.message))?,
                    );
                    viewport_id = parse_optional_string(Some(map), "viewport_id")
                        .map_err(|error| self.type_error(pos, error.message))?;
                } else if map_has_any(map, &["viewport_id"]) {
                    viewport_id = parse_optional_string(Some(map), "viewport_id")
                        .map_err(|error| self.type_error(pos, error.message))?;
                } else {
                    return Err(
                        self.type_error(pos, "screenshot expects a WidgetRef or viewport_id")
                    );
                }
            } else {
                widget_target = Some(
                    parse_widget_ref(target)
                        .map_err(|error| self.type_error(pos, error.message))?,
                );
            }
        }

        let id = self.next_image_id();
        if let Some(target) = widget_target {
            let (widget, viewport_id_resolved) =
                resolve_widget_and_viewport(&self.server.inner, viewport_id.as_deref(), &target)
                    .map_err(|error| self.tool_error(pos, error.into_tmcp()))?;
            let pixels_per_point = self
                .server
                .inner
                .viewports
                .input_snapshot(viewport_id_resolved)
                .map(|snapshot| snapshot.pixels_per_point)
                .unwrap_or(1.0);
            let data = self
                .handle
                .block_on(capture_screenshot(
                    &self.server.inner,
                    viewport_id_resolved,
                    ScreenshotKind::Widget {
                        rect: widget.interact_rect,
                        pixels_per_point,
                    },
                ))
                .map_err(|error| self.tool_error(pos, error.into_tmcp()))?;
            self.store_image(ImageCapture {
                id: id.clone(),
                data,
                kind: ScriptImageKind::Widget,
                viewport_id: viewport_id_to_string(viewport_id_resolved),
                target: Some(target),
                rect: Some(widget.interact_rect),
            });
            return Ok(image_ref_json(id));
        }

        let viewport_id_resolved = resolve_screenshot_viewport(&self.server.inner, viewport_id)
            .map_err(|error| self.tool_error(pos, error.into_tmcp()))?;
        let data = self
            .handle
            .block_on(capture_screenshot(
                &self.server.inner,
                viewport_id_resolved,
                ScreenshotKind::Viewport,
            ))
            .map_err(|error| self.tool_error(pos, error.into_tmcp()))?;
        self.store_image(ImageCapture {
            id: id.clone(),
            data,
            kind: ScriptImageKind::Viewport,
            viewport_id: viewport_id_to_string(viewport_id_resolved),
            target: None,
            rect: None,
        });
        Ok(image_ref_json(id))
    }

    pub(super) fn check_layout(
        &self,
        pos: ScriptPosition,
        viewport_id: Option<String>,
    ) -> ScriptResult<Value> {
        let result = self.block_on_tool(pos, self.server.check_layout(viewport_id, None))?;
        self.to_json(pos, result)
    }

    pub(super) fn check_layout_widget(
        &self,
        pos: ScriptPosition,
        target: &Value,
        viewport_id: Option<String>,
    ) -> ScriptResult<Value> {
        let target =
            parse_widget_ref(target).map_err(|error| self.type_error(pos, error.message))?;
        let result =
            self.block_on_tool(pos, self.server.check_layout(viewport_id, Some(target)))?;
        self.to_json(pos, result)
    }

    /// Show a highlight on a widget (by target) or a rect with a mandatory color.
    pub(super) fn show_highlight_widget(
        &self,
        pos: ScriptPosition,
        target: &Value,
        viewport_id: Option<String>,
        color: String,
    ) -> ScriptResult<Value> {
        let target =
            parse_widget_ref(target).map_err(|error| self.type_error(pos, error.message))?;
        let result = self.block_on_tool(
            pos,
            self.server
                .show_highlight(viewport_id, Some(target), None, color),
        )?;
        self.to_json(pos, result)
    }

    /// Show a highlight on a rect with a mandatory color.
    pub(super) fn show_highlight_rect(
        &self,
        pos: ScriptPosition,
        viewport_id: Option<String>,
        rect: Rect,
        color: String,
    ) -> ScriptResult<Value> {
        let result = self.block_on_tool(
            pos,
            self.server
                .show_highlight(viewport_id, None, Some(rect), color),
        )?;
        self.to_json(pos, result)
    }

    /// Hide a widget's highlight.
    pub(super) fn hide_highlight_widget(
        &self,
        pos: ScriptPosition,
        target: &Value,
        viewport_id: Option<String>,
    ) -> ScriptResult<Value> {
        let target =
            parse_widget_ref(target).map_err(|error| self.type_error(pos, error.message))?;
        self.block_on_tool(pos, self.server.hide_highlight(viewport_id, Some(target)))?;
        Ok(Value::Null)
    }

    /// Clear all highlights.
    pub(super) fn hide_highlight_all(&self, pos: ScriptPosition) -> ScriptResult<Value> {
        self.block_on_tool(pos, self.server.hide_highlight(None, None))?;
        Ok(Value::Null)
    }

    pub(super) fn show_debug_overlay(
        &self,
        pos: ScriptPosition,
        viewport_id: Option<String>,
        mode: Option<&Value>,
        options: Option<&Map<String, Value>>,
        scope: Option<WidgetRef>,
    ) -> ScriptResult<Value> {
        let mode = match mode {
            None => None,
            Some(value) => Some(
                parse_overlay_mode(value).map_err(|error| self.type_error(pos, error.message))?,
            ),
        };
        let options = options
            .map(|map| {
                Ok::<_, ScriptErrorInfo>(OverlayDebugOptionsInput {
                    show_labels: parse_optional_bool(Some(map), "show_labels")?,
                    show_sizes: parse_optional_bool(Some(map), "show_sizes")?,
                    label_font_size: parse_optional_f32(Some(map), "label_font_size")?,
                    bounds_color: parse_optional_string(Some(map), "bounds_color")?,
                    clip_color: parse_optional_string(Some(map), "clip_color")?,
                    overlap_color: parse_optional_string(Some(map), "overlap_color")?,
                })
            })
            .transpose()
            .map_err(|error| self.type_error(pos, error.message))?;
        self.block_on_tool(
            pos,
            self.server
                .show_debug_overlay(viewport_id, mode, scope, options),
        )?;
        self.to_json(pos, ())
    }

    pub(super) fn hide_debug_overlay(&self, pos: ScriptPosition) -> ScriptResult<Value> {
        self.block_on_tool(pos, self.server.hide_debug_overlay())?;
        self.to_json(pos, ())
    }

    pub(super) fn viewport_set_inner_size(
        &self,
        pos: ScriptPosition,
        size: &Value,
        viewport_id: Option<String>,
    ) -> ScriptResult<Value> {
        let size = parse_vec2(size).map_err(|error| self.type_error(pos, error.message))?;
        self.block_on_tool(pos, self.server.viewport_set_inner_size(viewport_id, size))?;
        self.to_json(pos, ())
    }

    pub(super) fn viewport_set_resize_options(
        &self,
        pos: ScriptPosition,
        options: Option<&Map<String, Value>>,
        viewport_id: Option<String>,
    ) -> ScriptResult<Value> {
        let min_size = parse_optional_vec2(options, "min_size")
            .map_err(|error| self.type_error(pos, error.message))?;
        let max_size = parse_optional_vec2(options, "max_size")
            .map_err(|error| self.type_error(pos, error.message))?;
        let increments = parse_optional_vec2(options, "increments")
            .map_err(|error| self.type_error(pos, error.message))?;
        let resizable = parse_optional_bool(options, "resizable")
            .map_err(|error| self.type_error(pos, error.message))?;
        self.block_on_tool(
            pos,
            self.server.viewport_set_resize_options(
                viewport_id,
                min_size,
                max_size,
                increments,
                resizable,
            ),
        )?;
        self.to_json(pos, ())
    }

    pub(super) fn focus_window(
        &self,
        pos: ScriptPosition,
        viewport: String,
    ) -> ScriptResult<Value> {
        self.block_on_tool(pos, self.server.focus_window(viewport))?;
        self.to_json(pos, ())
    }

    pub(super) fn fixture(&self, pos: ScriptPosition, name: String) -> ScriptResult<Value> {
        let timeout_ms = self.configured_timeout_ms();
        self.block_on_tool(pos, self.server.fixture(name, timeout_ms))?;
        self.settle_after_action(pos, None, None)?;
        self.to_json(pos, ())
    }

    pub(super) fn fixtures(&self, pos: ScriptPosition) -> ScriptResult<Value> {
        self.to_json(pos, self.server.inner.fixtures.fixtures_sorted())
    }

    fn assert_result(
        &self,
        pos: ScriptPosition,
        passed: bool,
        message: String,
    ) -> ScriptResult<()> {
        self.record_assertion(passed, message.clone(), pos);
        if passed {
            Ok(())
        } else {
            Err(self.assertion_error(pos, message))
        }
    }

    pub(super) fn assert_condition(
        &self,
        pos: ScriptPosition,
        condition: bool,
        message: Option<String>,
    ) -> ScriptResult<()> {
        let message = message.unwrap_or_else(|| {
            if condition {
                "assertion passed".to_string()
            } else {
                "assertion failed".to_string()
            }
        });
        self.assert_result(pos, condition, message)
    }

    pub(super) fn assert_widget_exists(
        &self,
        pos: ScriptPosition,
        target: &Value,
        options: Option<&Map<String, Value>>,
    ) -> ScriptResult<()> {
        let target =
            parse_widget_ref(target).map_err(|error| self.type_error(pos, error.message))?;
        let (viewport_id, timeout_ms, poll_interval_ms) = self.parse_wait_options(pos, options)?;
        self.block_on_tool(
            pos,
            self.server.wait_for_widget_state(
                viewport_id,
                target,
                timeout_ms,
                poll_interval_ms,
                |widget| widget.is_some(),
            ),
        )?;
        self.assert_result(pos, true, "widget exists".to_string())
    }
}

fn image_ref_json(id: String) -> Value {
    serde_json::json!({
        "type": "image_ref",
        "id": id,
    })
}
