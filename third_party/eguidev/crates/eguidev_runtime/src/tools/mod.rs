//! Internal egui automation helpers plus the script-first MCP facade.
//!
//! The supported embedded MCP surface is intentionally narrow: `script_eval`
//! and `script_api`. The broader helper set in this module exists to support
//! Luau scripts and internal testing, not as additional top-level MCP tools.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, atomic::Ordering},
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use image::codecs::jpeg::JpegEncoder;
use serde::Serialize;
use serde_json::{Value, json};
use tmcp::{
    ServerCtx, ToolResult, mcp_server,
    schema::{CallToolResult, ClientCapabilities, Implementation, InitializeResult},
    tool,
};
use tokio::{runtime::Handle, task::spawn_blocking, time::timeout};

use crate::{
    actions::{ActionTiming, InputAction},
    overlay::{
        OverlayDebugConfig, OverlayDebugMode, OverlayDebugOptions, OverlayEntry, parse_color,
    },
    registry::{Inner, viewport_id_to_string},
    runtime::Runtime,
    screenshots::{ScreenshotKind, ScreenshotState},
    script_definitions,
    tree::collect_subtree,
    types::{
        Anchor, AnchorCheck, FixtureSpec, Modifiers, Pos2, Rect, RoleState, Vec2, WidgetRef,
        WidgetRegistryEntry, WidgetRole, WidgetState, WidgetValue,
    },
    viewports::ViewportSnapshot,
};

pub const DEFAULT_WAIT_TIMEOUT_MS: u64 = 5_000;
pub const DEFAULT_POLL_INTERVAL_MS: u64 = 16;
const SCROLL_STABILITY_TOLERANCE: f32 = 0.75;
mod layout;
mod results;
pub mod script;
mod types;
mod utils;

use layout::*;
use results::*;
pub use script::{
    ScriptArgValue, ScriptArgs, ScriptAssertion, ScriptErrorInfo, ScriptEvalOptions,
    ScriptEvalOutcome, ScriptEvalRequest, ScriptImageInfo, ScriptLocation, ScriptTiming,
};
use types::{OverlayDebugModeName, OverlayDebugOptionsInput, PointerButtonName, ScrollAlign};
use utils::{parse_key_combo, resolve_key_name, *};

pub use crate::error::*;

pub const DEFAULT_SCRIPT_EVAL_TIMEOUT_MS: u64 = script::DEFAULT_SCRIPT_TIMEOUT_MS;

fn scroll_state(widget: &WidgetRegistryEntry) -> Option<crate::ScrollAreaMeta> {
    widget.role_state.as_ref().and_then(RoleState::scroll_state)
}

#[derive(Debug, Clone, Serialize)]
struct AnchorStatus {
    widget_id: String,
    viewport_id: Option<String>,
    check: String,
    satisfied: bool,
    detail: String,
    current_state: Option<WidgetState>,
}

#[derive(Debug, Clone, Serialize)]
struct AnchorEvaluationSnapshot {
    fixture: String,
    statuses: Vec<AnchorStatus>,
}

#[derive(Debug, Clone, Copy)]
struct ScrollSample {
    frame_count: u64,
    max_offset: Vec2,
    stabilized: bool,
}

fn reveal_axis(
    current: f32,
    viewport_min: f32,
    viewport_size: f32,
    item_min: f32,
    item_max: f32,
    max_offset: f32,
) -> f32 {
    let content_min = current + (item_min - viewport_min);
    let content_max = current + (item_max - viewport_min);
    let mut target = current;
    if content_min < current {
        target = content_min;
    } else if content_max > current + viewport_size {
        target = content_max - viewport_size;
    }
    target.clamp(0.0, max_offset.max(0.0))
}

fn scroll_area_target_offset(
    scroll_area: &WidgetRegistryEntry,
    target: &WidgetRegistryEntry,
) -> Option<Vec2> {
    let scroll = scroll_state(scroll_area)?;
    let current: egui::Vec2 = scroll.offset.into();
    Some(Vec2 {
        x: reveal_axis(
            current.x,
            scroll_area.rect.min.x,
            scroll.viewport_size.x,
            target.rect.min.x,
            target.rect.max.x,
            scroll.max_offset.x,
        ),
        y: reveal_axis(
            current.y,
            scroll_area.rect.min.y,
            scroll.viewport_size.y,
            target.rect.min.y,
            target.rect.max.y,
            scroll.max_offset.y,
        ),
    })
}

fn approx_eq_vec2(left: Vec2, right: Vec2, tolerance: f32) -> bool {
    (left.x - right.x).abs() <= tolerance && (left.y - right.y).abs() <= tolerance
}

fn anchor_target(anchor: &Anchor) -> WidgetRef {
    WidgetRef {
        id: Some(anchor.widget_id.clone()),
        viewport_id: anchor.viewport_id.clone(),
    }
}

fn anchor_status(
    anchor: &Anchor,
    satisfied: bool,
    detail: impl Into<String>,
    current_state: Option<WidgetState>,
) -> AnchorStatus {
    AnchorStatus {
        widget_id: anchor.widget_id.clone(),
        viewport_id: anchor.viewport_id.clone(),
        check: anchor.check.to_string(),
        satisfied,
        detail: detail.into(),
        current_state,
    }
}

fn format_anchor_status(status: &AnchorStatus) -> String {
    let marker = if status.satisfied { "✓" } else { "✗" };
    let target = match &status.viewport_id {
        Some(viewport_id) => format!("{} in {}", status.widget_id, viewport_id),
        None => status.widget_id.clone(),
    };
    format!("{marker} {target} {} — {}", status.check, status.detail)
}

fn update_scroll_stability(
    scroll_samples: &Mutex<HashMap<(String, String), ScrollSample>>,
    widget: &WidgetRegistryEntry,
    current_sample: ScrollSample,
) -> bool {
    let key = (widget.viewport_id.clone(), widget.id.clone());
    let mut samples = scroll_samples
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match samples.get_mut(&key) {
        Some(previous) => {
            let stabilized = if previous.stabilized {
                approx_eq_vec2(
                    previous.max_offset,
                    current_sample.max_offset,
                    SCROLL_STABILITY_TOLERANCE,
                )
            } else {
                previous.frame_count != current_sample.frame_count
                    && approx_eq_vec2(
                        previous.max_offset,
                        current_sample.max_offset,
                        SCROLL_STABILITY_TOLERANCE,
                    )
            };
            *previous = ScrollSample {
                stabilized,
                ..current_sample
            };
            stabilized
        }
        None => {
            samples.insert(
                key,
                ScrollSample {
                    stabilized: false,
                    ..current_sample
                },
            );
            false
        }
    }
}

fn resolve_viewport_id(
    inner: &Inner,
    viewport_id: Option<String>,
) -> Result<egui::ViewportId, ToolError> {
    inner
        .viewports
        .resolve_viewport_id(viewport_id)
        .map_err(Into::into)
}

fn resolve_widget(
    inner: &Inner,
    viewport_id: Option<&str>,
    target: &WidgetRef,
) -> Result<WidgetRegistryEntry, ToolError> {
    inner
        .widgets
        .resolve_widget(&inner.viewports, viewport_id, target)
        .map_err(Into::into)
}

pub struct DevMcpServer {
    inner: Arc<Inner>,
    runtime: Arc<Runtime>,
}

pub fn collect_widget_list(
    inner: &Inner,
    viewport_id: Option<String>,
    include_invisible: Option<bool>,
    role: Option<WidgetRole>,
    id_prefix: Option<&str>,
) -> ToolResult<Vec<WidgetRegistryEntry>> {
    ensure_automation_ready(inner)?;
    let viewport_id = resolve_viewport_id(inner, viewport_id)?;
    let mut widgets = inner.widgets.widget_list(viewport_id);
    let include_invisible = include_invisible.unwrap_or(false);
    if !include_invisible {
        widgets.retain(|entry| entry.visible);
    }
    if let Some(role) = role {
        widgets.retain(|entry| entry.role == role);
    }

    if let Some(prefix) = id_prefix {
        widgets.retain(|entry| entry.id.starts_with(prefix));
    }

    Ok(widgets)
}

#[mcp_server(initialize_fn = initialize)]
impl DevMcpServer {
    #[cfg(test)]
    pub(crate) fn new(inner: Arc<Inner>) -> Self {
        let runtime = Runtime::ensure_for_inner(&inner);
        Self { inner, runtime }
    }

    pub(crate) fn with_runtime(inner: Arc<Inner>, runtime: Arc<Runtime>) -> Self {
        Self { inner, runtime }
    }

    async fn resolve_widget_for_pointer(
        &self,
        viewport_id: Option<String>,
        target: &WidgetRef,
    ) -> ToolResult<(WidgetRegistryEntry, egui::ViewportId)> {
        let (widget, viewport_id) =
            resolve_widget_and_viewport(&self.inner, viewport_id.as_deref(), target)?;
        if widget.visible {
            return Ok((widget, viewport_id));
        }

        let viewport_id = viewport_id_to_string(viewport_id);
        let wait_result = self
            .wait_for_widget_state(
                Some(viewport_id.clone()),
                target.clone(),
                None,
                None,
                |widget| widget.is_some_and(|widget| widget.visible),
            )
            .await?;
        if wait_result.is_none() {
            return Err(ToolError::new(ErrorCode::NotFound, "Widget is not visible").into());
        }

        let (widget, viewport_id) =
            resolve_widget_and_viewport(&self.inner, Some(&viewport_id), target)?;
        if !widget.visible {
            return Err(ToolError::new(ErrorCode::NotFound, "Widget is not visible").into());
        }
        Ok((widget, viewport_id))
    }

    async fn fixture_apply_internal(&self, name: &str) -> Result<(FixtureSpec, u64), ToolError> {
        let Some(spec) = self.inner.fixtures.fixture(name) else {
            return Err(ToolError::new(
                ErrorCode::NotFound,
                format!("Unknown fixture: {name}"),
            ));
        };

        self.inner.clear_all();
        self.inner.reset_fixture_transient_state();
        let fixture_epoch = self.inner.begin_fixture_epoch();
        let result = self.inner.apply_fixture(name);
        self.inner.reset_fixture_transient_state();
        if let Err(message) = result {
            return Err(ToolError::new(ErrorCode::Internal, message));
        }
        Ok((spec, fixture_epoch))
    }

    fn evaluate_anchor(
        &self,
        anchor: &Anchor,
        fixture_epoch: u64,
        scroll_samples: &Mutex<HashMap<(String, String), ScrollSample>>,
    ) -> Result<AnchorStatus, ToolError> {
        let target = anchor_target(anchor);
        let widget = match resolve_widget(&self.inner, None, &target) {
            Ok(widget) => widget,
            Err(error) if error.code == ErrorCode::NotFound => {
                return Ok(anchor_status(anchor, false, "widget not found", None));
            }
            Err(error) => return Err(error),
        };

        let current_state = WidgetState::from(&widget);
        let viewport_id = match self
            .inner
            .viewports
            .resolve_viewport_id(Some(widget.viewport_id.clone()))
        {
            Ok(viewport_id) => viewport_id,
            Err(_) => {
                return Ok(anchor_status(
                    anchor,
                    false,
                    "viewport has no captured context yet",
                    Some(current_state),
                ));
            }
        };
        let Some(capture) = self.inner.viewports.capture_snapshot(viewport_id) else {
            return Ok(anchor_status(
                anchor,
                false,
                "viewport has no captured snapshot yet",
                Some(current_state),
            ));
        };
        if capture.fixture_epoch < fixture_epoch {
            return Ok(anchor_status(
                anchor,
                false,
                format!(
                    "waiting for post-fixture capture (current epoch {}, need {})",
                    capture.fixture_epoch, fixture_epoch
                ),
                Some(current_state),
            ));
        }

        let status = match &anchor.check {
            AnchorCheck::Visible => anchor_status(
                anchor,
                widget.visible,
                if widget.visible {
                    "widget is visible".to_string()
                } else {
                    "widget exists but is not visible".to_string()
                },
                Some(current_state),
            ),
            AnchorCheck::Label(expected) => {
                let actual = widget.label.as_deref().unwrap_or("");
                anchor_status(
                    anchor,
                    widget.label.as_deref() == Some(expected.as_str()),
                    format!("current label: {actual:?}"),
                    Some(current_state),
                )
            }
            AnchorCheck::Value(expected) => {
                let matched = widget.value.as_ref() == Some(expected);
                let actual = widget
                    .value
                    .as_ref()
                    .map(WidgetValue::to_text)
                    .unwrap_or_default();
                anchor_status(
                    anchor,
                    matched,
                    format!("current value: {actual:?}"),
                    Some(current_state),
                )
            }
            AnchorCheck::ScrollReady => {
                let Some(scroll) = scroll_state(&widget) else {
                    return Ok(anchor_status(
                        anchor,
                        false,
                        "widget has no scroll_state yet",
                        Some(current_state),
                    ));
                };
                if scroll.max_offset.y <= 0.0 {
                    return Ok(anchor_status(
                        anchor,
                        false,
                        format!(
                            "scroll max_offset is not ready: ({:.1}, {:.1})",
                            scroll.max_offset.x, scroll.max_offset.y
                        ),
                        Some(current_state),
                    ));
                }
                let current_sample = ScrollSample {
                    frame_count: capture.frame_count,
                    max_offset: scroll.max_offset,
                    stabilized: false,
                };
                if update_scroll_stability(scroll_samples, &widget, current_sample) {
                    anchor_status(
                        anchor,
                        true,
                        format!(
                            "scroll stabilized at max_offset ({:.1}, {:.1})",
                            scroll.max_offset.x, scroll.max_offset.y
                        ),
                        Some(current_state),
                    )
                } else {
                    anchor_status(
                        anchor,
                        false,
                        "waiting for scroll initialization to stabilize".to_string(),
                        Some(current_state),
                    )
                }
            }
            AnchorCheck::ScrollAt { offset, tolerance } => {
                let Some(scroll) = scroll_state(&widget) else {
                    return Ok(anchor_status(
                        anchor,
                        false,
                        "widget has no scroll_state yet",
                        Some(current_state),
                    ));
                };
                if scroll.max_offset.y <= 0.0 {
                    return Ok(anchor_status(
                        anchor,
                        false,
                        format!(
                            "scroll max_offset is not ready: ({:.1}, {:.1})",
                            scroll.max_offset.x, scroll.max_offset.y
                        ),
                        Some(current_state),
                    ));
                }
                let current_sample = ScrollSample {
                    frame_count: capture.frame_count,
                    max_offset: scroll.max_offset,
                    stabilized: false,
                };
                let stable = update_scroll_stability(scroll_samples, &widget, current_sample);
                if !stable {
                    return Ok(anchor_status(
                        anchor,
                        false,
                        "waiting for scroll initialization to stabilize".to_string(),
                        Some(current_state),
                    ));
                }
                let matched = approx_eq_vec2(scroll.offset, *offset, *tolerance);
                anchor_status(
                    anchor,
                    matched,
                    format!(
                        "current offset: ({:.1}, {:.1})",
                        scroll.offset.x, scroll.offset.y
                    ),
                    Some(current_state),
                )
            }
        };
        Ok(status)
    }

    async fn evaluate_anchors(
        &self,
        spec: &FixtureSpec,
        fixture_epoch: u64,
        timeout_ms: u64,
    ) -> Result<(), ToolError> {
        let scroll_samples = Arc::new(Mutex::new(HashMap::new()));
        let (matched, state, elapsed_ms) = wait_until_condition(
            &self.inner,
            timeout_ms,
            DEFAULT_POLL_INTERVAL_MS,
            None,
            || async {
                self.inner.request_repaint_all();
                let mut statuses = Vec::with_capacity(spec.anchors.len());
                for anchor in &spec.anchors {
                    statuses.push(self.evaluate_anchor(
                        anchor,
                        fixture_epoch,
                        scroll_samples.as_ref(),
                    )?);
                }
                let matched = statuses.iter().all(|status| status.satisfied);
                Ok::<_, ToolError>((
                    matched,
                    Some(AnchorEvaluationSnapshot {
                        fixture: spec.name.clone(),
                        statuses,
                    }),
                ))
            },
        )
        .await?;

        if matched {
            return Ok(());
        }

        let snapshot = state.unwrap_or_else(|| AnchorEvaluationSnapshot {
            fixture: spec.name.clone(),
            statuses: Vec::new(),
        });
        let status_lines = snapshot
            .statuses
            .iter()
            .map(format_anchor_status)
            .collect::<Vec<_>>();
        let message = if status_lines.is_empty() {
            format!("fixture \"{}\" timed out after {timeout_ms}ms", spec.name)
        } else {
            format!(
                "fixture \"{}\" timed out after {timeout_ms}ms\n{}",
                spec.name,
                status_lines.join("\n")
            )
        };
        Err(
            ToolError::new(ErrorCode::Timeout, message).with_details(json!({
                "fixture": spec.name,
                "elapsed_ms": elapsed_ms,
                "statuses": snapshot.statuses,
            })),
        )
    }

    async fn initialize(
        &self,
        _context: &ServerCtx,
        _protocol_version: String,
        _capabilities: ClientCapabilities,
        _client_info: Implementation,
    ) -> tmcp::Result<InitializeResult> {
        let version = env!("CARGO_PKG_VERSION").to_string();
        Ok(InitializeResult::new("eguidev")
            .with_version(version)
            .with_tools(true))
    }

    /// List viewports and their properties.
    async fn viewports_list(
        &self,
        viewport_id: Option<String>,
    ) -> ToolResult<Vec<ViewportSnapshot>> {
        let mut viewports = self.inner.viewports.viewports_snapshot();
        if let Some(filter) = viewport_id {
            viewports.retain(|entry| entry.viewport_id == filter);
        }
        Ok(viewports)
    }

    /// Inject a pointer move event (positions are in egui points).
    async fn input_pointer_move(&self, viewport_id: Option<String>, pos: Pos2) -> ToolResult<()> {
        let viewport_id = resolve_viewport_id(&self.inner, viewport_id)?;
        self.inner
            .queue_action(viewport_id, InputAction::PointerMove { pos });
        Ok(())
    }

    /// Inject a pointer button press or release (positions are in egui points).
    ///
    /// Common click sequence:
    /// 1. `input_pointer_move` to the target position.
    /// 2. `input_pointer_button` with `pressed: true`.
    /// 3. `input_pointer_button` with `pressed: false`.
    async fn input_pointer_button(
        &self,
        viewport_id: Option<String>,
        pos: Pos2,
        button: PointerButtonName,
        pressed: bool,
        modifiers: Option<Modifiers>,
    ) -> ToolResult<()> {
        let viewport_id = resolve_viewport_id(&self.inner, viewport_id)?;
        let pointer_button = button
            .to_pointer_button()
            .ok_or_else(|| ToolError::new(ErrorCode::InvalidRef, "Invalid pointer button"))?;
        let modifiers = modifiers.unwrap_or_default();
        self.inner.queue_action(
            viewport_id,
            InputAction::PointerButton {
                pos,
                button: pointer_button,
                pressed,
                modifiers,
            },
        );
        Ok(())
    }

