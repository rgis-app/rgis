//! Inspect what rgis is doing without looking at pixels.
//!
//! * [`state`]: [`AppState`], a JSON snapshot of layers, CRS, camera, windows,
//!   selection, errors, and in-flight work.
//! * [`idle`]: whether the app has settled (no jobs, flights, or fades).
//!
//! See `docs/automation.md` for the user-facing guide.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use bevy::prelude::*;

pub mod idle;
pub mod state;

pub use idle::IdleTracker;
pub use state::{app_state, AppState};

/// How many messages [`AutomationLog`] keeps.
const MAX_LOG_ENTRIES: usize = 50;

/// User-facing messages, for [`AppState`].
#[derive(Resource, Default, Debug)]
pub struct AutomationLog {
    messages: VecDeque<String>,
}

impl AutomationLog {
    pub fn messages(&self) -> impl Iterator<Item = &str> {
        self.messages.iter().map(String::as_str)
    }

    fn push_message(&mut self, message: String) {
        push_capped(&mut self.messages, message);
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

/// The state cached by [`cache_latest_state`] as JSON.
///
/// Returns `None` until a frame has run since the first call: that call is
/// what turns on the per-frame snapshot, so callers should poll.
pub fn latest_state_json() -> Option<Result<String, String>> {
    STATE_REQUESTED.store(true, Ordering::Relaxed);
    let state = LATEST_STATE.lock().ok()?.clone()?;
    Some(serde_json::to_string(&state).map_err(|e| e.to_string()))
}

/// Tracks idleness and collects messages for [`AppState`].
pub struct Plugin;

impl bevy::app::Plugin for Plugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AutomationLog>()
            .init_resource::<IdleTracker>()
            .add_systems(
                Update,
                capture_messages.after(rgis_primitives::RgisSet::Camera),
            )
            .add_systems(Last, idle::track_idle);

        #[cfg(target_arch = "wasm32")]
        app.add_systems(Last, cache_latest_state.after(idle::track_idle));
    }
}
