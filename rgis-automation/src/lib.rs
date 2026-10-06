//! Drive rgis programmatically and inspect what it's doing.
//!
//! * [`command`]: the command vocabulary (`{"cmd": "load_file", ...}`) used by
//!   `--script`, the wasm `dispatch` export, and tests.
//! * [`runner`]: runs commands against the world, one step per frame, with
//!   `wait_idle` / `frames` steps for anything asynchronous.
//! * [`state`]: [`AppState`], a JSON snapshot of layers, CRS, camera, windows,
//!   selection, errors, and in-flight work.
//!
//! See `docs/automation.md` for the user-facing guide.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use bevy::prelude::*;

pub mod command;
pub mod idle;
pub mod runner;
pub mod state;

pub use command::{parse_script, Command};
pub use idle::IdleTracker;
pub use runner::{enqueue, ScriptRunner};
pub use state::{app_state, AppState};

/// How many entries of each kind [`AutomationLog`] keeps.
const MAX_LOG_ENTRIES: usize = 50;

/// User-facing messages and command errors, for [`AppState`].
#[derive(Resource, Default, Debug)]
pub struct AutomationLog {
    messages: VecDeque<String>,
    command_errors: VecDeque<String>,
}

impl AutomationLog {
    pub fn messages(&self) -> impl Iterator<Item = &str> {
        self.messages.iter().map(String::as_str)
    }

    pub fn command_errors(&self) -> impl Iterator<Item = &str> {
        self.command_errors.iter().map(String::as_str)
    }

    fn push_message(&mut self, message: String) {
        push_capped(&mut self.messages, message);
    }

    fn push_command_error(&mut self, error: String) {
        push_capped(&mut self.command_errors, error);
    }
}

fn push_capped(entries: &mut VecDeque<String>, entry: String) {
    if entries.len() >= MAX_LOG_ENTRIES {
        entries.pop_front();
    }
    entries.push_back(entry);
}

/// The message window drains `RenderTextMessage`s in `PostUpdate`, so copy
/// them at the end of `Update`.
fn capture_messages(
    mut reader: MessageReader<rgis_ui_messages::RenderTextMessage>,
    mut log: ResMut<AutomationLog>,
) {
    for message in reader.read() {
        log.push_message(message.0.clone());
    }
}

/// The latest [`AppState`], for callers outside the schedule (the wasm
/// `get_app_state` export). Only filled in by [`cache_latest_state`].
static LATEST_STATE: Mutex<Option<AppState>> = Mutex::new(None);

/// Whether anyone has asked for the state yet. Until then
/// [`cache_latest_state`] does nothing, so an app nobody inspects (e.g.
/// rgis.app for regular visitors) doesn't pay for a snapshot every frame.
static STATE_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Exclusive system that snapshots the state into [`latest_state_json`] at
/// the end of every frame, once the state has been requested. Registered on
/// wasm, where JS can't reach the `World`.
pub fn cache_latest_state(world: &mut World) {
    if !STATE_REQUESTED.load(Ordering::Relaxed) {
        return;
    }
    let state = app_state(world);
    if let Ok(mut latest) = LATEST_STATE.lock() {
        *latest = Some(state);
    }
}

/// The state cached by [`cache_latest_state`] as JSON, accounting for
/// commands queued with [`enqueue`] since then.
///
/// Returns `None` until a frame has run since the first call: that call is
/// what turns on the per-frame snapshot, so callers should poll.
pub fn latest_state_json() -> Option<Result<String, String>> {
    STATE_REQUESTED.store(true, Ordering::Relaxed);
    let mut state = LATEST_STATE.lock().ok()?.clone()?;
    let queued = runner::inbox_len();
    if queued > 0 {
        state.script.pending_steps += queued;
        state.idle = false;
    }
    Some(serde_json::to_string(&state).map_err(|e| e.to_string()))
}

/// Parse `json` (one step or an array of steps) and queue it to run.
/// Returns how many steps were queued.
pub fn dispatch_json(json: &str) -> Result<usize, String> {
    // Callers will want to see the results.
    STATE_REQUESTED.store(true, Ordering::Relaxed);
    let commands = parse_script(json)?;
    let count = commands.len();
    enqueue(commands);
    Ok(count)
}

/// Runs [`ScriptRunner`] steps, tracks idleness, and collects messages.
pub struct Plugin;

impl bevy::app::Plugin for Plugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AutomationLog>()
            .init_resource::<IdleTracker>()
            .init_resource::<ScriptRunner>()
            .add_systems(PreUpdate, runner::run_script)
            .add_systems(
                Update,
                capture_messages.after(rgis_primitives::RgisSet::Camera),
            )
            .add_systems(Last, idle::track_idle);

        #[cfg(target_arch = "wasm32")]
        app.add_systems(Last, cache_latest_state.after(idle::track_idle));
    }
}
