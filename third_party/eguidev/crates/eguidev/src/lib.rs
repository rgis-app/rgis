//! Cross-target instrumentation for AI-assisted egui development.
//!
//! `eguidev` captures widget state at frame boundaries and injects input
//! through egui's `raw_input_hook`, keeping automation aligned with the app's
//! real event loop. This crate is the instrumentation half of the automation
//! stack: it is valid for native and `wasm32` builds, and it intentionally does
//! not ship the embedded script runtime, MCP server, or screenshot machinery.
//!
//! # Quick start
//!
//! Add a [`DevMcp`] handle to your app state, wrap each frame with
//! [`FrameGuard`], and forward raw input to [`raw_input_hook`]. The handle
//! stays inert until a native app opts into `eguidev_runtime` and attaches the
//! embedded runtime in one bootstrap location.
//!
//! ```rust
//! use eframe::{App, egui};
//! use eguidev::{DevMcp, DevUiExt, FrameGuard};
//!
//! struct MyApp {
//!     devmcp: DevMcp,
//!     name: String,
//! }
//!
//! impl MyApp {
//!     fn new() -> Self {
//!         Self {
//!             devmcp: DevMcp::new(),
//!             name: String::new(),
//!         }
//!     }
//! }
//!
//! impl App for MyApp {
//!     fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
//!         let ctx = ui.ctx().clone();
//!         let _guard = FrameGuard::new(&self.devmcp, &ctx);
//!         egui::Frame::central_panel(ui.style()).show(ui, |ui| {
//!             ui.dev_text_edit("app.name", &mut self.name);
//!             if ui.dev_button("app.submit", "Submit").clicked() {
//!                 // ...
//!             }
//!         });
//!     }
//!
//!     fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
//!         eguidev::raw_input_hook(&self.devmcp, ctx, raw_input);
//!     }
//! }
//! ```
//!
//! # Build modes
//!
//! - **Cross-target instrumentation**: depend on `eguidev` only. [`DevMcp`],
//!   [`FrameGuard`], [`raw_input_hook`], widget tagging, and fixtures all
//!   compile for native and `wasm32` targets.
//! - **Native embedded runtime**: add an app-local feature that enables the
//!   optional `eguidev_runtime` dependency, then call
//!   `eguidev_runtime::attach(devmcp)` in one bootstrap location. Keep that
//!   branch local to startup code instead of pushing `#[cfg]` into widget code.
//!
//! # Instrumenting widgets
//!
//! Use the [`DevUiExt`] trait for standard widgets. Each `dev_*` method takes
//! an explicit string id and auto-populates role, label, and value metadata:
//!
//! ```rust,ignore
//! ui.dev_button("settings.save", "Save");
//! ui.dev_text_edit("settings.name", &mut name);
//! ui.dev_checkbox("settings.enabled", &mut enabled, "Enabled");
//! ui.dev_slider("settings.level", &mut level, 0.0..=100.0);
//! ```
//!
//! For custom widgets, use [`id`] (geometry only) or [`id_with_meta`] (explicit
//! role/value/label). If you already have an `egui::Response`, use
//! [`track_response_full`] to register it after the fact. Use [`container`] to
//! annotate hierarchy so scripts can traverse parent/child relationships.
//!
//! Custom widgets that should accept scripted `set_value(...)` calls must also
//! consume queued overrides before rendering:
//!
//! ```rust,ignore
//! let id = "settings.mode";
//! if let Some(crate::WidgetValue::Int(index)) = take_widget_value_override(ui, id) {
//!     *selected = index as usize;
//! }
//!
//! let response = render_custom_mode_picker(ui, selected);
//! track_response_full(
//!     id,
//!     &response,
//!     WidgetMeta {
//!         role: WidgetRole::ComboBox,
//!         value: Some(WidgetValue::Int(*selected as i64)),
//!         role_state: Some(RoleState::ComboBox {
//!             options: mode_labels.clone(),
//!         }),
//!         ..Default::default()
//!     },
//! );
//! ```
//!
//! Widget ids are the one canonical selector in the scripting API. Explicit ids
//! must be unique within a captured frame; duplicates are treated as a hard
//! automation fault.
//!
//! # Fixtures
//!
//! Apps register fixtures with [`DevMcp::fixtures`] and a handler callback
//! with [`DevMcp::on_fixture`]. Each fixture must be independently invokable
//! from any prior state and must leave the app in a baseline that can be
//! described with readiness anchors. Scripts call `fixture("name")` to apply
//! the named baseline, wait for fresh captures, and verify those anchors
//! before returning. Widget and viewport handles resolve fresh across fixture
//! boundaries, so rebinding `root()` after each fixture is usually unnecessary.
//! Use `fixture_raw("name")` only when you explicitly want the old
//! fire-and-forget behavior for debugging or manual setup flows.
//!
//! # Scripting reference
//!
//! The canonical Luau API, direct script evaluation helpers, and the smoketest
//! runner all live in `eguidev_runtime`. `edev` serves those checked-in
//! definitions through `script_api` and `edev --script-docs`.

#![allow(clippy::missing_docs_in_private_items)]

mod actions;
mod devmcp;
mod error;
mod fixtures;
mod instrument;
mod overlay;
mod registry;
mod tree;
pub(crate) mod types;
mod ui_ext;
mod viewports;
mod widget_registry;

pub use crate::{
    devmcp::{DevMcp, FrameGuard, raw_input_hook},
    fixtures::FixtureHandler,
    instrument::{
        ContainerGuard, ScrollAreaState, container, id, id_with_meta, track_response_full,
    },
    types::{
        Anchor, AnchorCheck, FixtureSpec, RoleState, ScrollAreaMeta, WidgetLayout, WidgetRange,
        WidgetRole, WidgetState, WidgetValue,
    },
    ui_ext::{
        ButtonOptions, CheckboxOptions, DevScrollAreaExt, DevUiExt, ProgressBarOptions,
        TextEditOptions, take_widget_value_override,
    },
    widget_registry::WidgetMeta,
};
#[doc(hidden)]
pub mod internal {
    pub mod actions {
        pub use crate::actions::{ActionQueue, ActionTiming, InputAction};
    }

    pub mod devmcp {
        pub use crate::devmcp::RuntimeHooks;
    }

    pub mod error {
        pub use crate::error::{ErrorCode, ToolError};
    }

    pub mod overlay {
        pub use crate::overlay::{
            OverlayDebugConfig, OverlayDebugMode, OverlayDebugOptions, OverlayEntry,
            OverlayManager, parse_color, rect_intersection, rect_size,
        };
    }

    pub mod registry {
        pub use crate::registry::{Inner, lock, viewport_id_to_string};
    }

    pub mod tree {
        pub use crate::tree::collect_subtree;
    }

    pub mod types {
        pub use crate::types::{
            Anchor, AnchorCheck, FixtureSpec, Modifiers, Pos2, Rect, RoleState, ScrollAreaMeta,
            Vec2, WidgetLayout, WidgetRange, WidgetRef, WidgetRegistryEntry, WidgetRole,
            WidgetState, WidgetValue,
        };
    }

    pub mod ui_ext {
        pub use crate::ui_ext::parse_color_hex;
    }

    pub mod viewports {
        pub use crate::viewports::{InputSnapshot, ViewportSnapshot, ViewportState};
    }

    pub mod widget_registry {
        pub use crate::widget_registry::{WidgetMeta, WidgetRegistry, record_widget};
    }
}
