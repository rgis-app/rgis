use serde_json::Value;

use super::types::ImageReferenceCollector;

pub(super) fn collect_image_refs(value: &Value, collector: &mut ImageReferenceCollector) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_image_refs(value, collector);
            }
        }
        Value::Object(map) => {
            let is_image_ref = map
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind == "image_ref");
            if is_image_ref && let Some(id) = map.get("id").and_then(Value::as_str) {
                collector.record(id);
            }
            for value in map.values() {
                collect_image_refs(value, collector);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}
