#![allow(clippy::result_large_err)]

use serde_json::{Map, Value};

use super::{
    super::{OverlayDebugModeName, PointerButtonName, ScrollAlign},
    types::ScriptErrorInfo,
};
use crate::types::{Modifiers, Pos2, Rect, Vec2, WidgetRef, WidgetRole, WidgetValue};

pub(super) fn parse_f32(value: &Value) -> Result<f32, ScriptErrorInfo> {
    value
        .as_f64()
        .map(|value| value as f32)
        .ok_or_else(|| ScriptErrorInfo {
            error_type: "type_error".to_string(),
            message: "expected number".to_string(),
            location: None,
            backtrace: None,
            code: None,
            details: None,
        })
}

pub(super) fn parse_pos2(value: &Value) -> Result<Pos2, ScriptErrorInfo> {
    let map = as_object(value, "expected Pos2")?;
    let x = map
        .get("x")
        .ok_or_else(|| type_error("Pos2.x is missing"))?;
    let y = map
        .get("y")
        .ok_or_else(|| type_error("Pos2.y is missing"))?;
    Ok(Pos2 {
        x: parse_f32(x)?,
        y: parse_f32(y)?,
    })
}

pub(super) fn parse_vec2(value: &Value) -> Result<Vec2, ScriptErrorInfo> {
    let map = as_object(value, "expected Vec2")?;
    let x = map
        .get("x")
        .ok_or_else(|| type_error("Vec2.x is missing"))?;
    let y = map
        .get("y")
        .ok_or_else(|| type_error("Vec2.y is missing"))?;
    Ok(Vec2 {
        x: parse_f32(x)?,
        y: parse_f32(y)?,
    })
}

pub(super) fn parse_rect(value: &Value) -> Result<Rect, ScriptErrorInfo> {
    let map = as_object(value, "expected Rect")?;
    let min = map
        .get("min")
        .ok_or_else(|| type_error("Rect.min is missing"))?;
    let max = map
        .get("max")
        .ok_or_else(|| type_error("Rect.max is missing"))?;
    Ok(Rect {
        min: parse_pos2(min)?,
        max: parse_pos2(max)?,
    })
}

pub(super) fn map_value<'a>(map: Option<&'a Map<String, Value>>, key: &str) -> Option<&'a Value> {
    map.and_then(|map| map.get(key))
}

pub(super) fn map_has_any(map: &Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter().any(|key| map.contains_key(*key))
}

pub(super) fn parse_optional_string(
    map: Option<&Map<String, Value>>,
    key: &str,
) -> Result<Option<String>, ScriptErrorInfo> {
    match map_value(map, key) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(ToOwned::to_owned)
            .ok_or_else(|| type_error(format!("{key} must be a string")))
            .map(Some),
    }
}

pub(super) fn parse_optional_bool(
    map: Option<&Map<String, Value>>,
    key: &str,
) -> Result<Option<bool>, ScriptErrorInfo> {
    match map_value(map, key) {
        None => Ok(None),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| type_error(format!("{key} must be a boolean")))
            .map(Some),
    }
}

pub(super) fn parse_optional_u64_val(value: &Value) -> Result<Option<u64>, ScriptErrorInfo> {
    if value.is_null() {
        return Ok(None);
    }
    let Some(val) = value.as_i64() else {
        return Err(type_error("value must be an integer"));
    };
    if val < 0 {
        return Err(type_error("value must be non-negative"));
    }
    Ok(Some(val as u64))
}

pub(super) fn parse_optional_u64(
    map: Option<&Map<String, Value>>,
    key: &str,
) -> Result<Option<u64>, ScriptErrorInfo> {
    match map_value(map, key) {
        None => Ok(None),
        Some(value) => {
            let Some(value) = value.as_i64() else {
                return Err(type_error(format!("{key} must be an integer")));
            };
            if value < 0 {
                return Err(type_error(format!("{key} must be non-negative")));
            }
            Ok(Some(value as u64))
        }
    }
}

pub(super) fn parse_optional_u32(
    map: Option<&Map<String, Value>>,
    key: &str,
) -> Result<Option<u32>, ScriptErrorInfo> {
    parse_optional_u64(map, key)?
        .map(|value| u32::try_from(value).map_err(|_| type_error(format!("{key} is too large"))))
        .transpose()
}

pub(super) fn parse_optional_u8(
    map: Option<&Map<String, Value>>,
    key: &str,
) -> Result<Option<u8>, ScriptErrorInfo> {
    parse_optional_u64(map, key)?
        .map(|value| u8::try_from(value).map_err(|_| type_error(format!("{key} is too large"))))
        .transpose()
}