    /// Inject a raw key event.
    async fn input_key(
        &self,
        viewport_id: Option<String>,
        key: String,
        pressed: bool,
        modifiers: Option<Modifiers>,
    ) -> ToolResult<()> {
        let viewport_id = resolve_viewport_id(&self.inner, viewport_id)?;
        let parsed_key = resolve_key_name(&key)
            .ok_or_else(|| ToolError::new(ErrorCode::InvalidRef, format!("Unknown key: {key}")))?;
        let modifiers = modifiers.unwrap_or_default();
        self.inner.queue_action(
            viewport_id,
            InputAction::Key {
                key: parsed_key,
                pressed,
                modifiers,
            },
        );
        Ok(())
    }

    /// Press and release a key (optionally repeating), with modifiers.
    ///
    /// `key_name` is the original user-provided key name string, used to derive the text event
    /// (preserving case for single characters like `"a"` vs `"A"`).
    async fn action_key(
        &self,
        viewport_id: Option<String>,
        key: egui::Key,
        modifiers: Modifiers,
        key_name: &str,
        repeat: Option<u32>,
    ) -> ToolResult<()> {
        let viewport_id = resolve_viewport_id(&self.inner, viewport_id)?;
        let repeat = repeat.unwrap_or(1);
        if repeat == 0 {
            return Err(ToolError::new(ErrorCode::InvalidRef, "Repeat must be at least 1").into());
        }
        let text = if modifiers.ctrl || modifiers.command || modifiers.alt {
            None
        } else {
            printable_key_text(key_name)
        };
        for _ in 0..repeat {
            self.inner.queue_action(
                viewport_id,
                InputAction::Key {
                    key,
                    pressed: true,
                    modifiers,
                },
            );
            if let Some(text) = text.as_deref() {
                self.inner.queue_action(
                    viewport_id,
                    InputAction::Text {
                        text: text.to_string(),
                    },
                );
            }
            self.inner.queue_action(
                viewport_id,
                InputAction::Key {
                    key,
                    pressed: false,
                    modifiers,
                },
            );
        }
        Ok(())
    }

    async fn focus_widget_for_keyboard(
        &self,
        viewport_id: Option<String>,
        target: &WidgetRef,
        timeout_ms: Option<u64>,
    ) -> Result<(WidgetRegistryEntry, egui::ViewportId), ToolError> {
        let (widget, viewport_id) = self
            .resolve_widget_for_pointer(viewport_id, target)
            .await
            .map_err(|error| ToolError::new(ErrorCode::InvalidRef, error.message))?;
        if !widget.enabled {
            return Err(ToolError::new(
                ErrorCode::TargetNotFocusable,
                "Target widget is not focusable",
            ));
        }
        if widget.focused {
            return Ok((widget, viewport_id));
        }
        let click_pos = widget.interact_rect.center();
        queue_primary_click(&self.inner, viewport_id, click_pos);
        let Some(timeout_ms) = timeout_ms else {
            return Ok((widget, viewport_id));
        };
        let viewport_id_str = viewport_id_to_string(viewport_id);
        self.wait_for_widget_state(
            Some(viewport_id_str.clone()),
            target.clone(),
            Some(timeout_ms),
            None,
            |widget| widget.is_some_and(|widget| widget.focused),
        )
        .await
        .map_err(|error| ToolError::new(ErrorCode::FocusNotAcquired, error.message))?;
        match resolve_widget(&self.inner, Some(viewport_id_str.as_str()), target) {
            Ok(focused_widget) if focused_widget.focused => Ok((focused_widget, viewport_id)),
            Ok(_) => Err(ToolError::new(
                ErrorCode::FocusNotAcquired,
                "Widget did not retain focus",
            )),
            Err(error) if error.code == ErrorCode::NotFound => Err(ToolError::new(
                ErrorCode::TargetDetached,
                "Target widget detached while focusing",
            )),
            Err(error) => Err(error),
        }
    }

    /// Inject a text event.
    async fn input_text(&self, viewport_id: Option<String>, text: String) -> ToolResult<()> {
        let viewport_id = resolve_viewport_id(&self.inner, viewport_id)?;
        self.inner
            .queue_action(viewport_id, InputAction::Text { text });
        Ok(())
    }

    /// Paste text into the focused widget.
    async fn action_paste(&self, viewport_id: Option<String>, text: String) -> ToolResult<()> {
        let viewport_id = resolve_viewport_id(&self.inner, viewport_id)?;
        self.inner
            .queue_action(viewport_id, InputAction::Paste { text });
        Ok(())
    }

    /// Inject a scroll event (delta is in egui points).
    async fn input_scroll(
        &self,
        viewport_id: Option<String>,
        delta: Vec2,
        modifiers: Option<Modifiers>,
    ) -> ToolResult<()> {
        let viewport_id = resolve_viewport_id(&self.inner, viewport_id)?;
        let modifiers = modifiers.unwrap_or_default();
        self.inner
            .queue_action(viewport_id, InputAction::Scroll { delta, modifiers });
        Ok(())
    }

    /// Request OS-level focus for a viewport.
    ///
    /// Raises the window and steals keyboard focus from whatever the user is currently working in.
    ///
    /// **WARNING: Do not use this for general app interaction or automation.** Input injection,
    /// clicks, keyboard events, and all other automation actions work correctly without OS focus.
    /// This function exists solely for testing window focus events themselves (e.g. verifying that
    /// your app responds correctly when it gains or loses focus). Using it unnecessarily disrupts
    /// the user's workflow.
    async fn focus_window(&self, viewport_id: String) -> ToolResult<()> {
        let viewport_id = self
            .inner
            .viewports
            .resolve_viewport_id(Some(viewport_id))
            .map_err(ToolError::from)?;
        self.inner
            .queue_command(viewport_id, egui::ViewportCommand::Focus);
        Ok(())
    }

    /// Request a viewport size change (sizes are in egui points).
    async fn viewport_set_inner_size(
        &self,
        viewport_id: Option<String>,
        inner_size: Vec2,
    ) -> ToolResult<()> {
        let viewport_id = resolve_viewport_id(&self.inner, viewport_id)?;
        ensure_positive_vec2(inner_size, "inner_size")?;
        self.inner.queue_command(
            viewport_id,
            egui::ViewportCommand::InnerSize(inner_size.into()),
        );
        Ok(())
    }

    /// Configure resize constraints for a viewport.
    async fn viewport_set_resize_options(
        &self,
        viewport_id: Option<String>,
        min_size: Option<Vec2>,
        max_size: Option<Vec2>,
        increments: Option<Vec2>,
        resizable: Option<bool>,
    ) -> ToolResult<()> {
        let viewport_id = resolve_viewport_id(&self.inner, viewport_id)?;
        if let Some(min_size) = min_size {
            ensure_positive_vec2(min_size, "min_size")?;
            self.inner.queue_command(
                viewport_id,
                egui::ViewportCommand::MinInnerSize(min_size.into()),
            );
        }
        if let Some(max_size) = max_size {
            ensure_positive_vec2(max_size, "max_size")?;
            self.inner.queue_command(
                viewport_id,
                egui::ViewportCommand::MaxInnerSize(max_size.into()),
            );
        }
        if let Some(increments) = increments {
            ensure_positive_vec2(increments, "increments")?;
            self.inner.queue_command(
                viewport_id,
                egui::ViewportCommand::ResizeIncrements(Some(increments.into())),
            );
        }
        if let Some(resizable) = resizable {
            self.inner
                .queue_command(viewport_id, egui::ViewportCommand::Resizable(resizable));
        }
        Ok(())
    }

    async fn wait_for_frame_count(
        &self,
        count: Option<u64>,
        timeout_ms: Option<u64>,
    ) -> ToolResult<u64> {
        let count = count.unwrap_or(1);
        let timeout_ms = timeout_ms.unwrap_or(DEFAULT_WAIT_TIMEOUT_MS);
        let start_frame = self.inner.frame_count();
        let target_frame = start_frame + count;

        let (matched, _, elapsed_ms) = wait_until_condition(
            &self.inner,
            timeout_ms,
            DEFAULT_POLL_INTERVAL_MS,
            None,
            || async {
                let current = self.inner.frame_count();
                Ok::<_, ToolError>((current >= target_frame, None::<()>))
            },
        )
        .await?;

        let end_frame = self.inner.frame_count();
        if matched {
            return Ok(end_frame);
        }

        Err(ToolError::new(
            ErrorCode::Timeout,
            format!(
                "Timed out waiting for {count} frame(s) after {timeout_ms}ms. \
If this is an eframe app, prefer Renderer::Glow for automation and ensure \
fixture requests are processed in App::update."
            ),
        )
        .with_details(wait_timeout_details(
            "frames",
            elapsed_ms,
            None,
            None,
            Some(start_frame),
            Some(end_frame),
        ))
        .into())
    }

    /// Wait until the target viewport has produced a fresh captured snapshot.
    async fn wait_for_capture(
        &self,
        viewport_id: Option<String>,
        timeout_ms: Option<u64>,
        poll_interval_ms: Option<u64>,
    ) -> ToolResult<()> {
        let viewport_id = resolve_viewport_id(&self.inner, viewport_id)?;
        let timeout_ms = timeout_ms.unwrap_or(DEFAULT_WAIT_TIMEOUT_MS);
        let poll_interval_ms = poll_interval_ms.unwrap_or(DEFAULT_POLL_INTERVAL_MS);
        let start_capture = self
            .inner
            .viewports
            .capture_snapshot(viewport_id)
            .map(|snapshot| snapshot.frame_count)
            .unwrap_or(0);

        let (matched, _, elapsed_ms) =
            wait_until_condition(&self.inner, timeout_ms, poll_interval_ms, None, || async {
                self.inner.request_repaint_of(viewport_id);
                let current = self
                    .inner
                    .viewports
                    .capture_snapshot(viewport_id)
                    .map(|snapshot| snapshot.frame_count)
                    .unwrap_or(0);
                Ok::<_, ToolError>((current > start_capture, None::<()>))
            })
            .await?;

        if matched {
            return Ok(());
        }

        Err(ToolError::new(
            ErrorCode::Timeout,
            format!("Timed out waiting for a fresh capture after {timeout_ms}ms"),
        )
        .with_details(wait_timeout_details(
            "capture",
            elapsed_ms,
            None,
            viewport_snapshot_for(&self.inner, viewport_id).as_ref(),
            Some(start_capture),
            self.inner
                .viewports
                .capture_snapshot(viewport_id)
                .map(|snapshot| snapshot.frame_count),
        ))
        .into())
    }

    /// Wait until the UI has settled: all input actions and viewport commands are drained
    /// and at least one frame has been processed.
    async fn wait_for_settle(
        &self,
        viewport_id: Option<String>,
        timeout_ms: Option<u64>,
        poll_interval_ms: Option<u64>,
    ) -> ToolResult<()> {
        let viewport_id = resolve_viewport_id(&self.inner, viewport_id)?;
        let timeout_ms = timeout_ms.unwrap_or(DEFAULT_WAIT_TIMEOUT_MS);
        let poll_interval_ms = poll_interval_ms.unwrap_or(DEFAULT_POLL_INTERVAL_MS);
        let start_frame = self.inner.frame_count();

        let (matched, _, elapsed_ms) =
            wait_until_condition(&self.inner, timeout_ms, poll_interval_ms, None, || async {
                let has_snapshot = self.inner.viewports.input_snapshot(viewport_id).is_some();
                let has_pending_actions = self.inner.actions.has_pending_actions(viewport_id);
                let has_pending_commands = self.inner.actions.has_pending_commands(viewport_id);
                let observed_new_frame = self.inner.frame_count() > start_frame;
                let processed_last_action =
                    self.inner.frame_count() > self.inner.last_action_frame.load(Ordering::Relaxed);
                let matched = has_snapshot
                    && observed_new_frame
                    && !has_pending_actions
                    && !has_pending_commands
                    && processed_last_action;
                Ok::<_, ToolError>((matched, None::<()>))
            })
            .await?;

        if matched {
            return Ok(());
        }

        Err(ToolError::new(
            ErrorCode::Timeout,
            format!("Timed out waiting for UI to settle after {timeout_ms}ms"),
        )
        .with_details(wait_timeout_details(
            "settle",
            elapsed_ms,
            None,
            viewport_snapshot_for(&self.inner, viewport_id).as_ref(),
            Some(start_frame),
            Some(self.inner.frame_count()),
        ))
        .into())
    }

    /// Wait until a scroll area has initialized and stabilized across captures.
    async fn wait_for_scroll_ready(
        &self,
        viewport_id: Option<String>,
        target: WidgetRef,
        timeout_ms: Option<u64>,
        poll_interval_ms: Option<u64>,
    ) -> ToolResult<Option<WidgetRegistryEntry>> {
        let timeout_ms = timeout_ms.unwrap_or(DEFAULT_WAIT_TIMEOUT_MS);
        let poll_interval_ms = poll_interval_ms.unwrap_or(DEFAULT_POLL_INTERVAL_MS);
        let last_sample = Mutex::new(None);

        let (matched, widget, elapsed_ms) =
            wait_until_condition(&self.inner, timeout_ms, poll_interval_ms, None, || async {
                let widget = match resolve_widget(&self.inner, viewport_id.as_deref(), &target) {
                    Ok(widget) => widget,
                    Err(error) if error.code == ErrorCode::NotFound => {
                        return Ok::<_, ToolError>((false, None));
                    }
                    Err(error) => return Err(error),
                };
                let viewport_id = self
                    .inner
                    .viewports
                    .resolve_viewport_id(Some(widget.viewport_id.clone()))
                    .map_err(ToolError::from)?;
                self.inner.request_repaint_of(viewport_id);
                let Some(capture) = self.inner.viewports.capture_snapshot(viewport_id) else {
                    return Ok((false, Some(widget)));
                };
                let Some(scroll) = scroll_state(&widget) else {
                    return Ok((false, Some(widget)));
                };
                if scroll.max_offset.y <= 0.0 {
                    return Ok((false, Some(widget)));
                }
                let current = ScrollSample {
                    frame_count: capture.frame_count,
                    max_offset: scroll.max_offset,
                    stabilized: false,
                };
                let matched = match last_sample
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .replace(current)
                {
                    Some(previous) => {
                        previous.frame_count != current.frame_count
                            && approx_eq_vec2(
                                previous.max_offset,
                                current.max_offset,
                                SCROLL_STABILITY_TOLERANCE,
                            )
                    }
                    None => false,
                };
                Ok((matched, Some(widget)))
            })
            .await?;

        if matched {
            return Ok(widget);
        }

        Err(ToolError::new(
            ErrorCode::Timeout,
            format!("Timed out waiting for scroll readiness after {timeout_ms}ms"),
        )
        .with_details(wait_timeout_details(
            "scroll_ready",
            elapsed_ms,
            widget.as_ref(),
            None,
            None,
            None,
        ))
        .into())
    }

    /// Wait for a widget to match a predicate over its current snapshot.
    async fn wait_for_widget_state<F>(
        &self,
        viewport_id: Option<String>,
        target: WidgetRef,
        timeout_ms: Option<u64>,
        poll_interval_ms: Option<u64>,
        mut predicate: F,
    ) -> ToolResult<Option<WidgetRegistryEntry>>
    where
        F: FnMut(Option<&WidgetRegistryEntry>) -> bool,
    {
        let timeout_ms = timeout_ms.unwrap_or(DEFAULT_WAIT_TIMEOUT_MS);
        let poll_interval_ms = poll_interval_ms.unwrap_or(DEFAULT_POLL_INTERVAL_MS);

        let (matched, widget, elapsed_ms) =
            wait_until_condition(&self.inner, timeout_ms, poll_interval_ms, None, || {
                let result = match resolve_widget(&self.inner, viewport_id.as_deref(), &target) {
                    Ok(widget) => {
                        let matched = predicate(Some(&widget));
                        Ok::<_, ToolError>((matched, Some(widget)))
                    }
                    Err(error) => {
                        if error.code == ErrorCode::NotFound {
                            let matched = predicate(None);
                            Ok((matched, None))
                        } else {
                            Err(error)
                        }
                    }
                };
                async move { result }
            })
            .await?;

        if matched {
            return Ok(widget);
        }

        Err(ToolError::new(
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
        .into())
    }

    #[allow(dead_code)]
    #[allow(clippy::too_many_arguments)]
    /// List widgets with optional visibility, role, and id-prefix filters.
    async fn widget_list(
        &self,
        viewport_id: Option<String>,
        include_invisible: Option<bool>,
        role: Option<WidgetRole>,
        id_prefix: Option<String>,
    ) -> ToolResult {
        let widgets = collect_widget_list(
            &self.inner,
            viewport_id,
            include_invisible,
            role,
            id_prefix.as_deref(),
        )?;
        Ok(CallToolResult::structured(widgets).map_err(|error| {
            ToolError::new(
                ErrorCode::Internal,
                format!("Failed to serialize widget list: {error}"),
            )
        })?)
    }

    /// Get a single widget by id (error if not found or ambiguous).
    async fn widget_get(
        &self,
        viewport_id: Option<String>,
        target: WidgetRef,
    ) -> ToolResult<WidgetGetResult> {
        let widget = resolve_widget(&self.inner, viewport_id.as_deref(), &target)?;
        Ok(WidgetGetResult { widget })
    }

    /// Directly set a widget's value without simulating input.
    async fn widget_set_value(
        &self,
        viewport_id: Option<String>,
        target: WidgetRef,
        value: WidgetValue,
    ) -> ToolResult<()> {
        let widget = resolve_widget(&self.inner, viewport_id.as_deref(), &target)?;
        validate_widget_value(&widget, &value)?;
        let WidgetRegistryEntry {
            id: widget_id,
            viewport_id: widget_viewport_id,
            ..
        } = widget;
        let viewport_id = self
            .inner
            .viewports
            .resolve_viewport_id(Some(widget_viewport_id))
            .map_err(ToolError::from)?;
        self.inner
            .queue_widget_value_update(viewport_id, widget_id, value);
        Ok(())
    }

    /// Queue a click on a widget without verifying resulting UI state.
    async fn action_click(
        &self,
        viewport_id: Option<String>,
        target: WidgetRef,
        button: Option<PointerButtonName>,
        modifiers: Option<Modifiers>,
        click_count: Option<u8>,
    ) -> ToolResult<()> {
        let (widget, viewport_id) = self
            .resolve_widget_for_pointer(viewport_id, &target)
            .await?;
        let pos = widget.interact_rect.center();
        let modifiers = modifiers.unwrap_or_default();
        let button = button.unwrap_or_default();
        let click_count = click_count.unwrap_or(1);
        if !(1..=3).contains(&click_count) {
            return Err(ToolError::new(
                ErrorCode::InvalidRef,
                "click_count must be between 1 and 3",
            )
            .into());
        }
        let pointer_button = button
            .to_pointer_button()
            .ok_or_else(|| ToolError::new(ErrorCode::InvalidRef, "Invalid pointer button"))?;
        queue_click(
            &self.inner,
            viewport_id,
            pos,
            pointer_button,
            modifiers,
            click_count,
        );
        Ok(())
    }

    /// Hover over a widget without clicking.
    async fn action_hover(
        &self,
        viewport_id: Option<String>,
        target: WidgetRef,
        position: Option<Vec2>,
        duration_ms: Option<u64>,
    ) -> ToolResult<()> {
        let (widget, viewport_id) = self
            .resolve_widget_for_pointer(viewport_id, &target)
            .await?;
        let pos = if let Some(position) = position {
            resolve_relative_pos(widget.interact_rect, position)?
        } else {
            widget.interact_rect.center()
        };
        self.inner
            .queue_action(viewport_id, InputAction::PointerMove { pos });
        let duration_ms = duration_ms.unwrap_or(0);
        if duration_ms > 0 {
            let frames = frames_for_duration(duration_ms);
            wait_for_frames(&self.inner, frames, Instant::now(), duration_ms).await?;
        }
        Ok(())
    }

    /// Type into a widget (optionally clearing first).
    async fn action_type(
        &self,
        viewport_id: Option<String>,
        target: WidgetRef,
        text: String,
        enter: Option<bool>,
        clear: Option<bool>,
    ) -> ToolResult<()> {
        let (widget, viewport_id) = self
            .resolve_widget_for_pointer(viewport_id, &target)
            .await?;
        let pos = widget.interact_rect.center();
        let queue_for_next_frame = !widget.focused;
        if queue_for_next_frame {
            queue_primary_click(&self.inner, viewport_id, pos);
        }
        let queue_action = |action| {
            if queue_for_next_frame {
                self.inner
                    .queue_action_with_timing(viewport_id, ActionTiming::Next, action);
            } else {
                self.inner.queue_action(viewport_id, action);
            }
        };
        let queue_key_press = |key, modifiers| {
            queue_action(InputAction::Key {
                key,
                pressed: true,
                modifiers,
            });
            queue_action(InputAction::Key {
                key,
                pressed: false,
                modifiers,
            });
        };
        let clear = clear.unwrap_or(false);
        if clear {
            let modifiers = Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            };
            queue_key_press(egui::Key::A, modifiers);
            queue_key_press(egui::Key::Backspace, Modifiers::default());
        }
        queue_action(InputAction::Text { text: text.clone() });
        let enter = enter.unwrap_or(false);
        if enter {
            queue_key_press(egui::Key::Enter, Modifiers::default());
        }
        Ok(())
    }

