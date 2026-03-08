use bevy::prelude::*;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

const MAX_LOG_ENTRIES: usize = 500;

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub level: tracing::Level,
    pub target: String,
    pub message: String,
}

#[derive(Resource, Clone)]
pub struct LogBuffer(pub Arc<Mutex<VecDeque<LogEntry>>>);

impl Default for LogBuffer {
    fn default() -> Self {
        LogBuffer(Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LOG_ENTRIES))))
    }
}

pub struct EguiLogLayer {
    buffer: Arc<Mutex<VecDeque<LogEntry>>>,
}

impl EguiLogLayer {
    pub fn new(buffer: Arc<Mutex<VecDeque<LogEntry>>>) -> Self {
        Self { buffer }
    }
}

impl<S: tracing::Subscriber> Layer<S> for EguiLogLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let mut visitor = MessageVisitor(String::new());
        event.record(&mut visitor);

        let entry = LogEntry {
            level: *metadata.level(),
            target: metadata.target().to_string(),
            message: visitor.0,
        };

        if let Ok(mut buf) = self.buffer.lock() {
            if buf.len() >= MAX_LOG_ENTRIES {
                buf.pop_front();
            }
            buf.push_back(entry);
        }
    }
}

struct MessageVisitor(String);

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{:?}", value);
        } else if !self.0.is_empty() {
            self.0.push_str(&format!(" {}={:?}", field.name(), value));
        } else {
            self.0 = format!("{}={:?}", field.name(), value);
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_string();
        } else if !self.0.is_empty() {
            self.0.push_str(&format!(" {}={}", field.name(), value));
        } else {
            self.0 = format!("{}={}", field.name(), value);
        }
    }
}

/// Creates a `bevy_log::BoxedLayer` for use with `LogPlugin::custom_layer`.
///
/// Call this from the `custom_layer` closure, passing the `App` so the
/// `LogBuffer` resource can be inserted.
pub fn create_log_layer(app: &mut bevy::app::App) -> Option<bevy_log::BoxedLayer> {
    let buffer = LogBuffer::default();
    let layer = EguiLogLayer::new(Arc::clone(&buffer.0));
    app.insert_resource(buffer);
    Some(Box::new(layer))
}