pub(super) fn parse_optional_f32(
    map: Option<&Map<String, Value>>,
    key: &str,
) -> Result<Option<f32>, ScriptErrorInfo> {
    match map_value(map, key) {
        None => Ok(None),
        Some(value) => parse_f32(value).map(Some),
    }
}

pub(super) fn parse_optional_vec2(
    map: Option<&Map<String, Value>>,
    key: &str,
) -> Result<Option<Vec2>, ScriptErrorInfo> {
    match map_value(map, key) {
        None => Ok(None),
        Some(value) => parse_vec2(value).map(Some),
    }
}

pub(super) fn parse_modifiers(
    map: Option<&Map<String, Value>>,
) -> Result<Modifiers, ScriptErrorInfo> {
    let Some(map) = map else {
        return Ok(Modifiers::default());
    };
    if let Some(value) = map.get("modifiers") {
        let modifiers = as_object(value, "modifiers must be a map")?;
        return parse_modifiers_map(Some(modifiers));
    }
    parse_modifiers_map(Some(map))
}

fn parse_modifiers_map(map: Option<&Map<String, Value>>) -> Result<Modifiers, ScriptErrorInfo> {
    let ctrl = parse_optional_bool(map, "ctrl")?.unwrap_or(false);
    let shift = parse_optional_bool(map, "shift")?.unwrap_or(false);
    let alt = parse_optional_bool(map, "alt")?.unwrap_or(false);
    let command = parse_optional_bool(map, "command")?.unwrap_or(false);
    Ok(Modifiers {
        ctrl,
        shift,
        alt,
        command,
    })
}

pub(super) fn parse_pointer_button(value: &Value) -> Result<PointerButtonName, ScriptErrorInfo> {
    let value = value
        .as_str()
        .ok_or_else(|| type_error("button must be a string"))?;
    match value {
        "primary" => Ok(PointerButtonName::Primary),
        "secondary" => Ok(PointerButtonName::Secondary),
        "middle" => Ok(PointerButtonName::Middle),
        _ => Err(type_error("button must be primary, secondary, or middle")),
    }
}

pub(super) fn parse_scroll_align(value: &Value) -> Result<ScrollAlign, ScriptErrorInfo> {
    let value = value
        .as_str()
        .ok_or_else(|| type_error("align must be a string"))?;
    match value {
        "top" => Ok(ScrollAlign::Top),
        "center" => Ok(ScrollAlign::Center),
        "bottom" => Ok(ScrollAlign::Bottom),
        _ => Err(type_error("align must be top, center, or bottom")),
    }
}

pub(super) fn parse_overlay_mode(value: &Value) -> Result<OverlayDebugModeName, ScriptErrorInfo> {
    let value = value
        .as_str()
        .ok_or_else(|| type_error("mode must be a string"))?;
    serde_json::from_value(Value::String(value.to_string())).map_err(|_| {
        type_error("mode must be bounds, margins, clipping, overlaps, focus, layers, or containers")
    })
}

pub(super) fn parse_widget_role(value: &Value) -> Result<WidgetRole, ScriptErrorInfo> {
    let value = value
        .as_str()
        .ok_or_else(|| type_error("role must be a string"))?;
    serde_json::from_value(Value::String(value.to_string()))
        .map_err(|_| type_error("role must be a widget role"))
}

pub(super) fn parse_widget_ref(value: &Value) -> Result<WidgetRef, ScriptErrorInfo> {
    if let Some(id) = value.as_str() {
        return Ok(WidgetRef {
            id: Some(id.to_string()),
            viewport_id: None,
        });
    }
    let map = as_object(value, "expected WidgetRef")?;
    let id = parse_optional_string(Some(map), "id")?;
    let viewport_id = parse_optional_string(Some(map), "viewport_id")?;
    if id.is_none() {
        return Err(type_error("WidgetRef requires id"));
    }
    Ok(WidgetRef { id, viewport_id })
}

pub(super) fn widget_value_from_dynamic(value: &Value) -> Result<WidgetValue, ScriptErrorInfo> {
    serde_json::from_value(value.clone())
        .map_err(|error| type_error(format!("invalid widget value: {error}")))
}

fn as_object(
    value: &Value,
    message: impl Into<String>,
) -> Result<&Map<String, Value>, ScriptErrorInfo> {
    value.as_object().ok_or_else(|| type_error(message))
}

fn type_error(message: impl Into<String>) -> ScriptErrorInfo {
    ScriptErrorInfo {
        error_type: "type_error".to_string(),
        message: message.into(),
        location: None,
        backtrace: None,
        code: None,
        details: None,
    }
}