    /// Focus a widget by clicking on it.
    async fn action_focus(&self, viewport_id: Option<String>, target: WidgetRef) -> ToolResult<()> {
        let (widget, viewport_id) = self
            .resolve_widget_for_pointer(viewport_id, &target)
            .await?;
        let pos = widget.interact_rect.center();
        queue_primary_click(&self.inner, viewport_id, pos);
        Ok(())
    }

    /// Drag from a widget to an absolute position (points).
    async fn action_drag(
        &self,
        viewport_id: Option<String>,
        target: WidgetRef,
        to: Pos2,
        modifiers: Option<Modifiers>,
    ) -> ToolResult<()> {
        let (widget, viewport_id) = self
            .resolve_widget_for_pointer(viewport_id, &target)
            .await?;
        let start = widget.interact_rect.center();
        queue_drag(
            &self.inner,
            viewport_id,
            start,
            to,
            modifiers.unwrap_or_default(),
        );
        Ok(())
    }

    /// Drag within a widget using relative coordinates (0..1).
    async fn action_drag_relative(
        &self,
        viewport_id: Option<String>,
        target: WidgetRef,
        from: Option<Vec2>,
        to: Vec2,
        modifiers: Option<Modifiers>,
    ) -> ToolResult<()> {
        let (widget, viewport_id) = self
            .resolve_widget_for_pointer(viewport_id, &target)
            .await?;
        let start_relative = from.unwrap_or(Vec2 { x: 0.5, y: 0.5 });
        let start = resolve_relative_pos(widget.interact_rect, start_relative)?;
        let end = resolve_relative_pos(widget.interact_rect, to)?;
        queue_drag(
            &self.inner,
            viewport_id,
            start,
            end,
            modifiers.unwrap_or_default(),
        );
        Ok(())
    }

    /// Drag from one widget to another's center.
    async fn action_drag_to_widget(
        &self,
        viewport_id: Option<String>,
        from: WidgetRef,
        to: WidgetRef,
        modifiers: Option<Modifiers>,
    ) -> ToolResult<()> {
        let (from_widget, from_viewport) = self
            .resolve_widget_for_pointer(viewport_id.clone(), &from)
            .await?;
        let (to_widget, to_viewport) = self.resolve_widget_for_pointer(viewport_id, &to).await?;
        if from_viewport != to_viewport {
            return Err(ToolError::new(
                ErrorCode::InvalidRef,
                "Drag endpoints must be in the same viewport",
            )
            .into());
        }
        let viewport_id = from_viewport;
        let start = from_widget.interact_rect.center();
        let end = to_widget.interact_rect.center();
        queue_drag(
            &self.inner,
            viewport_id,
            start,
            end,
            modifiers.unwrap_or_default(),
        );
        Ok(())
    }

    /// Scroll a scroll area.
    async fn action_scroll(
        &self,
        viewport_id: Option<String>,
        target: WidgetRef,
        delta: Vec2,
        modifiers: Option<Modifiers>,
    ) -> ToolResult<()> {
        let (widget, viewport_id) = self
            .resolve_widget_for_pointer(viewport_id, &target)
            .await?;
        let pos = widget.interact_rect.center();
        self.inner
            .queue_action(viewport_id, InputAction::PointerMove { pos });
        let mut applied_override = false;
        if widget.role == WidgetRole::ScrollArea {
            let current = widget
                .role_state
                .as_ref()
                .and_then(RoleState::scroll_state)
                .map(|scroll| scroll.offset.into())
                .unwrap_or(egui::Vec2::ZERO);
            let delta_vec: egui::Vec2 = delta.into();
            let mut target = current - delta_vec;
            target.x = target.x.max(0.0);
            target.y = target.y.max(0.0);
            self.inner
                .set_scroll_override(viewport_id, widget.native_id, target);
            applied_override = true;
        }
        if !applied_override {
            self.inner.queue_action(
                viewport_id,
                InputAction::Scroll {
                    delta,
                    modifiers: modifiers.unwrap_or_default(),
                },
            );
        }
        Ok(())
    }

    /// Scroll a scroll area to an absolute offset or alignment.
    async fn action_scroll_to(
        &self,
        viewport_id: Option<String>,
        target: WidgetRef,
        offset: Option<Vec2>,
        align: Option<ScrollAlign>,
    ) -> ToolResult<()> {
        let (widget, viewport_id) =
            resolve_widget_and_viewport(&self.inner, viewport_id.as_deref(), &target)?;
        if widget.role != WidgetRole::ScrollArea {
            return Err(ToolError::new(
                ErrorCode::InvalidRef,
                "Target widget is not a scroll area",
            )
            .into());
        }
        let scroll = widget
            .role_state
            .as_ref()
            .and_then(RoleState::scroll_state)
            .ok_or_else(|| {
                ToolError::new(
                    ErrorCode::InvalidRef,
                    "Scroll metadata unavailable; render the scroll area before scrolling",
                )
            })?;
        if offset.is_some() && align.is_some() {
            return Err(ToolError::new(
                ErrorCode::InvalidRef,
                "Provide either offset or align, not both",
            )
            .into());
        }
        let mut target_offset = if let Some(offset) = offset {
            offset
        } else if let Some(align) = align {
            let y = match align {
                ScrollAlign::Top => 0.0,
                ScrollAlign::Center => scroll.max_offset.y * 0.5,
                ScrollAlign::Bottom => scroll.max_offset.y,
            };
            Vec2 {
                x: scroll.offset.x,
                y,
            }
        } else {
            return Err(
                ToolError::new(ErrorCode::InvalidRef, "Provide either offset or align").into(),
            );
        };
        target_offset.x = target_offset.x.clamp(0.0, scroll.max_offset.x);
        target_offset.y = target_offset.y.clamp(0.0, scroll.max_offset.y);
        let pos = widget.interact_rect.center();
        self.inner
            .queue_action(viewport_id, InputAction::PointerMove { pos });
        self.inner
            .set_scroll_override(viewport_id, widget.native_id, target_offset.into());
        Ok(())
    }

    /// Scroll ancestor scroll areas so the target widget becomes visible.
    async fn action_scroll_into_view(
        &self,
        viewport_id: Option<String>,
        target: WidgetRef,
    ) -> ToolResult<()> {
        let (widget, viewport_id) =
            resolve_widget_and_viewport(&self.inner, viewport_id.as_deref(), &target)?;
        let widgets = self.inner.widgets.widget_list(viewport_id);
        let by_id: HashMap<&str, &WidgetRegistryEntry> = widgets
            .iter()
            .map(|entry| (entry.id.as_str(), entry))
            .collect();
        let mut target_widget = widget;
        let mut parent_id = target_widget.parent_id.clone();

        while let Some(parent_key) = parent_id {
            let Some(parent) = by_id.get(parent_key.as_str()) else {
                break;
            };
            if parent.role == WidgetRole::ScrollArea
                && let Some(offset) = scroll_area_target_offset(parent, &target_widget)
            {
                self.inner
                    .set_scroll_override(viewport_id, parent.native_id, offset.into());
            }
            target_widget = (*parent).clone();
            parent_id = parent.parent_id.clone();
        }

        Ok(())
    }

    /// Analyze layout for common issues.
    async fn check_layout(
        &self,
        viewport_id: Option<String>,
        root: Option<WidgetRef>,
    ) -> ToolResult<Vec<LayoutIssue>> {
        let viewport_id = self.resolve_scope_viewport(viewport_id, root.as_ref())?;
        let mut widgets = self.inner.widgets.widget_list(viewport_id);
        let viewport_id_str = viewport_id_to_string(viewport_id);
        if let Some(root) = root.as_ref() {
            let root = resolve_widget(&self.inner, Some(viewport_id_str.as_str()), root)?;
            widgets = collect_subtree(&widgets, &root);
        }

        let viewport_rect = viewport_rect(&self.inner, viewport_id);
        let mut issues = Vec::new();
        issues.extend(check_zero_size(&widgets));
        issues.extend(check_clipping(&widgets, viewport_rect));
        issues.extend(check_overflow(&widgets, viewport_rect));
        issues.extend(check_overlaps(&widgets));
        if let Some(viewport_rect) = viewport_rect {
            issues.extend(check_offscreen(&widgets, viewport_rect));
        }
        if let Some(ctx) = self.inner.context_for(viewport_id) {
            issues.extend(check_text_truncation(&ctx, &widgets)?);
        }
        Ok(issues)
    }

    /// Measure text for a text-containing widget.
    async fn text_measure(&self, target: WidgetRef) -> ToolResult<TextMeasure> {
        let (widget, viewport_id) = resolve_widget_and_viewport(&self.inner, None, &target)?;
        let ctx = self.inner.context_for(viewport_id).ok_or_else(|| {
            ToolError::new(
                ErrorCode::InvalidRef,
                "Context not available for text measurement",
            )
        })?;
        Ok(measure_text(&ctx, &widget)?)
    }

    /// Find widget(s) at a specific coordinate.
    async fn widget_at_point(
        &self,
        pos: Pos2,
        all_layers: Option<bool>,
        viewport_id: Option<String>,
    ) -> ToolResult<WidgetAtPointResult> {
        let viewport_id = resolve_viewport_id(&self.inner, viewport_id)?;
        let widgets = self.inner.widgets.widget_list(viewport_id);
        let mut hits: Vec<WidgetRegistryEntry> = widgets
            .iter()
            .filter(|widget| point_in_rect(pos, widget.interact_rect))
            .cloned()
            .collect();
        let all_layers = all_layers.unwrap_or(false);
        hits.reverse(); // topmost first
        if !all_layers {
            hits.truncate(1);
        }
        Ok(WidgetAtPointResult { widgets: hits })
    }

    /// Show a persistent highlight on a widget or rectangle.
    async fn show_highlight(
        &self,
        viewport_id: Option<String>,
        target: Option<WidgetRef>,
        rect: Option<Rect>,
        color: String,
    ) -> ToolResult<OverlayHighlightResult> {
        let (rect, key) = if let Some(ref target) = target {
            let widget = resolve_widget(&self.inner, viewport_id.as_deref(), target)?;
            (widget.interact_rect, format!("widget:{}", widget.id))
        } else if let Some(rect) = rect {
            let key = format!(
                "rect:{},{},{},{}",
                rect.min.x, rect.min.y, rect.max.x, rect.max.y
            );
            (rect, key)
        } else {
            return Err(ToolError::new(ErrorCode::InvalidRef, "Missing rect or target").into());
        };
        let color = parse_color(&color).ok_or_else(|| {
            ToolError::new(ErrorCode::InvalidRef, format!("Invalid color: {color}"))
        })?;
        self.inner.set_overlay(
            key,
            OverlayEntry {
                rect: egui::Rect::from(rect),
                color,
                stroke_width: 2.0,
            },
        );
        Ok(OverlayHighlightResult { rect })
    }

    /// Hide highlights. If a target widget is given, removes just that widget's
    /// highlight. Otherwise clears all highlights.
    async fn hide_highlight(
        &self,
        viewport_id: Option<String>,
        target: Option<WidgetRef>,
    ) -> ToolResult<()> {
        if let Some(ref target) = target {
            let widget = resolve_widget(&self.inner, viewport_id.as_deref(), target)?;
            self.inner.remove_overlay(&format!("widget:{}", widget.id));
        } else {
            self.inner.clear_overlays();
        }
        Ok(())
    }

    /// Enable the persistent debug overlay with a fresh configuration.
    async fn show_debug_overlay(
        &self,
        _viewport_id: Option<String>,
        mode: Option<OverlayDebugModeName>,
        scope: Option<WidgetRef>,
        options: Option<OverlayDebugOptionsInput>,
    ) -> ToolResult<()> {
        let mut config = OverlayDebugConfig {
            enabled: true,
            mode: mode.map(Into::into).unwrap_or(OverlayDebugMode::Bounds),
            scope,
            options: OverlayDebugOptions::default(),
        };
        if let Some(input) = options {
            apply_overlay_debug_options(&mut config.options, input)?;
        }
        self.inner.set_overlay_debug_config(config);
        Ok(())
    }

    /// Disable the persistent debug overlay.
    async fn hide_debug_overlay(&self) -> ToolResult<()> {
        let config = OverlayDebugConfig::default();
        self.inner.set_overlay_debug_config(config);
        Ok(())
    }

    #[tool(defaults)]
    /// Evaluate a Luau script with DevMCP helpers. Scripts are assumed to be strict.
    async fn script_eval(
        &self,
        script: String,
        timeout_ms: Option<u64>,
        options: Option<ScriptEvalOptions>,
    ) -> ToolResult<CallToolResult> {
        let timeout_ms = timeout_ms.unwrap_or(script::DEFAULT_SCRIPT_TIMEOUT_MS);
        let options = options.unwrap_or_default();
        let source_name = options
            .source_name
            .unwrap_or_else(|| "script.luau".to_string());
        let inner = Arc::clone(&self.inner);
        let runtime = Arc::clone(&self.runtime);
        let handle = Handle::current();
        let eval = spawn_blocking(move || {
            script::run_script_eval(
                inner,
                runtime,
                handle,
                &script,
                timeout_ms,
                source_name,
                options.args,
            )
        })
        .await;
        Ok(match eval {
            Ok(result) => result.to_tool_result(),
            Err(error) => script::script_eval_task_error(&error).to_tool_result(),
        })
    }

    fn resolve_scope_viewport(
        &self,
        viewport_id: Option<String>,
        scope: Option<&WidgetRef>,
    ) -> Result<egui::ViewportId, ToolError> {
        if let Some(scope) = scope {
            let widget = resolve_widget(&self.inner, viewport_id.as_deref(), scope)?;
            return resolve_viewport_id(&self.inner, Some(widget.viewport_id));
        }
        resolve_viewport_id(&self.inner, viewport_id)
    }

    #[tool]
    /// Return the checked-in Luau definitions for the full scripting API.
    async fn script_api(&self) -> ToolResult<CallToolResult> {
        Ok(CallToolResult::new().with_text_content(script_definitions()))
    }

    /// Apply an app-defined fixture without waiting for readiness.
    async fn fixture_apply(&self, name: String) -> ToolResult<()> {
        self.fixture_apply_internal(&name).await?;
        Ok(())
    }

    /// Navigate to an app-defined fixture by name and wait for readiness anchors.
    async fn fixture(&self, name: String, timeout_ms: Option<u64>) -> ToolResult<()> {
        let (spec, fixture_epoch) = self.fixture_apply_internal(&name).await?;
        let timeout_ms = timeout_ms.unwrap_or(DEFAULT_WAIT_TIMEOUT_MS);
        self.evaluate_anchors(&spec, fixture_epoch, timeout_ms)
            .await
            .map_err(Into::into)
    }
}

const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(5);
const FRAME_WAIT_TIMEOUT: Duration = Duration::from_millis(500);

fn resolve_screenshot_viewport(
    inner: &Inner,
    viewport_id: Option<String>,
) -> Result<egui::ViewportId, ToolError> {
    if let Some(viewport_id) = viewport_id {
        return resolve_viewport_id(inner, Some(viewport_id));
    }
    Ok(egui::ViewportId::ROOT)
}

