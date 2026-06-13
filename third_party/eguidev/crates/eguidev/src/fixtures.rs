//! Fixture management for test data injection.

use std::sync::{Arc, Mutex};

use crate::{registry::lock, types::FixtureSpec};

/// Handler that applies a named fixture to the app state.
pub type FixtureHandler = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// Manages fixture metadata and dispatches fixture application through a registered handler.
pub struct FixtureManager {
    fixtures: Mutex<Vec<FixtureSpec>>,
    handler: Mutex<Option<FixtureHandler>>,
}

impl FixtureManager {
    /// Create a new fixture manager with no fixtures or handler.
    pub fn new() -> Self {
        Self {
            fixtures: Mutex::new(Vec::new()),
            handler: Mutex::new(None),
        }
    }

    /// Replace the registered fixture catalog.
    pub fn set_fixtures(&self, fixtures: Vec<FixtureSpec>) {
        for fixture in &fixtures {
            if let Err(error) = fixture.validate(true) {
                panic!("invalid fixture {}: {error}", fixture.name);
            }
        }
        let mut stored = lock(&self.fixtures, "fixtures lock");
        *stored = fixtures;
    }

    /// Return a snapshot of the current fixture catalog.
    pub fn fixtures(&self) -> Vec<FixtureSpec> {
        lock(&self.fixtures, "fixtures lock").clone()
    }

    /// Return the fixture catalog sorted by name.
    pub fn fixtures_sorted(&self) -> Vec<FixtureSpec> {
        let mut fixtures = self.fixtures();
        fixtures.sort_by(|a, b| a.name.cmp(&b.name));
        fixtures
    }

    /// Check whether a fixture with the given name is registered.
    pub fn has_fixture(&self, name: &str) -> bool {
        self.fixture(name).is_some()
    }

    /// Return a registered fixture by name.
    pub fn fixture(&self, name: &str) -> Option<FixtureSpec> {
        lock(&self.fixtures, "fixtures lock")
            .iter()
            .find(|fixture| fixture.name == name)
            .cloned()
    }

    /// Register the callback that applies a named fixture to app state.
    pub fn set_fixture_handler(&self, handler: FixtureHandler) {
        *lock(&self.handler, "fixture handler lock") = Some(handler);
    }

    /// Apply a named fixture by calling the registered handler.
    pub fn apply_fixture(&self, name: &str) -> Result<(), String> {
        if !self.has_fixture(name) {
            return Err(format!("unknown fixture: {name}"));
        }
        let handler = lock(&self.handler, "fixture handler lock").clone();
        match handler {
            Some(handler) => handler(name),
            None => Err("no fixture handler registered".to_string()),
        }
    }
}
