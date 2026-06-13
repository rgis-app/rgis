//! Native embedded runtime for `eguidev`.
//!
//! `eguidev_runtime` attaches the in-process automation server, script
//! evaluation, screenshots, and smoke runner to an inert [`eguidev::DevMcp`]
//! handle.
//!
//! For `eframe` applications, the most reliable integration pattern is:
//!
//! - choose `eframe::Renderer::Glow` for automation runs when possible
//! - register a fixture handler with [`eguidev::DevMcp::on_fixture`]
//! - wrap every frame in [`eguidev::FrameGuard`]
//! - forward raw input through [`eguidev::raw_input_hook`]
//!
//! The `wgpu` backend can exhibit idle-frame stalls in some `eframe`
//! integrations, so the demo and examples prefer `Glow`.

#![allow(clippy::missing_docs_in_private_items)]

#[cfg(target_arch = "wasm32")]
compile_error!("eguidev_runtime is native-only and is not supported on wasm32 targets");

mod error;
mod runtime;
mod screenshots;
mod script_docs;
mod server;
pub mod smoke;
mod tools;

pub(crate) mod actions {
    pub use eguidev::internal::actions::*;
}

pub(crate) mod overlay {
    pub use eguidev::internal::overlay::*;
}

pub(crate) mod registry {
    pub use eguidev::internal::registry::*;
}

pub(crate) mod tree {
    pub use eguidev::internal::tree::*;
}

pub(crate) mod types {
    pub use eguidev::internal::types::*;
}

pub(crate) mod ui_ext {
    pub use eguidev::internal::ui_ext::*;
}

pub(crate) mod viewports {
    pub use eguidev::internal::viewports::*;
}

pub use eguidev::{DevMcp, ScrollAreaMeta};

pub use crate::{
    runtime::{attach, eval_script},
    script_docs::{render_script_docs_markdown, script_definitions},
    tools::{
        ScriptArgValue, ScriptArgs, ScriptAssertion, ScriptErrorInfo, ScriptEvalOptions,
        ScriptEvalOutcome, ScriptEvalRequest, ScriptImageInfo, ScriptLocation, ScriptTiming,
    },
};

#[cfg(test)]
pub(crate) mod widget_registry {
    pub use eguidev::internal::widget_registry::*;
}