async fn capture_screenshot(
    inner: &Inner,
    runtime: &Runtime,
    viewport_id: egui::ViewportId,
    kind: ScreenshotKind,
) -> Result<String, ToolError> {
    // Best-effort wake-up before sending the screenshot command. Some idle windows won't
    // produce a frame until a command is queued, so only treat this as fatal if context
    // capture is not ready yet.
    let event_loop_ready = ensure_event_loop_active(inner, runtime, viewport_id).await;
    let has_snapshot = inner.viewports.has_viewport_snapshot(viewport_id);
    if !inner.has_context() {
        if let Err(error) = event_loop_ready {
            return Err(error);
        }
        return Err(ToolError::new(
            ErrorCode::InvalidRef,
            "Viewport context not ready for screenshots",
        )
        .with_details(screenshot_error_details(inner, runtime, viewport_id)));
    }
    if !has_snapshot {
        event_loop_ready?;
        return Err(
            ToolError::new(ErrorCode::InvalidRef, "Viewport not ready for screenshots")
                .with_details(screenshot_error_details(inner, runtime, viewport_id)),
        );
    }

    let start_frame = inner.frame_count();
    let request_id = inner.next_request_id();
    let kind_snapshot = kind.clone();
    runtime.insert_screenshot(request_id, ScreenshotState::pending(kind));
    inner.queue_command(
        viewport_id,
        egui::ViewportCommand::Screenshot(egui::UserData::new(request_id)),
    );
    runtime.record_screenshot_request(inner, request_id, viewport_id, &kind_snapshot);
    inner.request_repaint_of(viewport_id);
    let state = await_screenshot(
        inner,
        runtime,
        request_id,
        viewport_id,
        &kind_snapshot,
        start_frame,
    )
    .await?;
    build_screenshot_data(&state)
}

async fn ensure_event_loop_active(
    inner: &Inner,
    runtime: &Runtime,
    viewport_id: egui::ViewportId,
) -> Result<(), ToolError> {
    let initial_frame = inner.frame_count();

    // Wait for at least one frame to process. Use a short poll interval with
    // periodic repaint requests so we recover when the event loop stalls.
    let frame_wait = async {
        loop {
            if inner.frame_count() > initial_frame {
                return;
            }
            let notified = runtime.frame_notify().notified();
            inner.request_repaint_of(viewport_id);
            let poll = Duration::from_millis(DEFAULT_POLL_INTERVAL_MS);
            drop(timeout(poll, notified).await);
        }
    };

    if timeout(FRAME_WAIT_TIMEOUT, frame_wait).await.is_err() {
        return Err(ToolError::new(
            ErrorCode::Internal,
            "Window event loop not responding. The window may be minimized or hidden. \
For eframe apps, prefer Renderer::Glow for automation; Wgpu backends can stall idle frames.",
        )
        .with_details(screenshot_error_details(inner, runtime, viewport_id)));
    }

    Ok(())
}

async fn await_screenshot(
    inner: &Inner,
    runtime: &Runtime,
    request_id: u64,
    viewport_id: egui::ViewportId,
    kind: &ScreenshotKind,
    start_frame: u64,
) -> Result<ScreenshotState, ToolError> {
    let notify = match runtime.screenshot_state(request_id) {
        Some(state) => state.notify(),
        None => {
            return Err(
                ToolError::new(ErrorCode::InvalidRef, "Unknown request id").with_details(
                    screenshot_request_details(inner, runtime, request_id, viewport_id, kind),
                ),
            );
        }
    };

    let wait_loop = async {
        let mut requested_followup = false;
        loop {
            let notified = notify.notified();
            if let Some(state) = runtime.screenshot_state(request_id) {
                if state.is_ready() {
                    break;
                }
            } else {
                return Err(ToolError::new(ErrorCode::InvalidRef, "Unknown request id")
                    .with_details(screenshot_request_details(
                        inner,
                        runtime,
                        request_id,
                        viewport_id,
                        kind,
                    )));
            }
            if !requested_followup && inner.frame_count() > start_frame {
                inner.request_repaint_of(viewport_id);
                requested_followup = true;
            }
            notified.await;
        }
        Ok(())
    };

    timeout(SCREENSHOT_TIMEOUT, wait_loop).await.map_err(|_| {
        // Clean up the pending screenshot request.
        runtime.take_screenshot(request_id);
        runtime.log_screenshot(inner, format!(
            "timeout request_id={request_id} viewport={} start_frame={start_frame} end_frame={}",
            viewport_id_to_string(viewport_id),
            inner.frame_count(),
        ));
        ToolError::new(
            ErrorCode::Internal,
            "Screenshot timed out waiting for a screenshot event. The screenshot command may \
                 not have reached the viewport or the frame did not render. Ensure the \
                 DevMcp raw_input_hook is wired so screenshot events can be captured.",
        )
        .with_details(screenshot_request_details_with_frames(
            inner,
            runtime,
            request_id,
            viewport_id,
            kind,
            start_frame,
            inner.frame_count(),
        ))
    })??;

    runtime.take_screenshot(request_id).ok_or_else(|| {
        ToolError::new(ErrorCode::InvalidRef, "Unknown request id").with_details(
            screenshot_request_details_with_frames(
                inner,
                runtime,
                request_id,
                viewport_id,
                kind,
                start_frame,
                inner.frame_count(),
            ),
        )
    })
}

fn build_screenshot_data(state: &ScreenshotState) -> Result<String, ToolError> {
    let Some(image) = state.image() else {
        return Err(ToolError::new(
            ErrorCode::Internal,
            "Screenshot missing image",
        ));
    };
    let image = match &state.kind {
        ScreenshotKind::Viewport => image,
        ScreenshotKind::Widget {
            rect,
            pixels_per_point,
        } => crop_image(&image, *rect, *pixels_per_point)?,
    };
    encode_jpeg(&image)
}

fn screenshot_error_details(
    inner: &Inner,
    runtime: &Runtime,
    viewport_id: egui::ViewportId,
) -> Value {
    let snapshots = inner.viewports.viewports_snapshot();
    let known_viewports = snapshots
        .iter()
        .map(|snapshot| snapshot.viewport_id.clone())
        .collect::<Vec<_>>();
    serde_json::json!({
        "viewport_id": viewport_id_to_string(viewport_id),
        "has_context": inner.has_context(),
        "known_viewports": known_viewports,
        "frame_count": inner.frame_count(),
        "has_snapshot": inner.viewports.has_viewport_snapshot(viewport_id),
        "debug": runtime.screenshot_debug_snapshot(inner),
    })
}

fn screenshot_request_details(
    inner: &Inner,
    runtime: &Runtime,
    request_id: u64,
    viewport_id: egui::ViewportId,
    kind: &ScreenshotKind,
) -> Value {
    screenshot_request_details_with_frames(inner, runtime, request_id, viewport_id, kind, 0, 0)
}

fn screenshot_request_details_with_frames(
    inner: &Inner,
    runtime: &Runtime,
    request_id: u64,
    viewport_id: egui::ViewportId,
    kind: &ScreenshotKind,
    start_frame: u64,
    end_frame: u64,
) -> Value {
    let kind_details = match kind {
        ScreenshotKind::Viewport => serde_json::json!({ "kind": "viewport" }),
        ScreenshotKind::Widget {
            rect,
            pixels_per_point,
        } => serde_json::json!({
            "kind": "widget",
            "rect": rect,
            "pixels_per_point": pixels_per_point,
        }),
    };
    serde_json::json!({
        "request_id": request_id,
        "viewport_id": viewport_id_to_string(viewport_id),
        "kind": kind_details,
        "start_frame": start_frame,
        "end_frame": end_frame,
        "debug": runtime.screenshot_debug_snapshot(inner),
    })
}

fn encode_jpeg(image: &egui::ColorImage) -> Result<String, ToolError> {
    const JPEG_QUALITY: u8 = 80;
    let width = image.size[0] as u32;
    let height = image.size[1] as u32;
    let mut bytes = Vec::with_capacity((width * height * 3) as usize);
    for pixel in &image.pixels {
        let [r, g, b, a] = pixel.to_array();
        if a == 255 {
            bytes.extend_from_slice(&[r, g, b]);
        } else {
            let alpha = u16::from(a);
            let inv = 255_u16.saturating_sub(alpha);
            let r = ((u16::from(r) * alpha) + 255 * inv) / 255;
            let g = ((u16::from(g) * alpha) + 255 * inv) / 255;
            let b = ((u16::from(b) * alpha) + 255 * inv) / 255;
            bytes.extend_from_slice(&[r as u8, g as u8, b as u8]);
        }
    }
    let mut jpeg_data = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut jpeg_data, JPEG_QUALITY);
    image::ImageEncoder::write_image(
        encoder,
        &bytes,
        width,
        height,
        image::ExtendedColorType::Rgb8,
    )
    .map_err(|error| ToolError::new(ErrorCode::Internal, format!("JPEG encode failed: {error}")))?;
    Ok(STANDARD.encode(jpeg_data))
}

