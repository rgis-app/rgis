//! Adapter wiring eguidev (an egui automation/testing framework) into rgis's
//! bevy_egui-driven UI. Native-only and gated behind the `eguidev` feature.
//!
//! eguidev was built for eframe, where one thread owns the whole egui frame.
//! bevy_egui instead runs egui inside Bevy's schedule, so we reproduce the
//! eframe lifecycle with three hooks:
//!
//! * `raw_input_system` — runs in `PreUpdate` between bevy_egui assembling
//!   `EguiInput` and `begin_pass`, forwarding/injecting automation input
//!   (the equivalent of eframe's `App::raw_input_hook`).
//! * `begin_frame_system` / `end_frame_system` — bracket rgis's UI systems
//!   inside `EguiPrimaryContextPass`, driving eguidev's per-frame widget
//!   registry capture.
//!
//! eguidev tracks the active frame via a thread-local, so the egui pass is
//! forced onto a single-threaded executor to keep begin/UI/end on one thread.

use bevy::prelude::*;
use bevy_egui::{
    EguiContext, EguiContexts, EguiInput, EguiPreUpdateSet, EguiPrimaryContextPass,
    PrimaryEguiContext,
};

/// Bevy resource holding the eguidev `DevMcp` handle (clonable, `Send + Sync`).
#[derive(Resource)]
pub struct DevMcpRes(pub eguidev::DevMcp);

/// Forward Bevy-derived input through eguidev and inject queued automation
/// input, before bevy_egui consumes it in `begin_pass`.
pub fn raw_input_system(
    devmcp: Res<DevMcpRes>,
    mut query: Query<(&mut EguiContext, &mut EguiInput), With<PrimaryEguiContext>>,
) {
    for (mut ctx, mut input) in &mut query {
        let egui_ctx = ctx.get_mut().clone();
        eguidev::raw_input_hook(&devmcp.0, &egui_ctx, &mut input.0);
    }
}

/// Begin the eguidev frame (enables widget tracking for this pass).
pub fn begin_frame_system(devmcp: Res<DevMcpRes>, mut contexts: EguiContexts) -> Result {
    let ctx = contexts.ctx_mut()?.clone();
    devmcp.0.begin_frame(&ctx);
    Ok(())
}

/// End the eguidev frame (finalizes the widget registry, runs automation).
pub fn end_frame_system(devmcp: Res<DevMcpRes>, mut contexts: EguiContexts) -> Result {
    let ctx = contexts.ctx_mut()?.clone();
    devmcp.0.end_frame(&ctx);
    Ok(())
}

/// Install the resource, the embedded runtime/MCP server, the single-threaded
/// egui pass executor, and the `PreUpdate` input hook. Frame begin/end systems
/// are registered by `systems::configure` so they can be ordered around rgis's
/// `RenderSystemSet` chain.
pub fn install(app: &mut App) {
    let devmcp = eguidev_runtime::attach(eguidev::DevMcp::new());
    app.insert_resource(DevMcpRes(devmcp));

    app.edit_schedule(EguiPrimaryContextPass, |schedule| {
        schedule.set_executor_kind(bevy::ecs::schedule::ExecutorKind::SingleThreaded);
    });

    app.add_systems(
        PreUpdate,
        raw_input_system
            .after(EguiPreUpdateSet::ProcessInput)
            .before(EguiPreUpdateSet::BeginPass),
    );

    info!("eguidev adapter installed; MCP server attached on stdio");
}
