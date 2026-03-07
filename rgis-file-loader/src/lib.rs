use bevy::prelude::*;

mod jobs;
pub mod lod_builder;
mod systems;

pub use lod_builder::{LODBuilderJob, LODBuilderJobOutcome};

pub struct Plugin;

impl bevy::app::Plugin for Plugin {
    fn build(&self, app: &mut App) {
        systems::configure(app);
    }
}