fn crop_image(
    image: &egui::ColorImage,
    rect: Rect,
    pixels_per_point: f32,
) -> Result<Arc<egui::ColorImage>, ToolError> {
    let width = image.size[0] as i32;
    let height = image.size[1] as i32;
    let min_x = (rect.min.x * pixels_per_point).round() as i32;
    let min_y = (rect.min.y * pixels_per_point).round() as i32;
    let max_x = (rect.max.x * pixels_per_point).round() as i32;
    let max_y = (rect.max.y * pixels_per_point).round() as i32;
    let x0 = min_x.clamp(0, width);
    let y0 = min_y.clamp(0, height);
    let x1 = max_x.clamp(0, width);
    let y1 = max_y.clamp(0, height);
    let crop_width = (x1 - x0).max(0) as usize;
    let crop_height = (y1 - y0).max(0) as usize;
    if crop_width == 0 || crop_height == 0 {
        return Err(ToolError::new(
            ErrorCode::InvalidRef,
            "Widget rect is empty",
        ));
    }
    let mut pixels = Vec::with_capacity(crop_width * crop_height);
    for y in y0..y1 {
        for x in x0..x1 {
            let idx = (y as usize) * image.size[0] + x as usize;
            if let Some(pixel) = image.pixels.get(idx) {
                pixels.push(*pixel);
            }
        }
    }
    Ok(Arc::new(egui::ColorImage {
        size: [crop_width, crop_height],
        source_size: egui::Vec2::new(crop_width as f32, crop_height as f32),
        pixels,
    }))
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering as AtomicOrdering},
        },
        time::{Duration, Instant},
    };

    use eguidev::FixtureSpec;
    use serde_json::Value;
    use tmcp::schema::ContentBlock;
    use tokio::{task::yield_now, time::sleep};

    use super::*;
    use crate::{
        actions::InputAction,
        overlay::{OverlayDebugConfig, OverlayDebugMode},
        registry::{Inner, viewport_id_to_string},
        runtime::Runtime,
        tools::types::LayoutIssueKind,
        types::{
            Modifiers, Pos2, Rect, Vec2, WidgetRef, WidgetRegistryEntry, WidgetRole, WidgetValue,
        },
        viewports::InputSnapshot,
        widget_registry::{WidgetMeta, record_widget},
    };

    fn apply_actions(inner: &Inner, raw_input: &mut egui::RawInput) {
        let viewport_id = raw_input.viewport_id;
        let actions = inner.actions.drain_actions(viewport_id);
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
    }

    fn widget_ref_id(id: &str) -> WidgetRef {
        WidgetRef {
            id: Some(id.to_string()),
            viewport_id: None,
        }
    }

    fn make_entry(id: &str, native_id: u64, role: WidgetRole) -> WidgetRegistryEntry {
        make_entry_with_rect(
            id,
            native_id,
            role,
            Rect {
                min: Pos2 { x: 0.0, y: 0.0 },
                max: Pos2 { x: 10.0, y: 10.0 },
            },
            None,
        )
    }

    fn make_generated_entry(native_id: u64, role: WidgetRole) -> WidgetRegistryEntry {
        let mut entry = make_entry(&format!("{native_id:x}"), native_id, role);
        entry.explicit_id = false;
        entry
    }

    fn make_entry_with_rect(
        id: &str,
        native_id: u64,
        role: WidgetRole,
        rect: Rect,
        parent_id: Option<&str>,
    ) -> WidgetRegistryEntry {
        WidgetRegistryEntry {
            id: id.to_string(),
            explicit_id: true,
            native_id,
            viewport_id: "root".to_string(),
            layer_id: "layer".to_string(),
            rect,
            interact_rect: rect,
            role,
            label: None,
            value: None,
            layout: None,
            role_state: None,
            parent_id: parent_id.map(str::to_string),
            enabled: true,
            visible: true,
            focused: false,
        }
    }

    fn make_scroll_entry(
        id: &str,
        native_id: u64,
        offset: Vec2,
        max_offset: Vec2,
    ) -> WidgetRegistryEntry {
        let mut entry = make_entry(id, native_id, WidgetRole::ScrollArea);
        entry.role_state = Some(RoleState::ScrollArea {
            offset,
            viewport_size: Vec2 { x: 100.0, y: 100.0 },
            content_size: Vec2 {
                x: 100.0 + max_offset.x,
                y: 100.0 + max_offset.y,
            },
        });
        entry
    }

    fn capture_test_frame(inner: &Arc<Inner>, ctx: &egui::Context) {
        inner.capture_context(ctx.viewport_id(), ctx);
        inner
            .viewports
            .capture_input_snapshot(ctx, inner.fixture_epoch(), inner.frame_count() + 1);
        inner.advance_frame();
        Runtime::ensure_for_inner(inner)
            .frame_notify()
            .notify_waiters();
    }

    fn record_test_snapshot(inner: &Arc<Inner>, viewport_id: egui::ViewportId) {
        inner.viewports.record_input_snapshot(
            viewport_id,
            InputSnapshot {
                pixels_per_point: 1.0,
                pointer_pos: None,
            },
            inner.fixture_epoch(),
            inner.frame_count() + 1,
        );
        inner.advance_frame();
        Runtime::ensure_for_inner(inner)
            .frame_notify()
            .notify_waiters();
    }

    fn parse_script_eval_json(result: &CallToolResult) -> Value {
        let content = result.content.first().expect("content");
        match content {
            ContentBlock::Text(text) => serde_json::from_str(&text.text).expect("script eval json"),
            other => panic!("expected text content, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tools_list_contains_only_script_eval_and_script_api() {
        use tmcp::{ServerHandler, schema::Cursor, testutils::TestServerContext};

        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(inner);
        let ctx = TestServerContext::new();
        let tools = server
            .list_tools(ctx.ctx(), None::<Cursor>)
            .await
            .expect("list tools")
            .tools;
        let mut names: Vec<_> = tools.iter().map(|tool| tool.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["script_api", "script_eval"]);
    }

    #[tokio::test]
    async fn script_api_returns_checked_in_definitions() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(inner);
        let result = server.script_api().await.expect("script_api");
        let content = result.content.first().expect("content");
        match content {
            ContentBlock::Text(text) => assert_eq!(text.text, script_definitions()),
            other => panic!("expected text content, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn script_eval_returns_value_and_logs() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(inner);
        let result = server
            .script_eval("log(\"hello\")\nreturn 1 + 1".to_string(), None, None)
            .await
            .expect("script eval");
        let json = parse_script_eval_json(&result);
        assert_eq!(json["success"], true);
        assert_eq!(json["value"].as_i64(), Some(2));
        assert_eq!(json["logs"][0], "hello");
    }

    #[tokio::test]
    async fn script_eval_omitted_args_default_to_empty_table() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(inner);
        let result = server
            .script_eval("return next(args) == nil".to_string(), None, None)
            .await
            .expect("script eval");
        let json = parse_script_eval_json(&result);
        assert_eq!(json["success"], true);
        assert_eq!(json["value"], true);
    }

    #[tokio::test]
    async fn script_eval_exposes_scalar_args() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(inner);
        let result = server
            .script_eval(
                "return { name = args.name, count = args.count, ratio = args.ratio, enabled = args.enabled }"
                    .to_string(),
                None,
                Some(ScriptEvalOptions {
                    source_name: Some("args.luau".to_string()),
                    args: ScriptArgs::from([
                        ("name".to_string(), ScriptArgValue::String("Sky".to_string())),
                        ("count".to_string(), ScriptArgValue::Int(4)),
                        ("ratio".to_string(), ScriptArgValue::Float(1.5)),
                        ("enabled".to_string(), ScriptArgValue::Bool(true)),
                    ]),
                }),
            )
            .await
            .expect("script eval");
        let json = parse_script_eval_json(&result);
        assert_eq!(json["success"], true);
        assert_eq!(json["value"]["name"], "Sky");
        assert_eq!(json["value"]["count"], 4);
        assert_eq!(json["value"]["ratio"], 1.5);
        assert_eq!(json["value"]["enabled"], true);
    }

    #[tokio::test]
    async fn script_eval_reports_parse_errors() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(inner);
        let result = server
            .script_eval("local x =".to_string(), None, None)
            .await
            .expect("script eval");
        let json = parse_script_eval_json(&result);
        assert_eq!(json["success"], false);
        assert_eq!(json["error"]["type"], "parse");
    }

    #[tokio::test]
    async fn script_eval_wait_for_widget_closure_matches() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        let mut entry = make_entry("status", 1, WidgetRole::Label);
        entry.label = Some("Ready".to_string());
        inner.widgets.record_widget(viewport_id, entry);
        inner.widgets.finalize_registry(viewport_id);

        let result = server
            .script_eval(
                r#"configure({ timeout_ms = 50 }) return root():wait_for_widget("status", function(w) return w.label == "Ready" end)"#
                    .to_string(),
                None,
                None,
            )
            .await
            .expect("script eval");
        let json = parse_script_eval_json(&result);
        assert_eq!(json["success"], true);
        assert_eq!(json["value"]["label"], "Ready");
    }

    #[tokio::test]
    async fn script_eval_wait_for_widget_absent() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        inner.widgets.finalize_registry(viewport_id);

        let result = server
            .script_eval(
                r#"configure({ timeout_ms = 30 }) root():wait_for_widget_absent("missing") return true"#
                    .to_string(),
                None,
                None,
            )
            .await
            .expect("script eval");
        let json = parse_script_eval_json(&result);
        assert_eq!(json["success"], true);
        assert_eq!(json["value"], true);
    }

    #[tokio::test]
    async fn script_eval_wait_for_widget_visible_matches_existing_widget() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        let mut entry = make_entry("status", 1, WidgetRole::Label);
        entry.label = Some("Ready".to_string());
        inner.widgets.record_widget(viewport_id, entry);
        inner.widgets.finalize_registry(viewport_id);

        let result = server
            .script_eval(
                r#"configure({ timeout_ms = 50 }) return root():wait_for_widget_visible("status")"#
                    .to_string(),
                None,
                None,
            )
            .await
            .expect("script eval");
        let json = parse_script_eval_json(&result);
        assert_eq!(json["success"], true);
        assert_eq!(json["value"]["visible"], true);
        assert_eq!(json["value"]["label"], "Ready");
    }

    #[tokio::test]
    async fn script_eval_widget_wait_for_visible_waits_until_visible() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        let mut entry = make_entry("status", 1, WidgetRole::Label);
        entry.label = Some("Ready".to_string());
        entry.visible = false;
        inner.widgets.record_widget(viewport_id, entry);
        inner.widgets.finalize_registry(viewport_id);

        let inner_for_update = Arc::clone(&inner);
        tokio::spawn(async move {
            sleep(Duration::from_millis(5)).await;
            inner_for_update.widgets.clear_registry(viewport_id);
            let mut entry = make_entry("status", 1, WidgetRole::Label);
            entry.label = Some("Ready".to_string());
            entry.visible = true;
            inner_for_update.widgets.record_widget(viewport_id, entry);
            inner_for_update.widgets.finalize_registry(viewport_id);
        });

        let result = server
            .script_eval(
                r#"configure({ timeout_ms = 100, poll_interval_ms = 1 })
local widget = root():widget_get("status")
return widget:wait_for_visible()"#
                    .to_string(),
                None,
                None,
            )
            .await
            .expect("script eval");
        let json = parse_script_eval_json(&result);
        assert_eq!(json["success"], true);
        assert_eq!(json["value"]["visible"], true);
    }

    #[tokio::test]
    async fn script_eval_wait_for_widget_visible_waits_for_widget_to_appear() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        inner.widgets.finalize_registry(viewport_id);

        let inner_for_update = Arc::clone(&inner);
        tokio::spawn(async move {
            sleep(Duration::from_millis(5)).await;
            inner_for_update.widgets.clear_registry(viewport_id);
            let mut entry = make_entry("status", 1, WidgetRole::Label);
            entry.label = Some("Ready".to_string());
            inner_for_update.widgets.record_widget(viewport_id, entry);
            inner_for_update.widgets.finalize_registry(viewport_id);
        });

        let result = server
            .script_eval(
                r#"configure({ timeout_ms = 100, poll_interval_ms = 1 }) return root():wait_for_widget_visible("status")"#
                    .to_string(),
                None,
                None,
            )
            .await
            .expect("script eval");
        let json = parse_script_eval_json(&result);
        assert_eq!(json["success"], true);
        assert_eq!(json["value"]["visible"], true);
        assert_eq!(json["value"]["label"], "Ready");
    }

    #[tokio::test]
    async fn script_eval_wait_for_widget_visible_times_out_when_hidden() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        let mut entry = make_entry("status", 1, WidgetRole::Label);
        entry.visible = false;
        inner.widgets.record_widget(viewport_id, entry);
        inner.widgets.finalize_registry(viewport_id);

        let result = server
            .script_eval(
                r#"configure({ timeout_ms = 30, poll_interval_ms = 1 }) return root():wait_for_widget_visible("status")"#
                    .to_string(),
                None,
                None,
            )
            .await
            .expect("script eval");
        let json = parse_script_eval_json(&result);
        assert_eq!(json["success"], false);
        assert_eq!(json["error"]["type"], "timeout");
        assert_eq!(json["error"]["details"]["kind"], "widget_visible");
    }

    #[tokio::test]
    async fn script_eval_wait_for_widget_closure_times_out() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        let entry = make_entry("status", 1, WidgetRole::Label);
        inner.widgets.record_widget(viewport_id, entry);
        inner.widgets.finalize_registry(viewport_id);

        let result = server
            .script_eval(
                r#"configure({ timeout_ms = 30 }) return root():wait_for_widget("status", function(w) return w.label == "Never" end)"#
                    .to_string(),
                None,
                None,
            )
            .await
            .expect("script eval");
        let json = parse_script_eval_json(&result);
        assert_eq!(json["success"], false);
        assert_eq!(json["error"]["type"], "timeout");
        assert_eq!(json["error"]["details"]["kind"], "widget");
    }

    #[tokio::test]
    async fn script_eval_wait_for_widget_closure_respects_script_timeout() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;
        let entry = make_entry("status", 1, WidgetRole::Label);

        inner.widgets.clear_registry(viewport_id);
        inner.widgets.record_widget(viewport_id, entry);
        inner.widgets.finalize_registry(viewport_id);

        let started = Instant::now();
        let result = server
            .script_eval(
                r#"configure({ timeout_ms = 5000, poll_interval_ms = 250 }) return root():wait_for_widget("status", function(w) return w.label == "Never" end)"#
                    .to_string(),
                Some(50),
                None,
            )
            .await
            .expect("script eval");
        let json = parse_script_eval_json(&result);
        assert_eq!(json["success"], false);
        assert_eq!(json["error"]["type"], "timeout");
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[tokio::test]
    async fn script_eval_drag_relative_accepts_positional_from() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        inner.widgets.record_widget(
            viewport_id,
            make_entry_with_rect(
                "slider",
                1,
                WidgetRole::Slider,
                Rect {
                    min: Pos2 { x: 0.0, y: 0.0 },
                    max: Pos2 { x: 10.0, y: 10.0 },
                },
                None,
            ),
        );
        inner.widgets.finalize_registry(viewport_id);

        let result = server
            .script_eval(
                r#"root():widget_get("slider"):drag_relative(
                    { x = 0.8, y = 0.5 },
                    { x = 0.2, y = 0.5 },
                    { settle = false }
                )"#
                .to_string(),
                None,
                None,
            )
            .await
            .expect("script eval");
        let json = parse_script_eval_json(&result);
        assert_eq!(json["success"], true);

        let actions = inner.actions.drain_actions(viewport_id);
        let Some(InputAction::PointerMove { pos }) = actions.first() else {
            panic!("expected initial pointer move from drag_relative");
        };
        assert!((pos.x - 2.0).abs() < f32::EPSILON);
        assert!((pos.y - 5.0).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn script_eval_widget_at_point_accepts_boolean_all_layers() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        inner.widgets.record_widget(
            viewport_id,
            make_entry_with_rect(
                "bottom",
                1,
                WidgetRole::Button,
                Rect {
                    min: Pos2 { x: 0.0, y: 0.0 },
                    max: Pos2 { x: 10.0, y: 10.0 },
                },
                None,
            ),
        );
        inner.widgets.record_widget(
            viewport_id,
            make_entry_with_rect(
                "top",
                2,
                WidgetRole::Button,
                Rect {
                    min: Pos2 { x: 0.0, y: 0.0 },
                    max: Pos2 { x: 10.0, y: 10.0 },
                },
                None,
            ),
        );
        inner.widgets.finalize_registry(viewport_id);

        let result = server
            .script_eval(
                r#"local widgets = root():widget_at_point({ x = 5, y = 5 }, true)
                return { count = #widgets, first = widgets[1].id }"#
                    .to_string(),
                None,
                None,
            )
            .await
            .expect("script eval");
        let json = parse_script_eval_json(&result);
        assert_eq!(json["success"], true);
        assert_eq!(json["value"]["count"], 2);
        assert_eq!(json["value"]["first"], "top");
    }

    #[tokio::test]
    async fn script_eval_widget_show_debug_overlay_preserves_viewport_scope() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let secondary = egui::ViewportId::from_hash_of("secondary");
        let viewport_selector = viewport_id_to_string(secondary);
        let ctx = egui::Context::default();

        {
            let mut raw_input = egui::RawInput {
                viewport_id: egui::ViewportId::ROOT,
                ..Default::default()
            };
            raw_input
                .viewports
                .insert(egui::ViewportId::ROOT, Default::default());
            raw_input.viewports.insert(secondary, Default::default());
            drop(ctx.run(raw_input, |_| {}));
        }
        inner.capture_context(egui::ViewportId::ROOT, &ctx);
        inner.viewports.update_viewports(&ctx);
        inner.viewports.capture_input_snapshot(
            &ctx,
            inner.fixture_epoch(),
            inner.frame_count() + 1,
        );

        inner.widgets.clear_registry(secondary);
        let mut entry = make_entry("overlay", 1, WidgetRole::Button);
        entry.viewport_id = viewport_id_to_string(secondary);
        inner.widgets.record_widget(secondary, entry);
        inner.widgets.finalize_registry(secondary);

        let result = server
            .script_eval(
                format!(
                    "for _, vp in ipairs(viewports()) do if vp.id == \"{viewport_selector}\" then return vp:widget_get(\"overlay\"):show_debug_overlay() end end"
                ),
                None,
                None,
            )
            .await
            .expect("script eval");
        let json = parse_script_eval_json(&result);
        assert_eq!(json["success"], true);

        let config = inner.overlays.overlay_debug_config();
        let scope = config.scope.expect("widget-scoped overlay");
        assert_eq!(scope.id.as_deref(), Some("overlay"));
        assert_eq!(
            scope.viewport_id.as_deref(),
            Some(viewport_selector.as_str())
        );
    }

    #[tokio::test]
    async fn script_eval_action_settle_rejects_invalid_types() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(inner);
        let result = server
            .script_eval(
                r#"root():paste("hello", { settle = "fast" })"#.to_string(),
                None,
                None,
            )
            .await
            .expect("script eval");
        let json = parse_script_eval_json(&result);
        assert_eq!(json["success"], false);
        assert_eq!(json["error"]["type"], "type_error");
        assert_eq!(json["error"]["message"], "settle must be a boolean");
    }

    #[tokio::test]
    async fn script_eval_click_settle_targets_widget_viewport() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let ctx = egui::Context::default();
        let secondary = egui::ViewportId::from_hash_of("secondary");

        for _ in 0..2 {
            let mut raw_input = egui::RawInput {
                viewport_id: egui::ViewportId::ROOT,
                ..Default::default()
            };
            raw_input
                .viewports
                .insert(egui::ViewportId::ROOT, Default::default());
            raw_input.viewports.insert(secondary, Default::default());
            drop(ctx.run(raw_input, |_| {}));
        }
        inner.capture_context(egui::ViewportId::ROOT, &ctx);
        inner.viewports.update_viewports(&ctx);
        inner.viewports.capture_input_snapshot(
            &ctx,
            inner.fixture_epoch(),
            inner.frame_count() + 1,
        );

        inner.widgets.clear_registry(secondary);
        let mut entry = make_entry("status", 1, WidgetRole::Button);
        entry.viewport_id = viewport_id_to_string(secondary);
        inner.widgets.record_widget(secondary, entry);
        inner.widgets.finalize_registry(secondary);

        let viewport_selector = viewport_id_to_string(secondary);
        let result = server
            .script_eval(
                format!(
                    "configure({{ timeout_ms = 10, poll_interval_ms = 1 }})\nfor _, vp in ipairs(viewports()) do if vp.id == \"{viewport_selector}\" then return vp:widget_get(\"status\"):click() end end"
                ),
                None,
                None,
            )
            .await
            .expect("script eval");
        let json = parse_script_eval_json(&result);
        assert_eq!(json["success"], false);
        assert_eq!(json["error"]["type"], "timeout");
        assert_eq!(json["error"]["details"]["kind"], "settle");
    }

    #[tokio::test]
    async fn script_eval_reports_assertion_failures() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(inner);
        let result = server
            .script_eval("assert(false, \"nope\")".to_string(), None, None)
            .await
            .expect("script eval");
        let json = parse_script_eval_json(&result);
        assert_eq!(json["success"], false);
        assert_eq!(json["error"]["type"], "assertion");
        assert_eq!(json["assertions"][0]["passed"], false);
    }

    #[tokio::test]
    async fn fixture_apply_applies_handler_without_waiting_for_a_new_frame() {
        let inner = Arc::new(Inner::new());
        inner.fixtures.set_fixtures(vec![
            FixtureSpec::new("test_fixture", "Test fixture.").anchor("status"),
        ]);
        inner.fixtures.set_fixture_handler(Arc::new(|_name| Ok(())));
        let ctx = egui::Context::default();
        let raw_input = egui::RawInput {
            viewport_id: egui::ViewportId::ROOT,
            ..Default::default()
        };
        drop(ctx.run(raw_input, |_| {}));
        inner.capture_context(egui::ViewportId::ROOT, &ctx);
        inner.viewports.capture_input_snapshot(
            &ctx,
            inner.fixture_epoch(),
            inner.frame_count() + 1,
        );
        let server = DevMcpServer::new(Arc::clone(&inner));
        server
            .fixture_apply("test_fixture".to_string())
            .await
            .expect("fixture_apply result");
    }

    #[tokio::test]
    async fn fixture_returns_error_when_handler_fails() {
        let inner = Arc::new(Inner::new());
        inner.fixtures.set_fixtures(vec![
            FixtureSpec::new("broken", "Broken fixture.").anchor("status"),
        ]);
        inner
            .fixtures
            .set_fixture_handler(Arc::new(|_name| Err("fixture failed".to_string())));
        let server = DevMcpServer::new(Arc::clone(&inner));
        let result = server.fixture("broken".to_string(), None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fixture_returns_error_when_no_handler_registered() {
        let inner = Arc::new(Inner::new());
        inner.fixtures.set_fixtures(vec![
            FixtureSpec::new("no_handler", "No handler fixture.").anchor("status"),
        ]);
        let server = DevMcpServer::new(Arc::clone(&inner));
        let result = server.fixture("no_handler".to_string(), None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn script_eval_fixture_raw_succeeds_without_a_new_frame() {
        let inner = Arc::new(Inner::new());
        inner.fixtures.set_fixtures(vec![
            FixtureSpec::new("slow", "Slow fixture.").anchor("status"),
        ]);
        inner.fixtures.set_fixture_handler(Arc::new(|_name| Ok(())));
        let server = DevMcpServer::new(Arc::clone(&inner));
        let result = server
            .script_eval("fixture_raw(\"slow\")".to_string(), Some(20), None)
            .await
            .expect("script eval");
        let json = parse_script_eval_json(&result);
        assert_eq!(json["success"], true);
    }

    #[tokio::test]
    async fn script_eval_fixture_timeout_reports_stale_snapshot_diagnostics() {
        let inner = Arc::new(Inner::new());
        inner.fixtures.set_fixtures(vec![
            FixtureSpec::new("stale", "Stale fixture.").anchor("status"),
        ]);
        inner.fixtures.set_fixture_handler(Arc::new(|_name| Ok(())));

        let ctx = egui::Context::default();
        let raw_input = egui::RawInput {
            viewport_id: egui::ViewportId::ROOT,
            ..Default::default()
        };
        drop(ctx.run(raw_input, |_| {}));
        let entry = make_entry("status", 1, WidgetRole::Label);
        inner.widgets.clear_registry(egui::ViewportId::ROOT);
        inner.widgets.record_widget(egui::ViewportId::ROOT, entry);
        inner.widgets.finalize_registry(egui::ViewportId::ROOT);
        capture_test_frame(&inner, &ctx);

        let server = DevMcpServer::new(Arc::clone(&inner));
        let result = server
            .script_eval(
                "configure({ timeout_ms = 20, poll_interval_ms = 1 }) fixture(\"stale\")"
                    .to_string(),
                None,
                None,
            )
            .await
            .expect("script eval");
        let json = parse_script_eval_json(&result);
        assert_eq!(json["success"], false);
        assert_eq!(json["error"]["type"], "timeout");
        assert_eq!(json["error"]["details"]["fixture"], "stale");
        assert!(
            json["error"]["message"]
                .as_str()
                .expect("timeout message")
                .contains("status visible"),
            "timeout should include per-anchor diagnostics"
        );
        assert!(
            json["error"]["details"]["statuses"][0]["detail"]
                .as_str()
                .expect("status detail")
                .contains("post-fixture capture"),
            "timeout details should explain the stale capture"
        );
    }

    #[tokio::test]
    async fn fixture_waits_for_multiviewport_anchor_capture() {
        let inner = Arc::new(Inner::new());
        let secondary = egui::ViewportId::from_hash_of("fixture.secondary");
        inner.fixtures.set_fixtures(vec![
            FixtureSpec::new("multi", "Multi viewport fixture.").anchor_in("status", secondary),
        ]);
        inner.fixtures.set_fixture_handler(Arc::new(|_name| Ok(())));

        let root_ctx = egui::Context::default();
        let mut root_input = egui::RawInput {
            viewport_id: egui::ViewportId::ROOT,
            ..Default::default()
        };
        root_input
            .viewports
            .insert(egui::ViewportId::ROOT, Default::default());
        root_input.viewports.insert(secondary, Default::default());
        drop(root_ctx.run(root_input, |_| {}));
        inner.capture_context(egui::ViewportId::ROOT, &root_ctx);
        inner.viewports.update_viewports(&root_ctx);
        capture_test_frame(&inner, &root_ctx);

        let server = DevMcpServer::new(Arc::clone(&inner));
        let inner_for_capture = Arc::clone(&inner);
        tokio::spawn(async move {
            sleep(Duration::from_millis(20)).await;
            inner_for_capture.widgets.clear_registry(secondary);
            let mut entry = make_entry("status", 1, WidgetRole::Label);
            entry.viewport_id = viewport_id_to_string(secondary);
            inner_for_capture.widgets.record_widget(secondary, entry);
            inner_for_capture.widgets.finalize_registry(secondary);
            record_test_snapshot(&inner_for_capture, secondary);
        });

        server
            .fixture("multi".to_string(), Some(200))
            .await
            .expect("fixture");
    }

    #[tokio::test]
    async fn fixture_waits_for_scroll_anchor_stability() {
        let inner = Arc::new(Inner::new());
        inner.fixtures.set_fixtures(vec![
            FixtureSpec::new("scroll", "Scroll fixture.").anchor_scroll_at(
                "scroll",
                Vec2 { x: 0.0, y: 300.0 },
                1.0,
            ),
        ]);
        inner.fixtures.set_fixture_handler(Arc::new(|_name| Ok(())));

        let ctx = egui::Context::default();
        let raw_input = egui::RawInput {
            viewport_id: egui::ViewportId::ROOT,
            ..Default::default()
        };
        drop(ctx.run(raw_input, |_| {}));
        capture_test_frame(&inner, &ctx);

        let server = DevMcpServer::new(Arc::clone(&inner));
        let inner_for_capture = Arc::clone(&inner);
        tokio::spawn(async move {
            let capture_ctx = egui::Context::default();
            let raw_input = egui::RawInput {
                viewport_id: egui::ViewportId::ROOT,
                ..Default::default()
            };
            drop(capture_ctx.run(raw_input, |_| {}));
            sleep(Duration::from_millis(20)).await;
            inner_for_capture
                .widgets
                .clear_registry(egui::ViewportId::ROOT);
            inner_for_capture.widgets.record_widget(
                egui::ViewportId::ROOT,
                make_scroll_entry(
                    "scroll",
                    1,
                    Vec2 { x: 0.0, y: 300.0 },
                    Vec2 { x: 0.0, y: 600.0 },
                ),
            );
            inner_for_capture
                .widgets
                .finalize_registry(egui::ViewportId::ROOT);
            capture_test_frame(&inner_for_capture, &capture_ctx);
            sleep(Duration::from_millis(25)).await;
            inner_for_capture
                .widgets
                .clear_registry(egui::ViewportId::ROOT);
            inner_for_capture.widgets.record_widget(
                egui::ViewportId::ROOT,
                make_scroll_entry(
                    "scroll",
                    1,
                    Vec2 { x: 0.0, y: 300.0 },
                    Vec2 { x: 0.0, y: 600.0 },
                ),
            );
            inner_for_capture
                .widgets
                .finalize_registry(egui::ViewportId::ROOT);
            capture_test_frame(&inner_for_capture, &capture_ctx);
        });

        server
            .fixture("scroll".to_string(), Some(250))
            .await
            .expect("fixture");
    }

    #[tokio::test]
    async fn wait_for_capture_waits_for_new_snapshot() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let ctx = egui::Context::default();
        let raw_input = egui::RawInput {
            viewport_id: egui::ViewportId::ROOT,
            ..Default::default()
        };
        drop(ctx.run(raw_input, |_| {}));
        capture_test_frame(&inner, &ctx);

        let inner_for_capture = Arc::clone(&inner);
        tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            let capture_ctx = egui::Context::default();
            let raw_input = egui::RawInput {
                viewport_id: egui::ViewportId::ROOT,
                ..Default::default()
            };
            drop(capture_ctx.run(raw_input, |_| {}));
            capture_test_frame(&inner_for_capture, &capture_ctx);
        });

        server
            .wait_for_capture(None, Some(500), Some(1))
            .await
            .expect("wait_for_capture");
    }

    #[tokio::test]
    async fn wait_for_scroll_ready_waits_for_stable_scroll_state() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let ctx = egui::Context::default();
        let raw_input = egui::RawInput {
            viewport_id: egui::ViewportId::ROOT,
            ..Default::default()
        };
        drop(ctx.run(raw_input, |_| {}));
        inner.widgets.clear_registry(egui::ViewportId::ROOT);
        inner.widgets.record_widget(
            egui::ViewportId::ROOT,
            make_scroll_entry(
                "scroll",
                1,
                Vec2 { x: 0.0, y: 150.0 },
                Vec2 { x: 0.0, y: 400.0 },
            ),
        );
        inner.widgets.finalize_registry(egui::ViewportId::ROOT);
        capture_test_frame(&inner, &ctx);

        let inner_for_capture = Arc::clone(&inner);
        tokio::spawn(async move {
            let capture_ctx = egui::Context::default();
            let raw_input = egui::RawInput {
                viewport_id: egui::ViewportId::ROOT,
                ..Default::default()
            };
            drop(capture_ctx.run(raw_input, |_| {}));
            sleep(Duration::from_millis(50)).await;
            inner_for_capture
                .widgets
                .clear_registry(egui::ViewportId::ROOT);
            inner_for_capture.widgets.record_widget(
                egui::ViewportId::ROOT,
                make_scroll_entry(
                    "scroll",
                    1,
                    Vec2 { x: 0.0, y: 150.0 },
                    Vec2 { x: 0.0, y: 400.0 },
                ),
            );
            inner_for_capture
                .widgets
                .finalize_registry(egui::ViewportId::ROOT);
            capture_test_frame(&inner_for_capture, &capture_ctx);
            sleep(Duration::from_millis(60)).await;
            inner_for_capture
                .widgets
                .clear_registry(egui::ViewportId::ROOT);
            inner_for_capture.widgets.record_widget(
                egui::ViewportId::ROOT,
                make_scroll_entry(
                    "scroll",
                    1,
                    Vec2 { x: 0.0, y: 150.0 },
                    Vec2 { x: 0.0, y: 400.0 },
                ),
            );
            inner_for_capture
                .widgets
                .finalize_registry(egui::ViewportId::ROOT);
            capture_test_frame(&inner_for_capture, &capture_ctx);
        });

        let result = server
            .wait_for_scroll_ready(None, widget_ref_id("scroll"), Some(500), Some(1))
            .await
            .expect("wait_for_scroll_ready")
            .expect("widget snapshot");
        let scroll = scroll_state(&result).expect("scroll metadata");
        assert_eq!(scroll.offset.y, 150.0);
    }

    #[tokio::test]
    async fn fixture_rejects_unregistered_names() {
        let inner = Arc::new(Inner::new());
        inner.fixtures.set_fixtures(vec![
            FixtureSpec::new("known", "Known fixture.").anchor("status"),
        ]);
        let server = DevMcpServer::new(Arc::clone(&inner));
        let result = server.fixture("unknown".to_string(), None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fixtures_are_sorted_for_scripts() {
        let inner = Arc::new(Inner::new());
        inner.fixtures.set_fixtures(vec![
            FixtureSpec::new("zeta", "Last fixture.").anchor("status"),
            FixtureSpec::new("alpha", "First fixture.").anchor("status"),
        ]);
        let specs = inner.fixtures.fixtures_sorted();
        let specs: Vec<_> = specs.into_iter().map(|fixture| fixture.name).collect();
        assert_eq!(specs, vec!["alpha".to_string(), "zeta".to_string()]);
    }

    #[tokio::test]
    async fn fixture_clears_transient_automation_state_on_apply_boundaries() {
        let inner = Arc::new(Inner::new());
        inner.fixtures.set_fixtures(vec![
            FixtureSpec::new("reset", "Reset fixture.").anchor("status"),
        ]);
        let applied = Arc::new(AtomicBool::new(false));
        let applied_handler = Arc::clone(&applied);
        inner.fixtures.set_fixture_handler(Arc::new(move |_name| {
            applied_handler.store(true, AtomicOrdering::Relaxed);
            Ok(())
        }));
        let ctx = egui::Context::default();
        let raw_input = egui::RawInput {
            viewport_id: egui::ViewportId::ROOT,
            ..Default::default()
        };
        drop(ctx.run(raw_input, |_| {}));
        inner.capture_context(egui::ViewportId::ROOT, &ctx);
        inner.viewports.capture_input_snapshot(
            &ctx,
            inner.fixture_epoch(),
            inner.frame_count() + 1,
        );
        let viewport_id = egui::ViewportId::ROOT;
        inner.queue_action(
            viewport_id,
            InputAction::Text {
                text: "stale".to_string(),
            },
        );
        inner.queue_action_with_timing(
            viewport_id,
            ActionTiming::Next,
            InputAction::Text {
                text: "staged".to_string(),
            },
        );
        inner.queue_command(
            viewport_id,
            egui::ViewportCommand::Title("stale title".to_string()),
        );
        inner.queue_widget_value_update(
            viewport_id,
            "field".to_string(),
            WidgetValue::Text("queued".to_string()),
        );
        inner.set_scroll_override(viewport_id, 7, egui::vec2(1.0, 2.0));
        inner.set_overlay_debug_config(OverlayDebugConfig {
            enabled: true,
            ..Default::default()
        });

        let inner_for_frame = Arc::clone(&inner);
        let runtime_for_frame = Runtime::ensure_for_inner(&inner);
        tokio::spawn(async move {
            for _ in 0..4 {
                yield_now().await;
                inner_for_frame.advance_frame();
                runtime_for_frame.frame_notify().notify_waiters();
            }
        });
        let server = DevMcpServer::new(Arc::clone(&inner));
        server
            .fixture_apply("reset".to_string())
            .await
            .expect("fixture result");

        assert!(applied.load(AtomicOrdering::Relaxed));
        // After fixture application, transient state should be cleared.
        assert!(!inner.actions.has_pending_actions(viewport_id));
        assert!(!inner.actions.has_pending_commands(viewport_id));
        assert!(
            inner
                .take_widget_value_update(viewport_id, "field")
                .is_none()
        );
        assert!(inner.take_scroll_override(viewport_id, 7).is_none());
        assert!(!inner.overlays.overlay_debug_config().enabled);
    }

    #[tokio::test]
    async fn action_drag_relative_updates_slider_value() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let ctx = egui::Context::default();
        let viewport_id = egui::ViewportId::ROOT;
        let mut value = 42.0_f32;

        inner.widgets.clear_registry(viewport_id);
        let raw_input = egui::RawInput {
            viewport_id,
            ..Default::default()
        };
        let _output = ctx.run(raw_input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let response = ui.add(egui::Slider::new(&mut value, 0.0..=100.0));
                let visible = ui.is_visible() && ui.is_rect_visible(response.rect);
                record_widget(
                    &inner.widgets,
                    "slider".to_string(),
                    &response,
                    WidgetMeta {
                        role: WidgetRole::Slider,
                        value: Some(WidgetValue::Float(f64::from(value))),
                        visible,
                        ..Default::default()
                    },
                );
            });
        });
        inner.widgets.finalize_registry(viewport_id);

        server
            .action_drag_relative(
                None,
                widget_ref_id("slider"),
                Some(Vec2 { x: 0.2, y: 0.5 }),
                Vec2 { x: 0.8, y: 0.5 },
                None,
            )
            .await
            .expect("drag relative");

        let mut raw_input = egui::RawInput {
            viewport_id,
            ..Default::default()
        };
        apply_actions(&inner, &mut raw_input);
        let _output = ctx.run(raw_input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add(egui::Slider::new(&mut value, 0.0..=100.0));
            });
        });

        let mut raw_input = egui::RawInput {
            viewport_id,
            ..Default::default()
        };
        apply_actions(&inner, &mut raw_input);
        let _output = ctx.run(raw_input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add(egui::Slider::new(&mut value, 0.0..=100.0));
            });
        });

        assert!(value > 42.0_f32);
    }

    #[tokio::test]
    async fn action_type_clear_replaces_text() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let ctx = egui::Context::default();
        let viewport_id = egui::ViewportId::ROOT;
        let mut text = "Hello".to_string();

        inner.widgets.clear_registry(viewport_id);
        let raw_input = egui::RawInput {
            viewport_id,
            ..Default::default()
        };
        let _output = ctx.run(raw_input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let response = ui.text_edit_multiline(&mut text);
                let visible = ui.is_visible() && ui.is_rect_visible(response.rect);
                record_widget(
                    &inner.widgets,
                    "notes".to_string(),
                    &response,
                    WidgetMeta {
                        role: WidgetRole::TextEdit,
                        value: Some(WidgetValue::Text(text.clone())),
                        visible,
                        ..Default::default()
                    },
                );
            });
        });
        inner.widgets.finalize_registry(viewport_id);

        server
            .action_type(
                None,
                widget_ref_id("notes"),
                "World".to_string(),
                None,
                Some(true),
            )
            .await
            .expect("type action");

        let mut raw_input = egui::RawInput {
            viewport_id,
            ..Default::default()
        };
        apply_actions(&inner, &mut raw_input);
        let _output = ctx.run(raw_input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.text_edit_multiline(&mut text);
            });
        });

        let mut raw_input = egui::RawInput {
            viewport_id,
            ..Default::default()
        };
        apply_actions(&inner, &mut raw_input);
        let _output = ctx.run(raw_input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.text_edit_multiline(&mut text);
            });
        });

        assert_eq!(text, "World");
    }

    #[tokio::test]
    async fn action_type_skips_click_when_widget_already_focused() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        let mut entry = make_entry("notes", 1, WidgetRole::TextEdit);
        entry.focused = true;
        inner.widgets.record_widget(viewport_id, entry);
        inner.widgets.finalize_registry(viewport_id);

        server
            .action_type(
                None,
                widget_ref_id("notes"),
                "World".to_string(),
                None,
                None,
            )
            .await
            .expect("action type");

        let actions = inner.actions.drain_actions(viewport_id);
        let click_presses = actions
            .iter()
            .filter(|action| matches!(action, InputAction::PointerButton { pressed: true, .. }))
            .count();
        assert_eq!(click_presses, 0);
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, InputAction::Text { text } if text == "World"))
        );
    }

    #[tokio::test]
    async fn action_focus_updates_widget_focus_after_frame() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let ctx = egui::Context::default();
        let viewport_id = egui::ViewportId::ROOT;
        let mut text = "Hello".to_string();

        inner.widgets.clear_registry(viewport_id);
        let raw_input = egui::RawInput {
            viewport_id,
            ..Default::default()
        };
        let _output = ctx.run(raw_input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let response = ui.text_edit_multiline(&mut text);
                let visible = ui.is_visible() && ui.is_rect_visible(response.rect);
                record_widget(
                    &inner.widgets,
                    "notes".to_string(),
                    &response,
                    WidgetMeta {
                        role: WidgetRole::TextEdit,
                        value: Some(WidgetValue::Text(text.clone())),
                        visible,
                        ..Default::default()
                    },
                );
            });
        });
        inner.widgets.finalize_registry(viewport_id);

        server
            .action_focus(None, widget_ref_id("notes"))
            .await
            .expect("action focus");

        let mut raw_input = egui::RawInput {
            viewport_id,
            ..Default::default()
        };
        apply_actions(&inner, &mut raw_input);
        inner.widgets.clear_registry(viewport_id);
        let _output = ctx.run(raw_input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let response = ui.text_edit_multiline(&mut text);
                let visible = ui.is_visible() && ui.is_rect_visible(response.rect);
                record_widget(
                    &inner.widgets,
                    "notes".to_string(),
                    &response,
                    WidgetMeta {
                        role: WidgetRole::TextEdit,
                        value: Some(WidgetValue::Text(text.clone())),
                        visible,
                        ..Default::default()
                    },
                );
            });
        });
        inner.widgets.finalize_registry(viewport_id);

        let focused = server
            .widget_get(None, widget_ref_id("notes"))
            .await
            .expect("widget get")
            .widget
            .focused;
        assert!(focused);
    }

    #[tokio::test]
    async fn viewport_set_inner_size_rejects_invalid_sizes() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(inner);
        let result = server
            .viewport_set_resize_options(None, Some(Vec2 { x: -1.0, y: 480.0 }), None, None, None)
            .await
            .expect_err("invalid min_inner_size");
        assert_eq!(result.code, ErrorCode::InvalidRef.as_str());
        assert!(result.message.contains("min_size"));
    }

    #[tokio::test]
    async fn viewport_set_inner_size_queues_commands() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let inner_size = Vec2 { x: 800.0, y: 600.0 };
        let min_inner_size = Vec2 { x: 400.0, y: 300.0 };
        let max_inner_size = Vec2 {
            x: 1200.0,
            y: 900.0,
        };
        let resize_increments = Vec2 { x: 10.0, y: 20.0 };

        server
            .viewport_set_inner_size(None, inner_size)
            .await
            .expect("viewport set inner size");
        server
            .viewport_set_resize_options(
                None,
                Some(min_inner_size),
                Some(max_inner_size),
                Some(resize_increments),
                Some(true),
            )
            .await
            .expect("viewport set resize options");

        let pending = inner.actions.drain_all_commands();
        assert_eq!(pending.len(), 1);
        let (viewport_id, commands) = &pending[0];
        assert_eq!(*viewport_id, egui::ViewportId::ROOT);
        assert_eq!(
            commands,
            &vec![
                egui::ViewportCommand::InnerSize(inner_size.into()),
                egui::ViewportCommand::MinInnerSize(min_inner_size.into()),
                egui::ViewportCommand::MaxInnerSize(max_inner_size.into()),
                egui::ViewportCommand::ResizeIncrements(Some(resize_increments.into())),
                egui::ViewportCommand::Resizable(true),
            ]
        );
    }

    #[tokio::test]
    async fn widget_list_respects_visibility() {
        let inner = Arc::new(Inner::new());
        let ctx = egui::Context::default();
        let raw_input = egui::RawInput::default();
        let _output = ctx.run(raw_input, |ctx| {
            inner.capture_context(ctx.viewport_id(), ctx);
            inner.widgets.clear_registry(ctx.viewport_id());
            egui::CentralPanel::default().show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(40.0)
                    .show(ui, |ui| {
                        let visible = ui.button("Visible");
                        let visible_flag = ui.is_rect_visible(visible.rect);
                        record_widget(
                            &inner.widgets,
                            "visible".to_string(),
                            &visible,
                            WidgetMeta {
                                role: WidgetRole::Button,
                                visible: visible_flag,
                                ..Default::default()
                            },
                        );

                        ui.add_space(200.0);

                        let hidden = ui.button("Hidden");
                        let hidden_flag = ui.is_rect_visible(hidden.rect);
                        record_widget(
                            &inner.widgets,
                            "hidden".to_string(),
                            &hidden,
                            WidgetMeta {
                                role: WidgetRole::Button,
                                visible: hidden_flag,
                                ..Default::default()
                            },
                        );
                    });
            });
            inner.widgets.finalize_registry(ctx.viewport_id());
        });

        let server = DevMcpServer::new(Arc::clone(&inner));
        let result: Vec<WidgetRegistryEntry> = server
            .widget_list(None, Some(false), None, None)
            .await
            .expect("widget list")
            .structured_as()
            .expect("widget list payload");
        let tags: Vec<_> = result.iter().map(|entry| entry.id.as_str()).collect();
        assert!(tags.contains(&"visible"));
        assert!(!tags.contains(&"hidden"));

        let result: Vec<WidgetRegistryEntry> = server
            .widget_list(None, Some(true), None, None)
            .await
            .expect("widget list")
            .structured_as()
            .expect("widget list payload");
        let tags: Vec<_> = result.iter().map(|entry| entry.id.as_str()).collect();
        assert!(tags.contains(&"visible"));
        assert!(tags.contains(&"hidden"));
    }

    #[tokio::test]
    async fn widget_get_missing_returns_error() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(inner);
        let result = server
            .widget_get(None, widget_ref_id("missing"))
            .await
            .expect_err("missing widget");
        assert_eq!(result.code, ErrorCode::NotFound.as_str());
        assert!(result.message.contains("missing"));
    }

    #[tokio::test]
    async fn widget_get_round_trips_widget_list_id() {
        let inner = Arc::new(Inner::new());
        let viewport_id = egui::ViewportId::ROOT;
        let big_id = (1_u64 << 63) + 5;
        assert!(big_id > (1_u64 << 53));

        inner.widgets.clear_registry(viewport_id);
        inner
            .widgets
            .record_widget(viewport_id, make_entry("big", big_id, WidgetRole::Button));
        inner.widgets.finalize_registry(viewport_id);

        let server = DevMcpServer::new(Arc::clone(&inner));
        let list: Vec<WidgetRegistryEntry> = server
            .widget_list(None, Some(true), None, None)
            .await
            .expect("widget list")
            .structured_as()
            .expect("widget list payload");
        let entry = list
            .iter()
            .find(|entry| entry.id == "big")
            .expect("big widget");
        assert_eq!(entry.id, "big");

        let fetched = server
            .widget_get(None, widget_ref_id(&entry.id))
            .await
            .expect("widget get");
        assert_eq!(fetched.widget.id, "big");
    }

    #[tokio::test]
    async fn widget_get_fails_on_duplicate_explicit_ids() {
        let inner = Arc::new(Inner::new());
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        inner.widgets.record_widget(
            viewport_id,
            make_entry_with_rect(
                "dup",
                1,
                WidgetRole::Button,
                Rect {
                    min: Pos2 { x: 0.0, y: 0.0 },
                    max: Pos2 { x: 10.0, y: 10.0 },
                },
                Some("left"),
            ),
        );
        inner.widgets.record_widget(
            viewport_id,
            make_entry_with_rect(
                "dup",
                2,
                WidgetRole::Button,
                Rect {
                    min: Pos2 { x: 0.0, y: 0.0 },
                    max: Pos2 { x: 10.0, y: 10.0 },
                },
                Some("right"),
            ),
        );
        inner.widgets.finalize_registry(viewport_id);

        let server = DevMcpServer::new(Arc::clone(&inner));
        let result = server
            .widget_get(None, widget_ref_id("dup"))
            .await
            .expect_err("duplicate explicit ids should block automation");
        assert_eq!(result.code, ErrorCode::DuplicateWidgetId.as_str());
        let duplicate_ids = result
            .structured
            .as_ref()
            .and_then(|value| value.get("error"))
            .and_then(|error| error.get("details"))
            .and_then(|details| details.get("duplicate_ids"))
            .and_then(|value| value.as_array())
            .expect("duplicate ids");
        assert_eq!(duplicate_ids.len(), 1);
        assert_eq!(
            duplicate_ids[0].get("id").and_then(Value::as_str),
            Some("dup")
        );
    }

    #[tokio::test]
    async fn widget_get_generated_duplicates_remain_ambiguous() {
        let inner = Arc::new(Inner::new());
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        let mut first = make_generated_entry(10, WidgetRole::Button);
        first.id = "dup".to_string();
        inner.widgets.record_widget(viewport_id, first);
        let mut second = make_generated_entry(11, WidgetRole::Button);
        second.id = "dup".to_string();
        inner.widgets.record_widget(viewport_id, second);
        inner.widgets.finalize_registry(viewport_id);

        let server = DevMcpServer::new(Arc::clone(&inner));
        let result = server
            .widget_get(None, widget_ref_id("dup"))
            .await
            .expect_err("ambiguous id");
        assert_eq!(result.code, ErrorCode::Ambiguous.as_str());
        assert_ne!(result.code, ErrorCode::DuplicateWidgetId.as_str());
    }

    #[tokio::test]
    async fn widget_get_accepts_generated_hex_id() {
        let inner = Arc::new(Inner::new());
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        inner
            .widgets
            .record_widget(viewport_id, make_generated_entry(5, WidgetRole::Button));
        inner.widgets.finalize_registry(viewport_id);

        let server = DevMcpServer::new(Arc::clone(&inner));
        let list: Vec<WidgetRegistryEntry> = server
            .widget_list(None, Some(true), None, None)
            .await
            .expect("widget list")
            .structured_as()
            .expect("widget list payload");
        let entry = list
            .iter()
            .find(|entry| entry.id == "5")
            .expect("generated widget");

        let fetched = server
            .widget_get(None, widget_ref_id(&entry.id))
            .await
            .expect("widget get by generated id");
        assert_eq!(fetched.widget.id, entry.id);
    }

    #[tokio::test]
    async fn input_pointer_button_toggles_checkbox() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let ctx = egui::Context::default();
        let viewport_id = egui::ViewportId::ROOT;
        let mut checked = false;

        inner.widgets.clear_registry(viewport_id);
        let raw_input = egui::RawInput {
            viewport_id,
            ..Default::default()
        };
        let _output = ctx.run(raw_input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let response = ui.checkbox(&mut checked, "Enabled");
                let visible = ui.is_visible() && ui.is_rect_visible(response.rect);
                record_widget(
                    &inner.widgets,
                    "toggle".to_string(),
                    &response,
                    WidgetMeta {
                        role: WidgetRole::Checkbox,
                        label: Some("Enabled".to_string()),
                        value: Some(WidgetValue::Bool(checked)),
                        visible,
                        ..Default::default()
                    },
                );
            });
        });
        inner.widgets.finalize_registry(viewport_id);

        let widget = inner
            .widgets
            .widget_list(viewport_id)
            .into_iter()
            .find(|entry| entry.id == "toggle")
            .expect("toggle widget");
        let pos = widget.interact_rect.center();

        server
            .input_pointer_move(None, pos)
            .await
            .expect("pointer move");
        server
            .input_pointer_button(None, pos, PointerButtonName::Primary, true, None)
            .await
            .expect("pointer down");
        server
            .input_pointer_button(None, pos, PointerButtonName::Primary, false, None)
            .await
            .expect("pointer up");

        let mut raw_input = egui::RawInput {
            viewport_id,
            ..Default::default()
        };
        apply_actions(&inner, &mut raw_input);
        let _output = ctx.run(raw_input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.checkbox(&mut checked, "Enabled");
            });
        });

        assert!(checked);
    }

    #[tokio::test]
    async fn input_tools_queue_actions_and_commands() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;
        let pos = Pos2 { x: 12.0, y: 24.0 };
        let modifiers = Modifiers {
            shift: true,
            ..Default::default()
        };

        server
            .input_pointer_move(None, pos)
            .await
            .expect("pointer move");
        server
            .input_pointer_button(None, pos, PointerButtonName::Primary, true, Some(modifiers))
            .await
            .expect("pointer button");
        server
            .input_key(None, "Enter".to_string(), true, Some(modifiers))
            .await
            .expect("key input");
        server
            .input_text(None, "Hello".to_string())
            .await
            .expect("text input");
        server
            .input_scroll(None, Vec2 { x: 1.0, y: -2.0 }, Some(modifiers))
            .await
            .expect("scroll input");
        server
            .focus_window("root".to_string())
            .await
            .expect("focus window");

        let queued_actions = inner.actions.drain_actions(viewport_id);
        assert!(queued_actions.iter().any(|action| {
            matches!(
                action,
                InputAction::PointerMove { pos: queued_pos }
                    if queued_pos.x == pos.x && queued_pos.y == pos.y
            )
        }));
        assert!(queued_actions.iter().any(|action| {
            matches!(
                action,
                InputAction::PointerButton {
                    pos: queued_pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: queued_modifiers,
                } if queued_pos.x == pos.x && queued_pos.y == pos.y && queued_modifiers.shift
            )
        }));
        assert!(queued_actions.iter().any(|action| {
            matches!(
                action,
                InputAction::Key {
                    key: egui::Key::Enter,
                    pressed: true,
                    modifiers: queued_modifiers,
                } if queued_modifiers.shift
            )
        }));
        assert!(
            queued_actions
                .iter()
                .any(|action| matches!(action, InputAction::Text { text } if text == "Hello"))
        );
        assert!(queued_actions.iter().any(|action| {
            matches!(
                action,
                InputAction::Scroll {
                    delta,
                    modifiers: queued_modifiers,
                } if delta.x == 1.0 && delta.y == -2.0 && queued_modifiers.shift
            )
        }));

        let pending_commands = inner.actions.drain_all_commands();
        assert_eq!(pending_commands.len(), 1);
        assert_eq!(pending_commands[0].0, viewport_id);
        assert_eq!(pending_commands[0].1, vec![egui::ViewportCommand::Focus]);

        let highlight_rect = Rect {
            min: Pos2 { x: 1.0, y: 2.0 },
            max: Pos2 { x: 3.0, y: 4.0 },
        };
        let highlight_result = server
            .show_highlight(None, None, Some(highlight_rect), "#ff0000".to_string())
            .await
            .expect("show_highlight");
        assert_eq!(highlight_result.rect.min.x, 1.0);
        assert_eq!(highlight_result.rect.min.y, 2.0);
        assert_eq!(highlight_result.rect.max.x, 3.0);
        assert_eq!(highlight_result.rect.max.y, 4.0);
        server
            .hide_highlight(None, None)
            .await
            .expect("hide_highlight");
    }

    #[test]
    fn scroll_input_moves_scroll_area() {
        let inner = Arc::new(Inner::new());
        let viewport_id = egui::ViewportId::ROOT;
        let mut raw_input = egui::RawInput {
            viewport_id,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 400.0),
            )),
            ..Default::default()
        };
        inner.queue_action(
            viewport_id,
            InputAction::PointerMove {
                pos: Pos2 { x: 50.0, y: 50.0 },
            },
        );
        inner.queue_action(
            viewport_id,
            InputAction::Scroll {
                delta: Vec2 { x: 0.0, y: -120.0 },
                modifiers: Modifiers::default(),
            },
        );

        apply_actions(&inner, &mut raw_input);

        let ctx = egui::Context::default();
        let mut offset = egui::Vec2::ZERO;
        let _output = ctx.run(raw_input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let output = egui::ScrollArea::vertical()
                    .max_height(100.0)
                    .show(ui, |ui| {
                        for row in 0..100 {
                            ui.label(format!("Row {row}"));
                        }
                    });
                offset = output.state.offset;
            });
        });

        assert!(
            offset.y > 0.0,
            "expected scroll offset to move, got {offset:?}"
        );
    }

    #[tokio::test]
    async fn widget_list_filters_role_and_id_prefix() {
        let inner = Arc::new(Inner::new());
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        inner
            .widgets
            .record_widget(viewport_id, make_entry("alpha", 1, WidgetRole::Button));
        inner.widgets.record_widget(
            viewport_id,
            make_entry("filter.match", 2, WidgetRole::Slider),
        );
        inner.widgets.record_widget(
            viewport_id,
            make_entry("filter.skip", 3, WidgetRole::Button),
        );
        inner.widgets.finalize_registry(viewport_id);

        let server = DevMcpServer::new(Arc::clone(&inner));
        let result: Vec<WidgetRegistryEntry> = server
            .widget_list(
                None,
                Some(true),
                Some(WidgetRole::Slider),
                Some("filter.".to_string()),
            )
            .await
            .expect("widget list")
            .structured_as()
            .expect("widget list payload");
        let tags: Vec<_> = result.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(tags, vec!["filter.match"]);
    }

    #[tokio::test]
    async fn widget_list_includes_values() {
        let inner = Arc::new(Inner::new());
        let ctx = egui::Context::default();
        let raw_input = egui::RawInput::default();
        let mut checked = true;
        let mut intensity = 42.0_f32;
        let _output = ctx.run(raw_input, |ctx| {
            inner.capture_context(ctx.viewport_id(), ctx);
            inner.widgets.clear_registry(ctx.viewport_id());
            egui::CentralPanel::default().show(ctx, |ui| {
                let checkbox = ui.checkbox(&mut checked, "Enabled");
                let checkbox_visible = ui.is_visible() && ui.is_rect_visible(checkbox.rect);
                record_widget(
                    &inner.widgets,
                    "basic.enabled".to_string(),
                    &checkbox,
                    WidgetMeta {
                        role: WidgetRole::Checkbox,
                        label: Some("Enabled".to_string()),
                        value: Some(WidgetValue::Bool(checked)),
                        visible: checkbox_visible,
                        ..Default::default()
                    },
                );

                let slider = ui.add(egui::Slider::new(&mut intensity, 0.0..=100.0));
                let slider_visible = ui.is_visible() && ui.is_rect_visible(slider.rect);
                record_widget(
                    &inner.widgets,
                    "basic.intensity".to_string(),
                    &slider,
                    WidgetMeta {
                        role: WidgetRole::Slider,
                        value: Some(WidgetValue::Float(f64::from(intensity))),
                        visible: slider_visible,
                        ..Default::default()
                    },
                );
            });
            inner.widgets.finalize_registry(ctx.viewport_id());
        });

        let server = DevMcpServer::new(Arc::clone(&inner));
        let result: Vec<WidgetRegistryEntry> = server
            .widget_list(None, Some(true), None, None)
            .await
            .expect("widget list")
            .structured_as()
            .expect("widget list payload");
        let enabled = result
            .iter()
            .find(|entry| entry.id == "basic.enabled")
            .expect("enabled widget");
        let intensity_entry = result
            .iter()
            .find(|entry| entry.id == "basic.intensity")
            .expect("intensity widget");
        assert!(matches!(enabled.value, Some(WidgetValue::Bool(true))));
        assert!(matches!(
            intensity_entry.value,
            Some(WidgetValue::Float(value)) if (value - 42.0).abs() < f64::EPSILON
        ));
    }

    #[tokio::test]
    async fn action_click_click_count_queues_multiple_clicks() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        inner
            .widgets
            .record_widget(viewport_id, make_entry("click", 1, WidgetRole::Button));
        inner.widgets.finalize_registry(viewport_id);

        server
            .action_click(None, widget_ref_id("click"), None, None, Some(2))
            .await
            .expect("action click");

        let mut raw_input = egui::RawInput {
            viewport_id,
            ..Default::default()
        };
        apply_actions(&inner, &mut raw_input);
        let pointer_buttons = raw_input
            .events
            .iter()
            .filter(|event| matches!(event, egui::Event::PointerButton { .. }))
            .count();
        assert_eq!(pointer_buttons, 4);
    }

    #[tokio::test]
    async fn action_click_can_succeed_without_widget_state_change() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        let mut status = make_entry("status", 1, WidgetRole::Label);
        status.value = Some(WidgetValue::Text("stable".to_string()));
        inner.widgets.record_widget(viewport_id, status);
        inner.widgets.finalize_registry(viewport_id);

        let before = inner
            .widgets
            .resolve_widget(&inner.viewports, None, &widget_ref_id("status"))
            .expect("status before");
        server
            .action_click(None, widget_ref_id("status"), None, None, None)
            .await
            .expect("action click");

        let mut raw_input = egui::RawInput {
            viewport_id,
            ..Default::default()
        };
        apply_actions(&inner, &mut raw_input);
        assert!(
            raw_input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::PointerButton { .. }))
        );

        let after = inner
            .widgets
            .resolve_widget(&inner.viewports, None, &widget_ref_id("status"))
            .expect("status after");
        assert_eq!(before.value, after.value);
    }

    #[tokio::test]
    async fn action_key_injects_text_event() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        server
            .action_key(None, egui::Key::A, Modifiers::default(), "a", None)
            .await
            .expect("action key");

        let mut raw_input = egui::RawInput {
            viewport_id,
            ..Default::default()
        };
        apply_actions(&inner, &mut raw_input);
        assert!(
            raw_input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Text(text) if text == "a"))
        );
    }

    #[tokio::test]
    async fn action_paste_queues_paste_event() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        server
            .action_paste(None, "Hello".to_string())
            .await
            .expect("action paste");

        let mut raw_input = egui::RawInput {
            viewport_id,
            ..Default::default()
        };
        apply_actions(&inner, &mut raw_input);
        assert!(
            raw_input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Paste(text) if text == "Hello"))
        );
    }

    #[tokio::test]
    async fn action_hover_queues_pointer_move() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        inner
            .widgets
            .record_widget(viewport_id, make_entry("hover", 1, WidgetRole::Button));
        inner.widgets.finalize_registry(viewport_id);

        server
            .action_hover(None, widget_ref_id("hover"), None, Some(0))
            .await
            .expect("action hover");

        let mut raw_input = egui::RawInput {
            viewport_id,
            ..Default::default()
        };
        apply_actions(&inner, &mut raw_input);
        assert!(raw_input.events.iter().any(|event| {
            matches!(event, egui::Event::PointerMoved(pos)
                if (pos.x - 5.0).abs() < f32::EPSILON && (pos.y - 5.0).abs() < f32::EPSILON)
        }));
    }

    #[tokio::test]
    async fn wait_for_widget_state_matches_existing_widget() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        let mut entry = make_entry("status", 1, WidgetRole::Label);
        entry.value = Some(WidgetValue::Text("Ready".to_string()));
        entry.label = Some("Ready".to_string());
        inner.widgets.record_widget(viewport_id, entry);
        inner.widgets.finalize_registry(viewport_id);

        let result = server
            .wait_for_widget_state(None, widget_ref_id("status"), Some(50), Some(1), |widget| {
                widget.and_then(|widget| widget.value.as_ref())
                    == Some(&WidgetValue::Text("Ready".to_string()))
            })
            .await
            .expect("wait for widget");
        assert!(result.is_some());
    }

    #[tokio::test]
    async fn wait_for_widget_state_matches_missing_widget() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));

        let result = server
            .wait_for_widget_state(
                None,
                widget_ref_id("missing"),
                Some(50),
                Some(1),
                |widget| widget.is_none(),
            )
            .await
            .expect("wait for widget");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn wait_for_widget_state_matches_when_widget_becomes_visible() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        let mut entry = make_entry("status", 1, WidgetRole::Label);
        entry.visible = false;
        inner.widgets.record_widget(viewport_id, entry);
        inner.widgets.finalize_registry(viewport_id);

        let inner_for_update = Arc::clone(&inner);
        tokio::spawn(async move {
            sleep(Duration::from_millis(5)).await;
            inner_for_update.widgets.clear_registry(viewport_id);
            let mut entry = make_entry("status", 1, WidgetRole::Label);
            entry.visible = true;
            inner_for_update.widgets.record_widget(viewport_id, entry);
            inner_for_update.widgets.finalize_registry(viewport_id);
        });

        let result = server
            .wait_for_widget_state(
                None,
                widget_ref_id("status"),
                Some(100),
                Some(1),
                |widget| widget.is_some_and(|widget| widget.visible),
            )
            .await
            .expect("wait for visible widget");
        assert!(result.is_some_and(|widget| widget.visible));
    }

    #[tokio::test]
    async fn wait_for_widget_state_matches_when_widget_appears() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        inner.widgets.finalize_registry(viewport_id);

        let inner_for_update = Arc::clone(&inner);
        tokio::spawn(async move {
            sleep(Duration::from_millis(5)).await;
            inner_for_update.widgets.clear_registry(viewport_id);
            inner_for_update
                .widgets
                .record_widget(viewport_id, make_entry("status", 1, WidgetRole::Label));
            inner_for_update.widgets.finalize_registry(viewport_id);
        });

        let result = server
            .wait_for_widget_state(
                None,
                widget_ref_id("status"),
                Some(100),
                Some(1),
                |widget| widget.is_some(),
            )
            .await
            .expect("wait for appearing widget");
        assert!(result.is_some_and(|widget| widget.visible));
    }

    #[tokio::test]
    async fn wait_for_settle_matches_when_idle() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let ctx = egui::Context::default();
        let viewport_id = egui::ViewportId::ROOT;

        // Run one frame so that the input snapshot is populated.
        let raw_input = egui::RawInput {
            viewport_id,
            ..Default::default()
        };
        drop(ctx.run(raw_input, |_| {}));
        inner.capture_context(viewport_id, &ctx);
        inner.viewports.capture_input_snapshot(
            &ctx,
            inner.fixture_epoch(),
            inner.frame_count() + 1,
        );
        let runtime_for_frame = Runtime::ensure_for_inner(&inner);
        tokio::spawn(async move {
            yield_now().await;
            inner.advance_frame();
            runtime_for_frame.frame_notify().notify_waiters();
        });

        server
            .wait_for_settle(None, Some(50), Some(1))
            .await
            .expect("wait_for_settle");
    }

    #[tokio::test]
    async fn wait_for_settle_requires_a_new_frame() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let ctx = egui::Context::default();
        let viewport_id = egui::ViewportId::ROOT;

        let raw_input = egui::RawInput {
            viewport_id,
            ..Default::default()
        };
        drop(ctx.run(raw_input, |_| {}));
        inner.capture_context(viewport_id, &ctx);
        inner.viewports.capture_input_snapshot(
            &ctx,
            inner.fixture_epoch(),
            inner.frame_count() + 1,
        );

        let error = server
            .wait_for_settle(None, Some(10), Some(1))
            .await
            .expect_err("wait_for_settle should time out");
        assert_eq!(error.code, "timeout");
    }

    #[tokio::test]
    async fn widget_set_value_queues_update() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        let mut entry = make_entry("field", 1, WidgetRole::TextEdit);
        entry.value = Some(WidgetValue::Text("Before".to_string()));
        inner.widgets.record_widget(viewport_id, entry);
        inner.widgets.finalize_registry(viewport_id);

        server
            .widget_set_value(
                None,
                widget_ref_id("field"),
                WidgetValue::Text("After".to_string()),
            )
            .await
            .expect("widget set value");

        let updated = inner
            .take_widget_value_update(viewport_id, "field")
            .expect("queued update");
        assert!(matches!(updated, WidgetValue::Text(value) if value == "After"));
    }

    #[tokio::test]
    async fn widget_set_value_accepts_collapsing_header_bool() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        let mut entry = make_entry("advanced", 1, WidgetRole::CollapsingHeader);
        entry.value = Some(WidgetValue::Bool(false));
        inner.widgets.record_widget(viewport_id, entry);
        inner.widgets.finalize_registry(viewport_id);

        server
            .widget_set_value(None, widget_ref_id("advanced"), WidgetValue::Bool(true))
            .await
            .expect("widget set value");

        let updated = inner
            .take_widget_value_update(viewport_id, "advanced")
            .expect("queued update");
        assert_eq!(updated, WidgetValue::Bool(true));
    }

    #[tokio::test]
    async fn script_widget_parent_and_children_follow_hierarchy() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        inner.widgets.record_widget(
            viewport_id,
            make_entry_with_rect(
                "root",
                1,
                WidgetRole::Window,
                Rect {
                    min: Pos2 { x: 0.0, y: 0.0 },
                    max: Pos2 { x: 100.0, y: 100.0 },
                },
                None,
            ),
        );
        inner.widgets.record_widget(
            viewport_id,
            make_entry_with_rect(
                "child",
                2,
                WidgetRole::Button,
                Rect {
                    min: Pos2 { x: 0.0, y: 0.0 },
                    max: Pos2 { x: 10.0, y: 10.0 },
                },
                Some("root"),
            ),
        );
        inner.widgets.record_widget(
            viewport_id,
            make_entry_with_rect(
                "sibling",
                3,
                WidgetRole::Button,
                Rect {
                    min: Pos2 { x: 12.0, y: 0.0 },
                    max: Pos2 { x: 22.0, y: 10.0 },
                },
                Some("root"),
            ),
        );
        inner.widgets.record_widget(
            viewport_id,
            make_entry_with_rect(
                "grand",
                4,
                WidgetRole::Button,
                Rect {
                    min: Pos2 { x: 0.0, y: 12.0 },
                    max: Pos2 { x: 10.0, y: 22.0 },
                },
                Some("child"),
            ),
        );
        inner.widgets.finalize_registry(viewport_id);

        let result = server
            .script_eval(
                r#"
                    local vp = root()
                    local root = vp:widget_get("root")
                    local child = vp:widget_get("child")
                    local child_ids = {}
                    for _, widget in ipairs(root:children()) do
                        table.insert(child_ids, widget.id)
                    end
                    local grand_ids = {}
                    for _, widget in ipairs(child:children()) do
                        table.insert(grand_ids, widget.id)
                    end
                    local grand_parent = vp:widget_get("grand"):parent()
                    return {
                        child_count = #child_ids,
                        child_ids = table.concat(child_ids, ","),
                        grand_count = #grand_ids,
                        grand_ids = table.concat(grand_ids, ","),
                        grand_parent_child_count = grand_parent and #grand_parent:children() or 0,
                    }
                "#
                .to_string(),
                None,
                None,
            )
            .await
            .expect("script eval");
        let json = parse_script_eval_json(&result);
        assert_eq!(json["success"], true, "{json:?}");
        let value = &json["value"];
        assert_eq!(value.get("child_count").and_then(Value::as_u64), Some(2));
        assert_eq!(
            value.get("child_ids").and_then(Value::as_str),
            Some("child,sibling")
        );
        assert_eq!(value.get("grand_count").and_then(Value::as_u64), Some(1));
        assert_eq!(
            value.get("grand_ids").and_then(Value::as_str),
            Some("grand")
        );
        assert_eq!(
            value
                .get("grand_parent_child_count")
                .and_then(Value::as_u64),
            Some(1)
        );
    }

    #[tokio::test]
    async fn script_widget_state_and_wait_for_use_widget_state() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        let mut button = make_entry("status", 1, WidgetRole::Button);
        button.label = Some("Ready".to_string());
        button.value = Some(WidgetValue::Text("Ready".to_string()));
        button.focused = true;
        inner.widgets.record_widget(viewport_id, button);
        inner.widgets.finalize_registry(viewport_id);

        let result = server
            .script_eval(
                r#"
                    local widget = root():widget_get("status")
                    local state = widget:state()
                    local waited = widget:wait_for(function(current)
                        return current ~= nil
                            and current.focused
                            and current.value ~= nil
                            and current.value == "Ready"
                    end, { timeout_ms = 20, poll_interval_ms = 1 })
                    return {
                        role = state.role,
                        focused = state.focused,
                        value_text = state.value,
                        waited = waited ~= nil,
                        waited_role = waited ~= nil and waited.role or nil,
                    }
                "#
                .to_string(),
                None,
                None,
            )
            .await
            .expect("script eval");
        let json = parse_script_eval_json(&result);
        let value = &json["value"];
        assert_eq!(value.get("role").and_then(Value::as_str), Some("button"));
        assert_eq!(
            value.get("value_text").and_then(Value::as_str),
            Some("Ready")
        );
        assert_eq!(value.get("focused").and_then(Value::as_bool), Some(true));
        assert_eq!(value.get("waited").and_then(Value::as_bool), Some(true));
        assert_eq!(
            value.get("waited_role").and_then(Value::as_str),
            Some("button")
        );
    }

    #[tokio::test]
    async fn check_layout_reports_overlap() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        inner.widgets.record_widget(
            viewport_id,
            make_entry_with_rect(
                "first",
                1,
                WidgetRole::Button,
                Rect {
                    min: Pos2 { x: 0.0, y: 0.0 },
                    max: Pos2 { x: 10.0, y: 10.0 },
                },
                None,
            ),
        );
        inner.widgets.record_widget(
            viewport_id,
            make_entry_with_rect(
                "second",
                2,
                WidgetRole::Button,
                Rect {
                    min: Pos2 { x: 5.0, y: 5.0 },
                    max: Pos2 { x: 15.0, y: 15.0 },
                },
                None,
            ),
        );
        inner.widgets.finalize_registry(viewport_id);

        let result = server.check_layout(None, None).await.expect("layout check");
        assert!(
            result
                .iter()
                .any(|issue| matches!(issue.kind, LayoutIssueKind::Overlap))
        );
    }

    #[tokio::test]
    async fn text_measure_returns_text() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let ctx = egui::Context::default();
        let raw_input = egui::RawInput::default();
        let text = "Hello".to_string();
        let _output = ctx.run(raw_input, |ctx| {
            inner.capture_context(ctx.viewport_id(), ctx);
            inner.widgets.clear_registry(ctx.viewport_id());
            egui::CentralPanel::default().show(ctx, |ui| {
                let response = ui.label(&text);
                let visible = ui.is_visible() && ui.is_rect_visible(response.rect);
                record_widget(
                    &inner.widgets,
                    "label".to_string(),
                    &response,
                    WidgetMeta {
                        role: WidgetRole::Label,
                        label: Some(text.clone()),
                        value: Some(WidgetValue::Text(text.clone())),
                        visible,
                        ..Default::default()
                    },
                );
            });
            inner.widgets.finalize_registry(ctx.viewport_id());
        });

        let result = server
            .text_measure(widget_ref_id("label"))
            .await
            .expect("text measure");
        assert_eq!(result.text, "Hello");
        assert_eq!(result.text.chars().count(), 5);
        assert!(!result.lines.is_empty());
    }

    #[tokio::test]
    async fn text_measure_uses_widget_viewport_context() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let ctx = egui::Context::default();
        let secondary = egui::ViewportId::from_hash_of("secondary");
        for _ in 0..2 {
            let mut raw_input = egui::RawInput {
                viewport_id: egui::ViewportId::ROOT,
                ..Default::default()
            };
            raw_input
                .viewports
                .insert(egui::ViewportId::ROOT, Default::default());
            raw_input.viewports.insert(secondary, Default::default());
            drop(ctx.run(raw_input, |_| {}));
        }
        inner.capture_context(secondary, &ctx);
        inner.viewports.update_viewports(&ctx);

        inner.widgets.clear_registry(secondary);
        let mut entry = make_entry("label.secondary", 1, WidgetRole::Label);
        entry.viewport_id = viewport_id_to_string(secondary);
        entry.value = Some(WidgetValue::Text("Hello".to_string()));
        inner.widgets.record_widget(secondary, entry);
        inner.widgets.finalize_registry(secondary);

        let result = server
            .text_measure(WidgetRef {
                id: Some("label.secondary".to_string()),
                viewport_id: Some(viewport_id_to_string(secondary)),
            })
            .await
            .expect("text measure");
        assert_eq!(result.text, "Hello");
    }

    #[tokio::test]
    async fn check_layout_text_truncation_uses_scope_viewport_context() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let ctx = egui::Context::default();
        let secondary = egui::ViewportId::from_hash_of("secondary");
        for _ in 0..2 {
            let mut raw_input = egui::RawInput {
                viewport_id: egui::ViewportId::ROOT,
                ..Default::default()
            };
            raw_input
                .viewports
                .insert(egui::ViewportId::ROOT, Default::default());
            raw_input.viewports.insert(secondary, Default::default());
            drop(ctx.run(raw_input, |_| {}));
        }
        inner.capture_context(secondary, &ctx);
        inner.viewports.update_viewports(&ctx);

        inner.widgets.clear_registry(secondary);
        let mut entry = make_entry_with_rect(
            "label.secondary",
            1,
            WidgetRole::Label,
            Rect {
                min: Pos2 { x: 0.0, y: 0.0 },
                max: Pos2 { x: 20.0, y: 10.0 },
            },
            None,
        );
        entry.viewport_id = viewport_id_to_string(secondary);
        entry.value = Some(WidgetValue::Text("HelloWorld".to_string()));
        inner.widgets.record_widget(secondary, entry);
        inner.widgets.finalize_registry(secondary);

        let result = server
            .check_layout(Some(viewport_id_to_string(secondary)), None)
            .await
            .expect("layout check");
        assert!(result.iter().all(|issue| !issue.widgets.is_empty()));
    }

    #[tokio::test]
    async fn widget_at_point_reports_topmost() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        inner.widgets.record_widget(
            viewport_id,
            make_entry_with_rect(
                "bottom",
                1,
                WidgetRole::Button,
                Rect {
                    min: Pos2 { x: 0.0, y: 0.0 },
                    max: Pos2 { x: 10.0, y: 10.0 },
                },
                None,
            ),
        );
        inner.widgets.record_widget(
            viewport_id,
            make_entry_with_rect(
                "top",
                2,
                WidgetRole::Button,
                Rect {
                    min: Pos2 { x: 0.0, y: 0.0 },
                    max: Pos2 { x: 10.0, y: 10.0 },
                },
                None,
            ),
        );
        inner.widgets.finalize_registry(viewport_id);

        let result = server
            .widget_at_point(Pos2 { x: 5.0, y: 5.0 }, Some(true), None)
            .await
            .expect("widget at point");
        assert_eq!(result.widgets.len(), 2);
        assert_eq!(result.widgets[0].id, "top");
    }

    #[tokio::test]
    async fn check_layout_scopes_to_widget_subtree() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        inner
            .widgets
            .record_widget(viewport_id, make_entry("left", 1, WidgetRole::Window));
        inner.widgets.record_widget(
            viewport_id,
            make_entry_with_rect(
                "left.row",
                2,
                WidgetRole::Button,
                Rect {
                    min: Pos2 { x: 0.0, y: 0.0 },
                    max: Pos2 { x: 20.0, y: 10.0 },
                },
                Some("left"),
            ),
        );
        inner
            .widgets
            .record_widget(viewport_id, make_entry("right", 3, WidgetRole::Window));
        inner.widgets.record_widget(
            viewport_id,
            make_entry_with_rect(
                "right.row",
                4,
                WidgetRole::Button,
                Rect {
                    min: Pos2 { x: 50.0, y: 0.0 },
                    max: Pos2 { x: 80.0, y: 10.0 },
                },
                Some("right"),
            ),
        );
        inner.widgets.finalize_registry(viewport_id);

        let result = server
            .check_layout(None, Some(widget_ref_id("right.row")))
            .await
            .expect("layout check");
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn show_hide_debug_overlay() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let viewport_id = egui::ViewportId::ROOT;

        inner.widgets.clear_registry(viewport_id);
        inner
            .widgets
            .record_widget(viewport_id, make_entry("overlay", 1, WidgetRole::Button));
        inner.widgets.finalize_registry(viewport_id);

        server
            .show_debug_overlay(None, Some(OverlayDebugModeName::Bounds), None, None)
            .await
            .expect("show debug overlay");
        let config = inner.overlays.overlay_debug_config();
        assert!(config.enabled);
        assert_eq!(config.mode, OverlayDebugMode::Bounds);

        server
            .hide_debug_overlay()
            .await
            .expect("hide debug overlay");
        assert!(!inner.overlays.overlay_debug_config().enabled);
    }

    /// Verify that injecting Enter via action_key causes a singleline TextEdit
    /// to surrender focus, matching the standard egui pattern:
    ///   response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))
    /// Uses raw_input.focused = false on non-action frames to prove the
    /// mechanism works even when the window lacks OS focus.
    #[tokio::test]
    async fn action_key_enter_surrenders_focus_on_singleline() {
        let inner = Arc::new(Inner::new());
        let server = DevMcpServer::new(Arc::clone(&inner));
        let ctx = egui::Context::default();
        let viewport_id = egui::ViewportId::ROOT;
        let mut text = String::new();
        let mut enter_detected = false;

        let render_widget =
            |inner: &Inner, ctx: &egui::Context, text: &mut String, raw_input: egui::RawInput| {
                let output = ctx.run(raw_input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let response = ui.text_edit_singleline(text);
                        let visible = ui.is_visible() && ui.is_rect_visible(response.rect);
                        record_widget(
                            &inner.widgets,
                            "input".to_string(),
                            &response,
                            WidgetMeta {
                                role: WidgetRole::TextEdit,
                                value: Some(WidgetValue::Text(text.clone())),
                                visible,
                                ..Default::default()
                            },
                        );
                        response.lost_focus()
                    });
                });
                inner.widgets.finalize_registry(viewport_id);
                output
            };

        // Simulate no OS window focus — platform frames report focused=false.
        let no_focus_raw = || egui::RawInput {
            viewport_id,
            focused: false,
            ..Default::default()
        };

        // Frame 1: initial render (unfocused, no OS focus).
        inner.widgets.clear_registry(viewport_id);
        render_widget(&inner, &ctx, &mut text, no_focus_raw());

        // Type text (queues click-to-focus + text input).
        server
            .action_type(
                None,
                widget_ref_id("input"),
                "hello".to_string(),
                None,
                None,
            )
            .await
            .expect("type action");

        // Frame 2: apply click-to-focus.
        let mut raw = no_focus_raw();
        apply_actions(&inner, &mut raw);
        render_widget(&inner, &ctx, &mut text, raw);

        // Frame 3: apply staged text input after focus is established.
        let mut raw = no_focus_raw();
        apply_actions(&inner, &mut raw);
        render_widget(&inner, &ctx, &mut text, raw);
        assert_eq!(text, "hello");

        // Widget should report focused=true via egui memory focus even when
        // raw_input.focused was forced only for the action frame.
        let widget = inner
            .widgets
            .resolve_widget(&inner.viewports, None, &widget_ref_id("input"))
            .expect("widget should exist");
        assert!(widget.focused, "widget should have egui memory focus");

        // Frame 4: no actions, no OS focus — the settle frame.
        // Widget must retain egui memory focus even though the window
        // doesn't have OS focus.
        render_widget(&inner, &ctx, &mut text, no_focus_raw());
        let widget = inner
            .widgets
            .resolve_widget(&inner.viewports, None, &widget_ref_id("input"))
            .expect("widget should exist");
        assert!(
            widget.focused,
            "widget must retain memory focus across settle frame without OS focus"
        );

        // Queue Enter key.
        server
            .action_key(None, egui::Key::Enter, Modifiers::default(), "Enter", None)
            .await
            .expect("key action");

        // Frame 5: apply Enter and detect lost_focus + key_pressed(Enter).
        let mut raw = no_focus_raw();
        apply_actions(&inner, &mut raw);
        let _output = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let response = ui.text_edit_singleline(&mut text);
                let visible = ui.is_visible() && ui.is_rect_visible(response.rect);
                record_widget(
                    &inner.widgets,
                    "input".to_string(),
                    &response,
                    WidgetMeta {
                        role: WidgetRole::TextEdit,
                        value: Some(WidgetValue::Text(text.clone())),
                        visible,
                        ..Default::default()
                    },
                );
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    enter_detected = true;
                }
            });
        });
        inner.widgets.finalize_registry(viewport_id);

        assert!(
            enter_detected,
            "Enter key should cause lost_focus + key_pressed(Enter)"
        );
    }
}
