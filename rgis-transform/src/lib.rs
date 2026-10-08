mod jobs;
mod systems;

pub struct Plugin;

impl bevy::app::Plugin for Plugin {
    fn build(&self, app: &mut bevy::app::App) {
        systems::configure(app);
    }
}

#[cfg(test)]
mod alignment_tests;
