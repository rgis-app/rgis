//! Whether the app has settled: no background jobs, camera flights, or fades.

use bevy::prelude::*;

/// Consecutive quiet frames before the app counts as idle. A finished job is
/// picked up a frame after its entity disappears, and its follow-up job (e.g.
/// load → reproject → build meshes) is spawned in that later frame, so one
/// quiet frame isn't enough.
const IDLE_FRAMES: u32 = 3;

/// Counts consecutive frames with nothing in flight.
#[derive(Resource, Default, Debug)]
pub struct IdleTracker {
    quiet_frames: u32,
}

impl IdleTracker {
    pub fn is_idle(&self) -> bool {
        self.quiet_frames >= IDLE_FRAMES
    }
}

/// Describes what's keeping the app busy, or `None` if nothing is.
pub(crate) fn busy_reason(world: &mut World) -> Option<String> {
    let jobs: Vec<String> = world
        .query::<&bevy_jobs::InProgressJob>()
        .iter(world)
        .map(|job| job.name.clone())
        .collect();
    let flights = world
        .query_filtered::<(), With<rgis_camera::fly_to::CameraFlyTo>>()
        .iter(world)
        .count();
    let fades = rgis_renderer::ACTIVE_FADE_COUNT.load(std::sync::atomic::Ordering::Relaxed);

    let mut reasons = vec![];
    if !jobs.is_empty() {
        reasons.push(format!(
            "{} job(s) running: {}",
            jobs.len(),
            jobs.join(", ")
        ));
    }
    if flights > 0 {
        reasons.push("camera is still flying".to_string());
    }
    if fades > 0 {
        reasons.push(format!("{fades} fade animation(s) running"));
    }
    (!reasons.is_empty()).then(|| reasons.join("; "))
}

pub(crate) fn track_idle(world: &mut World) {
    let busy = busy_reason(world).is_some();
    if let Some(mut tracker) = world.get_resource_mut::<IdleTracker>() {
        tracker.quiet_frames = if busy {
            0
        } else {
            tracker.quiet_frames.saturating_add(1)
        };
    }
}
