#![allow(clippy::needless_pass_by_value, clippy::result_large_err)]

use std::{
    error::Error as StdError,
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};

use mlua::{
    Error as LuaError, ExternalError, Function, Lua, LuaOptions, LuaSerdeExt, MultiValue, StdLib,
    Table as LuaTable, Value as LuaValue, VmState,
};
use serde_json::Value;
use tmcp::schema::ContentBlock;
use tokio::{runtime::Handle, task::JoinError};

use super::{
    runtime::ScriptRuntime,
    types::{
        ImageReferenceCollector, ScriptArgValue, ScriptArgs, ScriptErrorInfo, ScriptEvalOutcome,
        ScriptImageInfo, ScriptPosition, ScriptResult, ScriptTiming, ScriptValue,
    },
    value::collect_image_refs,
};
use crate::{registry::Inner, types::WidgetRef};

#[derive(Debug, Clone)]
struct LuauHostError {
    info: ScriptErrorInfo,
}

impl fmt::Display for LuauHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.info.message)
    }
}

impl StdError for LuauHostError {}

#[derive(Debug, Clone)]
struct LuauTimeoutError {
    timeout_ms: u64,
}

impl fmt::Display for LuauTimeoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Script timed out after {}ms", self.timeout_ms)
    }
}

impl StdError for LuauTimeoutError {}

/// Convert a spawned script task failure into the standard script-eval outcome shape.
pub fn script_eval_task_error(error: &JoinError) -> ScriptEvalOutcome {
    let message = if error.is_panic() {
        "Script evaluation panicked".to_string()
    } else {
        "Script evaluation task failed".to_string()
    };
    ScriptEvalOutcome::error_only(ScriptErrorInfo {
        error_type: "runtime".to_string(),
        message,
        location: None,
        backtrace: None,
        code: None,
        details: None,
    })
}

/// Evaluate a Luau script against the DevMCP runtime and return the structured outcome.
pub fn run_script_eval(
    inner: Arc<Inner>,
    handle: Handle,
    script: &str,
    timeout_ms: u64,
    source_name: String,
    args: ScriptArgs,
) -> ScriptEvalOutcome {
    let _guard = super::SCRIPT_EVAL_LOCK.blocking_lock();
    let runtime = Arc::new(ScriptRuntime::new(
        inner,
        handle,
        source_name.clone(),
        timeout_ms,
    ));

    let start = Instant::now();
    let compile_start = Instant::now();

    match execute_luau(&runtime, script, timeout_ms, &source_name, &args) {
        Ok((lua, values, compile_ms)) => {
            let exec_ms = start
                .elapsed()
                .as_millis()
                .saturating_sub(u128::from(compile_ms)) as u64;
            let timing = ScriptTiming {
                compile_ms,
                exec_ms,
                total_ms: start.elapsed().as_millis() as u64,
            };
            match values_to_script_value(&lua, runtime.as_ref(), values) {
                Ok(script_value) => build_success_outcome(runtime.as_ref(), script_value, timing),
                Err(error) => build_error_outcome(runtime.as_ref(), error, timing),
            }
        }
        Err(error) => {
            let compile_ms = compile_start.elapsed().as_millis() as u64;
            let timing = ScriptTiming {
                compile_ms,
                exec_ms: 0,
                total_ms: start.elapsed().as_millis() as u64,
            };
            build_error_outcome(
                runtime.as_ref(),
                lua_error_info(runtime.as_ref(), &error),
                timing,
            )
        }
    }
}

fn execute_luau(
    runtime: &Arc<ScriptRuntime>,
    script: &str,
    timeout_ms: u64,
    source_name: &str,
    args: &ScriptArgs,
) -> mlua::Result<(Lua, MultiValue, u64)> {
    let lua = Lua::new_with(StdLib::ALL_SAFE, LuaOptions::default())?;
    lua.sandbox(true)?;
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(timeout_ms))
        .unwrap_or_else(Instant::now);
    lua.set_interrupt(move |_| {
        if Instant::now() >= deadline {
            return Err(LuauTimeoutError { timeout_ms }.into_lua_err());
        }
        Ok(VmState::Continue)
    });

    register_globals(&lua, Arc::clone(runtime))?;
    register_script_args(&lua, args)?;

    let compile_start = Instant::now();
    let values = lua
        .load(script)
        .set_name(format!("@{source_name}"))
        .eval::<MultiValue>()?;
    let compile_ms = compile_start.elapsed().as_millis() as u64;
    Ok((lua, values, compile_ms))
}

fn register_script_args(lua: &Lua, args: &ScriptArgs) -> mlua::Result<()> {
    let arg_table = lua.create_table()?;
    for (key, value) in args {
        match value {
            ScriptArgValue::String(value) => arg_table.set(key.as_str(), value.as_str())?,
            ScriptArgValue::Int(value) => arg_table.set(key.as_str(), *value)?,
            ScriptArgValue::Float(value) => arg_table.set(key.as_str(), *value)?,
            ScriptArgValue::Bool(value) => arg_table.set(key.as_str(), *value)?,
        }
    }
    let table_lib: LuaTable = lua.globals().get("table")?;
    let freeze: Function = table_lib.get("freeze")?;
    let frozen: LuaTable = freeze.call(arg_table)?;
    lua.globals().set("args", frozen)?;
    Ok(())
}

/// Extract the widget id as a JSON string from a Widget Lua table.
///
/// Returns just the id string (not a map) so that runtime methods take the simple
/// string path in `parse_widget_ref` and skip disambiguation logic.
fn widget_id(table: &LuaTable) -> mlua::Result<serde_json::Value> {
    let id: String = table.get("id")?;
    Ok(serde_json::Value::String(id))
}

fn viewport_id(table: &LuaTable) -> mlua::Result<String> {
    table.get("id")
}

/// Inject the widget's viewport_id into a JSON options map (or create one if absent).
fn inject_viewport(table: &LuaTable, options: &mut Option<serde_json::Value>) -> mlua::Result<()> {
    let viewport_id: Option<String> =
        table
            .get::<Option<String>>("__viewport_id")?
            .or(table.get::<Option<String>>("viewport_id")?);
    if let Some(vp) = viewport_id {
        let map = options
            .get_or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
            .as_object_mut();
        if let Some(map) = map {
            map.entry("viewport_id")
                .or_insert(serde_json::Value::String(vp));
        }
    }
    Ok(())
}

fn inject_viewport_id(viewport_id: String, options: &mut Option<serde_json::Value>) {
    let map = options
        .get_or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
        .as_object_mut();
    if let Some(map) = map {
        map.entry("viewport_id")
            .or_insert(serde_json::Value::String(viewport_id));
    }
}

fn insert_option(
    options: &mut Option<serde_json::Value>,
    key: &str,
    value: serde_json::Value,
) -> mlua::Result<()> {
    let Some(map) = options
        .get_or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
        .as_object_mut()
    else {
        return Err(LuaError::runtime(
            "options must be a table when adding script binding options",
        ));
    };
    map.insert(key.to_string(), value);
    Ok(())
}

fn is_vec2_value(value: &serde_json::Value) -> bool {
    let Some(map) = value.as_object() else {
        return false;
    };
    map.len() == 2
        && map.get("x").and_then(serde_json::Value::as_f64).is_some()
        && map.get("y").and_then(serde_json::Value::as_f64).is_some()
}

fn widget_at_point_options_from_lua(
    lua: &Lua,
    arg: Option<LuaValue>,
) -> mlua::Result<Option<serde_json::Value>> {
    match arg {
        None => Ok(None),
        Some(LuaValue::Boolean(all_layers)) => {
            let mut options = None;
            insert_option(
                &mut options,
                "all_layers",
                serde_json::Value::Bool(all_layers),
            )?;
            Ok(options)
        }
        Some(value) => optional_json_from_lua(lua, Some(value)),
    }
}

fn drag_relative_options_from_lua(
    lua: &Lua,
    third: Option<LuaValue>,
    fourth: Option<LuaValue>,
) -> mlua::Result<Option<serde_json::Value>> {
    let has_fourth = fourth.is_some();
    let mut options = match fourth {
        Some(value) => optional_json_from_lua(lua, Some(value))?,
        None => None,
    };

    match third {
        None => Ok(options),
        Some(value) => {
            let json = json_from_lua(lua, value)?;
            if has_fourth || is_vec2_value(&json) {
                insert_option(&mut options, "from", json)?;
                Ok(options)
            } else {
                Ok(Some(json))
            }
        }
    }
}

/// Attach the widget metatable to a single Lua table representing a Widget.
fn attach_widget_mt(lua: &Lua, value: &LuaValue) -> mlua::Result<()> {
    if let LuaValue::Table(table) = value {
        let mt: LuaTable = lua.named_registry_value("widget_mt")?;
        table.set_metatable(Some(mt))?;
    }
    Ok(())
}

/// Attach the widget metatable to each element in a Lua array of Widgets.
fn attach_widget_mt_array(lua: &Lua, value: &LuaValue) -> mlua::Result<()> {
    if let LuaValue::Table(table) = value {
        for pair in table.clone().sequence_values::<LuaValue>() {
            attach_widget_mt(lua, &pair?)?;
        }
    }
    Ok(())
}

fn attach_viewport_mt(lua: &Lua, value: &LuaValue) -> mlua::Result<()> {
    if let LuaValue::Table(table) = value {
        let mt: LuaTable = lua.named_registry_value("viewport_mt")?;
        table.set_metatable(Some(mt))?;
    }
    Ok(())
}

fn attach_viewport_mt_array(lua: &Lua, value: &LuaValue) -> mlua::Result<()> {
    if let LuaValue::Table(table) = value {
        for pair in table.clone().sequence_values::<LuaValue>() {
            attach_viewport_mt(lua, &pair?)?;
        }
    }
    Ok(())
}

/// Create the Widget metatable with all instance methods and store it in the Lua registry.
fn create_widget_metatable(lua: &Lua, runtime: Arc<ScriptRuntime>) -> mlua::Result<()> {
    let methods = lua.create_table()?;

    // viewport_id(self) - exposed as a readable field via __index
    // The __viewport_id internal field is already set; we expose it as viewport_id.

    // viewport(self) -> Viewport
    let rt = Arc::clone(&runtime);
    methods.set(
        "viewport",
        lua.create_function(move |lua, self_t: LuaTable| {
            let pos = current_position(lua);
            let vp_id: String = self_t
                .get("__viewport_id")
                .unwrap_or_else(|_| "root".to_string());
            let result = rt.viewport_handle(pos, &vp_id).map_err(host_error)?;
            let lua_val = lua.to_value(&result)?;
            attach_viewport_mt(lua, &lua_val)?;
            Ok(lua_val)
        })?,
    )?;

    // click(self, options?)
    let rt = Arc::clone(&runtime);
    methods.set(
        "click",
        lua.create_function(
            move |lua, (self_t, options): (LuaTable, Option<LuaValue>)| {
                let pos = current_position(lua);
                let target = widget_id(&self_t)?;
                let mut options = optional_json_from_lua(lua, options)?;
                inject_viewport(&self_t, &mut options)?;
                let result = rt
                    .action_click(pos, &target, options.as_ref().and_then(Value::as_object))
                    .map_err(host_error)?;
                lua.to_value(&result)
            },
        )?,
    )?;

    // hover(self, options?)
    let rt = Arc::clone(&runtime);
    methods.set(
        "hover",
        lua.create_function(
            move |lua, (self_t, options): (LuaTable, Option<LuaValue>)| {
                let pos = current_position(lua);
                let target = widget_id(&self_t)?;
                let mut options = optional_json_from_lua(lua, options)?;
                inject_viewport(&self_t, &mut options)?;
                let result = rt
                    .action_hover(pos, &target, options.as_ref().and_then(Value::as_object))
                    .map_err(host_error)?;
                lua.to_value(&result)
            },
        )?,
    )?;

    // type_text(self, text, options?)
    let rt = Arc::clone(&runtime);
    methods.set(
        "type_text",
        lua.create_function(
            move |lua, (self_t, text, options): (LuaTable, String, Option<LuaValue>)| {
                let pos = current_position(lua);
                let target = widget_id(&self_t)?;
                let mut options = optional_json_from_lua(lua, options)?;
                inject_viewport(&self_t, &mut options)?;
                let result = rt
                    .action_type(
                        pos,
                        &target,
                        text,
                        options.as_ref().and_then(Value::as_object),
                    )
                    .map_err(host_error)?;
                lua.to_value(&result)
            },
        )?,
    )?;

    // focus(self)
    let rt = Arc::clone(&runtime);
    methods.set(
        "focus",
        lua.create_function(move |lua, self_t: LuaTable| {
            let pos = current_position(lua);
            let target = widget_id(&self_t)?;
            let result = rt.action_focus(pos, &target).map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    // set_value(self, value, options?)
    let rt = Arc::clone(&runtime);
    methods.set(
        "set_value",
        lua.create_function(
            move |lua, (self_t, value, options): (LuaTable, LuaValue, Option<LuaValue>)| {
                let pos = current_position(lua);
                let target = widget_id(&self_t)?;
                let value = json_from_lua(lua, value)?;
                let mut options = optional_json_from_lua(lua, options)?;
                inject_viewport(&self_t, &mut options)?;
                let result = rt
                    .widget_set_value(
                        pos,
                        &target,
                        &value,
                        options.as_ref().and_then(Value::as_object),
                    )
                    .map_err(host_error)?;
                lua.to_value(&result)
            },
        )?,
    )?;

    // drag(self, to_pos, options?)
    let rt = Arc::clone(&runtime);
    methods.set(
        "drag",
        lua.create_function(
            move |lua, (self_t, to, options): (LuaTable, LuaValue, Option<LuaValue>)| {
                let pos = current_position(lua);
                let target = widget_id(&self_t)?;
                let to = json_from_lua(lua, to)?;
                let mut options = optional_json_from_lua(lua, options)?;
                inject_viewport(&self_t, &mut options)?;
                let result = rt
                    .action_drag(
                        pos,
                        &target,
                        &to,
                        options.as_ref().and_then(Value::as_object),
                    )
                    .map_err(host_error)?;
                lua.to_value(&result)
            },
        )?,
    )?;

    // drag_relative(self, relative, from? [, options?])
    let rt = Arc::clone(&runtime);
    methods.set(
        "drag_relative",
        lua.create_function(
            move |lua,
                  (self_t, relative, third, fourth): (
                LuaTable,
                LuaValue,
                Option<LuaValue>,
                Option<LuaValue>,
            )| {
                let pos = current_position(lua);
                let target = widget_id(&self_t)?;
                let relative = json_from_lua(lua, relative)?;
                let mut options = drag_relative_options_from_lua(lua, third, fourth)?;
                inject_viewport(&self_t, &mut options)?;
                let result = rt
                    .action_drag_relative(
                        pos,
                        &target,
                        &relative,
                        options.as_ref().and_then(Value::as_object),
                    )
                    .map_err(host_error)?;
                lua.to_value(&result)
            },
        )?,
    )?;

    // drag_to(self, to_widget, options?)
    let rt = Arc::clone(&runtime);
    methods.set(
        "drag_to",
        lua.create_function(
            move |lua, (self_t, to, options): (LuaTable, LuaTable, Option<LuaValue>)| {
                let pos = current_position(lua);
                let from = widget_id(&self_t)?;
                let to = widget_id(&to)?;
                let mut options = optional_json_from_lua(lua, options)?;
                inject_viewport(&self_t, &mut options)?;
                let result = rt
                    .action_drag_to_widget(
                        pos,
                        &from,
                        &to,
                        options.as_ref().and_then(Value::as_object),
                    )
                    .map_err(host_error)?;
                lua.to_value(&result)
            },
        )?,
    )?;

    // scroll(self, delta, options?)
    let rt = Arc::clone(&runtime);
    methods.set(
        "scroll",
        lua.create_function(
            move |lua, (self_t, delta, options): (LuaTable, LuaValue, Option<LuaValue>)| {
                let pos = current_position(lua);
                let target = widget_id(&self_t)?;
                let delta = json_from_lua(lua, delta)?;
                let mut options = optional_json_from_lua(lua, options)?;
                inject_viewport(&self_t, &mut options)?;
                let result = rt
                    .action_scroll(
                        pos,
                        &target,
                        &delta,
                        options.as_ref().and_then(Value::as_object),
                    )
                    .map_err(host_error)?;
                lua.to_value(&result)
            },
        )?,
    )?;

    // scroll_to(self, options?)
    let rt = Arc::clone(&runtime);
    methods.set(
        "scroll_to",
        lua.create_function(
            move |lua, (self_t, options): (LuaTable, Option<LuaValue>)| {
                let pos = current_position(lua);
                let target = widget_id(&self_t)?;
                let mut options = optional_json_from_lua(lua, options)?;
                inject_viewport(&self_t, &mut options)?;
                let result = rt
                    .action_scroll_to(pos, &target, options.as_ref().and_then(Value::as_object))
                    .map_err(host_error)?;
                lua.to_value(&result)
            },
        )?,
    )?;

    // scroll_into_view(self, options?)
    let rt = Arc::clone(&runtime);
    methods.set(
        "scroll_into_view",
        lua.create_function(
            move |lua, (self_t, options): (LuaTable, Option<LuaValue>)| {
                let pos = current_position(lua);
                let target = widget_id(&self_t)?;
                let mut options = optional_json_from_lua(lua, options)?;
                inject_viewport(&self_t, &mut options)?;
                let result = rt
                    .action_scroll_into_view(
                        pos,
                        &target,
                        options.as_ref().and_then(Value::as_object),
                    )
                    .map_err(host_error)?;
                lua.to_value(&result)
            },
        )?,
    )?;

    // state(self)
    let rt = Arc::clone(&runtime);
    methods.set(
        "state",
        lua.create_function(move |lua, self_t: LuaTable| {
            let pos = current_position(lua);
            let target = widget_id(&self_t)?;
            let result = rt.widget_state(pos, &target).map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    // parent(self)
    let rt = Arc::clone(&runtime);
    methods.set(
        "parent",
        lua.create_function(move |lua, self_t: LuaTable| {
            let pos = current_position(lua);
            let target = widget_id(&self_t)?;
            let result = rt.widget_parent(pos, &target).map_err(host_error)?;
            if result.is_null() {
                return Ok(LuaValue::Nil);
            }
            let lua_val = lua.to_value(&result)?;
            attach_widget_mt(lua, &lua_val)?;
            Ok(lua_val)
        })?,
    )?;

    // children(self)
    let rt = Arc::clone(&runtime);
    methods.set(
        "children",
        lua.create_function(move |lua, self_t: LuaTable| {
            let pos = current_position(lua);
            let target = widget_id(&self_t)?;
            let result = rt.widget_children(pos, &target).map_err(host_error)?;
            let lua_val = lua.to_value(&result)?;
            attach_widget_mt_array(lua, &lua_val)?;
            Ok(lua_val)
        })?,
    )?;

    // text_measure(self)
    let rt = Arc::clone(&runtime);
    methods.set(
        "text_measure",
        lua.create_function(move |lua, self_t: LuaTable| {
            let pos = current_position(lua);
            let target = widget_id(&self_t)?;
            let result = rt.text_measure(pos, &target).map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    // check_layout(self)
    let rt = Arc::clone(&runtime);
    methods.set(
        "check_layout",
        lua.create_function(move |lua, self_t: LuaTable| {
            let pos = current_position(lua);
            let target = widget_id(&self_t)?;
            let viewport_id: Option<String> = self_t.get("__viewport_id")?;
            let result = rt
                .check_layout_widget(pos, &target, viewport_id)
                .map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    // screenshot(self)
    let rt = Arc::clone(&runtime);
    methods.set(
        "screenshot",
        lua.create_function(move |lua, self_t: LuaTable| {
            let pos = current_position(lua);
            let target = widget_id(&self_t)?;
            let result = rt.screenshot(pos, Some(&target)).map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    // show_highlight(self, color)
    let rt = Arc::clone(&runtime);
    methods.set(
        "show_highlight",
        lua.create_function(move |lua, (self_t, color): (LuaTable, String)| {
            let pos = current_position(lua);
            let target = widget_id(&self_t)?;
            let viewport_id: Option<String> = self_t.get("__viewport_id")?;
            let result = rt
                .show_highlight_widget(pos, &target, viewport_id, color)
                .map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    // hide_highlight(self)
    let rt = Arc::clone(&runtime);
    methods.set(
        "hide_highlight",
        lua.create_function(move |lua, self_t: LuaTable| {
            let pos = current_position(lua);
            let target = widget_id(&self_t)?;
            let viewport_id: Option<String> = self_t.get("__viewport_id")?;
            rt.hide_highlight_widget(pos, &target, viewport_id)
                .map_err(host_error)?;
            Ok(LuaValue::Nil)
        })?,
    )?;

    // show_debug_overlay(self, mode?, options?)
    let rt = Arc::clone(&runtime);
    methods.set(
        "show_debug_overlay",
        lua.create_function(
            move |lua, (self_t, mode, options): (LuaTable, Option<LuaValue>, Option<LuaValue>)| {
                let pos = current_position(lua);
                let mode = mode.map(|value| json_from_lua(lua, value)).transpose()?;
                let mut options = optional_json_from_lua(lua, options)?;
                inject_viewport(&self_t, &mut options)?;
                let target = widget_id(&self_t)?;
                let viewport_id: Option<String> = self_t.get::<Option<String>>("__viewport_id")?;
                let id = target.as_str().unwrap_or("").to_string();
                let scope = Some(WidgetRef {
                    id: Some(id),
                    viewport_id,
                });
                rt.show_debug_overlay(
                    pos,
                    None,
                    mode.as_ref(),
                    options.as_ref().and_then(Value::as_object),
                    scope,
                )
                .map_err(host_error)?;
                Ok(LuaValue::Nil)
            },
        )?,
    )?;

    // hide_debug_overlay(self)
    let rt = Arc::clone(&runtime);
    methods.set(
        "hide_debug_overlay",
        lua.create_function(move |lua, _self_t: LuaTable| {
            let pos = current_position(lua);
            rt.hide_debug_overlay(pos).map_err(host_error)?;
            Ok(LuaValue::Nil)
        })?,
    )?;

    // wait_for(self, predicate, options?)
    let rt = Arc::clone(&runtime);
    methods.set(
        "wait_for",
        lua.create_function(move |lua, (self_t, predicate): (LuaTable, Function)| {
            let pos = current_position(lua);
            let target = widget_id(&self_t)?;
            let mut options = Some(Value::Object(serde_json::Map::new()));
            inject_viewport(&self_t, &mut options)?;
            let options_map = options.as_ref().and_then(Value::as_object);
            let result = rt
                .wait_for_widget_predicate(pos, &target, options_map, |widget| {
                    predicate_matches(lua, rt.as_ref(), predicate.clone(), widget)
                })
                .map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    // wait_for_visible(self)
    let rt = Arc::clone(&runtime);
    methods.set(
        "wait_for_visible",
        lua.create_function(move |lua, self_t: LuaTable| {
            let pos = current_position(lua);
            let target = widget_id(&self_t)?;
            let mut options = Some(Value::Object(serde_json::Map::new()));
            inject_viewport(&self_t, &mut options)?;
            let options_map = options.as_ref().and_then(Value::as_object);
            let result = rt
                .wait_for_widget_visible(pos, &target, options_map)
                .map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    // wait_for_absent(self)
    let rt = Arc::clone(&runtime);
    methods.set(
        "wait_for_absent",
        lua.create_function(move |lua, self_t: LuaTable| {
            let pos = current_position(lua);
            let target = widget_id(&self_t)?;
            let mut options = Some(Value::Object(serde_json::Map::new()));
            inject_viewport(&self_t, &mut options)?;
            let options_map = options.as_ref().and_then(Value::as_object);
            rt.wait_for_widget_absent(pos, &target, options_map)
                .map_err(host_error)?;
            Ok(LuaValue::Nil)
        })?,
    )?;

    let metatable = lua.create_table()?;
    metatable.set("__index", methods)?;
    lua.set_named_registry_value("widget_mt", metatable)?;
    Ok(())
}

fn create_viewport_metatable(lua: &Lua, runtime: Arc<ScriptRuntime>) -> mlua::Result<()> {
    let methods = lua.create_table()?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "state",
        lua.create_function(move |lua, self_t: LuaTable| {
            let pos = current_position(lua);
            let result = rt
                .viewport_state(pos, viewport_id(&self_t)?)
                .map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "widget_list",
        lua.create_function(
            move |lua, (self_t, options): (LuaTable, Option<LuaValue>)| {
                let pos = current_position(lua);
                let mut options = optional_json_from_lua(lua, options)?;
                inject_viewport_id(viewport_id(&self_t)?, &mut options);
                let result = rt
                    .widget_list(pos, options.as_ref().and_then(Value::as_object))
                    .map_err(host_error)?;
                let lua_val = lua.to_value(&result)?;
                attach_widget_mt_array(lua, &lua_val)?;
                Ok(lua_val)
            },
        )?,
    )?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "widget_get",
        lua.create_function(move |lua, (self_t, id): (LuaTable, String)| {
            let pos = current_position(lua);
            let target = Value::String(id);
            let mut options = Some(Value::Object(serde_json::Map::new()));
            inject_viewport_id(viewport_id(&self_t)?, &mut options);
            let result = rt
                .widget_get(pos, &target, options.as_ref().and_then(Value::as_object))
                .map_err(host_error)?;
            let lua_val = lua.to_value(&result)?;
            attach_widget_mt(lua, &lua_val)?;
            Ok(lua_val)
        })?,
    )?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "widget_at_point",
        lua.create_function(
            move |lua, (self_t, point, arg): (LuaTable, LuaValue, Option<LuaValue>)| {
                let pos = current_position(lua);
                let point = json_from_lua(lua, point)?;
                let mut options = widget_at_point_options_from_lua(lua, arg)?;
                inject_viewport_id(viewport_id(&self_t)?, &mut options);
                let result = rt
                    .widget_at_point(pos, &point, options.as_ref().and_then(Value::as_object))
                    .map_err(host_error)?;
                let lua_val = lua.to_value(&result)?;
                attach_widget_mt_array(lua, &lua_val)?;
                Ok(lua_val)
            },
        )?,
    )?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "wait_for_widget",
        lua.create_function(
            move |lua,
                  (self_t, id, predicate, options): (
                LuaTable,
                String,
                Function,
                Option<LuaValue>,
            )| {
                let pos = current_position(lua);
                let target = Value::String(id);
                let mut options = optional_json_from_lua(lua, options)?;
                inject_viewport_id(viewport_id(&self_t)?, &mut options);
                let options_map = options.as_ref().and_then(Value::as_object);
                let result = rt
                    .wait_for_widget_predicate(pos, &target, options_map, |widget| {
                        predicate_matches(lua, rt.as_ref(), predicate.clone(), widget)
                    })
                    .map_err(host_error)?;
                lua.to_value(&result)
            },
        )?,
    )?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "wait_for_widget_visible",
        lua.create_function(move |lua, (self_t, id): (LuaTable, String)| {
            let pos = current_position(lua);
            let target = Value::String(id);
            let mut options = Some(Value::Object(serde_json::Map::new()));
            inject_viewport_id(viewport_id(&self_t)?, &mut options);
            let options_map = options.as_ref().and_then(Value::as_object);
            let result = rt
                .wait_for_widget_visible(pos, &target, options_map)
                .map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "wait_for_widget_absent",
        lua.create_function(
            move |lua, (self_t, id, options): (LuaTable, String, Option<LuaValue>)| {
                let pos = current_position(lua);
                let target = Value::String(id);
                let mut options = optional_json_from_lua(lua, options)?;
                inject_viewport_id(viewport_id(&self_t)?, &mut options);
                let options_map = options.as_ref().and_then(Value::as_object);
                rt.wait_for_widget_absent(pos, &target, options_map)
                    .map_err(host_error)?;
                Ok(LuaValue::Nil)
            },
        )?,
    )?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "wait_for_settle",
        lua.create_function(move |lua, self_t: LuaTable| {
            let pos = current_position(lua);
            let mut options = Some(Value::Object(serde_json::Map::new()));
            inject_viewport_id(viewport_id(&self_t)?, &mut options);
            let result = rt
                .wait_for_settle(pos, options.as_ref().and_then(Value::as_object))
                .map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "wait_for",
        lua.create_function(
            move |lua, (self_t, predicate, options): (LuaTable, Function, Option<LuaValue>)| {
                let pos = current_position(lua);
                let mut options = optional_json_from_lua(lua, options)?;
                inject_viewport_id(viewport_id(&self_t)?, &mut options);
                let result = rt
                    .wait_for_viewport_predicate(
                        pos,
                        options.as_ref().and_then(Value::as_object),
                        |viewport| predicate_matches(lua, rt.as_ref(), predicate.clone(), viewport),
                    )
                    .map_err(host_error)?;
                lua.to_value(&result)
            },
        )?,
    )?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "key",
        lua.create_function(
            move |lua, (self_t, key, options): (LuaTable, String, Option<LuaValue>)| {
                let pos = current_position(lua);
                let mut options = optional_json_from_lua(lua, options)?;
                inject_viewport_id(viewport_id(&self_t)?, &mut options);
                let result = rt
                    .action_key(pos, key, options.as_ref().and_then(Value::as_object))
                    .map_err(host_error)?;
                lua.to_value(&result)
            },
        )?,
    )?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "paste",
        lua.create_function(
            move |lua, (self_t, text, options): (LuaTable, String, Option<LuaValue>)| {
                let pos = current_position(lua);
                let mut options = optional_json_from_lua(lua, options)?;
                inject_viewport_id(viewport_id(&self_t)?, &mut options);
                let result = rt
                    .action_paste(pos, text, options.as_ref().and_then(Value::as_object))
                    .map_err(host_error)?;
                lua.to_value(&result)
            },
        )?,
    )?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "raw_pointer_move",
        lua.create_function(move |lua, (self_t, point): (LuaTable, LuaValue)| {
            let pos = current_position(lua);
            let point = json_from_lua(lua, point)?;
            let mut options = Some(Value::Object(serde_json::Map::new()));
            inject_viewport_id(viewport_id(&self_t)?, &mut options);
            let result = rt
                .raw_pointer_move(pos, &point, options.as_ref().and_then(Value::as_object))
                .map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "raw_pointer_button",
        lua.create_function(
            move |lua,
                  (self_t, point, button, action, options): (
                LuaTable,
                LuaValue,
                LuaValue,
                String,
                Option<LuaValue>,
            )| {
                let pos = current_position(lua);
                let point = json_from_lua(lua, point)?;
                let button = json_from_lua(lua, button)?;
                let pressed = parse_raw_action(&action).map_err(host_error)?;
                let mut options = optional_json_from_lua(lua, options)?;
                inject_viewport_id(viewport_id(&self_t)?, &mut options);
                let result = rt
                    .raw_pointer_button(
                        pos,
                        &point,
                        &button,
                        pressed,
                        options.as_ref().and_then(Value::as_object),
                    )
                    .map_err(host_error)?;
                lua.to_value(&result)
            },
        )?,
    )?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "raw_key",
        lua.create_function(
            move |lua, (self_t, key, action, options): (LuaTable, String, String, Option<LuaValue>)| {
                let pos = current_position(lua);
                let pressed = parse_raw_action(&action).map_err(host_error)?;
                let mut options = optional_json_from_lua(lua, options)?;
                inject_viewport_id(viewport_id(&self_t)?, &mut options);
                let result = rt
                    .raw_key(pos, key, pressed, options.as_ref().and_then(Value::as_object))
                    .map_err(host_error)?;
                lua.to_value(&result)
            },
        )?,
    )?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "raw_text",
        lua.create_function(move |lua, (self_t, text): (LuaTable, String)| {
            let pos = current_position(lua);
            let mut options = Some(Value::Object(serde_json::Map::new()));
            inject_viewport_id(viewport_id(&self_t)?, &mut options);
            let result = rt
                .raw_text(pos, text, options.as_ref().and_then(Value::as_object))
                .map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "raw_scroll",
        lua.create_function(
            move |lua, (self_t, delta, options): (LuaTable, LuaValue, Option<LuaValue>)| {
                let pos = current_position(lua);
                let delta = json_from_lua(lua, delta)?;
                let mut options = optional_json_from_lua(lua, options)?;
                inject_viewport_id(viewport_id(&self_t)?, &mut options);
                let result = rt
                    .raw_scroll(pos, &delta, options.as_ref().and_then(Value::as_object))
                    .map_err(host_error)?;
                lua.to_value(&result)
            },
        )?,
    )?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "focus",
        lua.create_function(move |lua, self_t: LuaTable| {
            let pos = current_position(lua);
            let result = rt
                .focus_window(pos, viewport_id(&self_t)?)
                .map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "set_inner_size",
        lua.create_function(move |lua, (self_t, size): (LuaTable, LuaValue)| {
            let pos = current_position(lua);
            let size = json_from_lua(lua, size)?;
            let vp_id = viewport_id(&self_t)?;
            let result = rt
                .viewport_set_inner_size(pos, &size, Some(vp_id))
                .map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "set_resize_options",
        lua.create_function(move |lua, (self_t, options): (LuaTable, LuaValue)| {
            let pos = current_position(lua);
            let options = json_from_lua(lua, options)?;
            let vp_id = viewport_id(&self_t)?;
            let result = rt
                .viewport_set_resize_options(pos, options.as_object(), Some(vp_id))
                .map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "screenshot",
        lua.create_function(move |lua, self_t: LuaTable| {
            let pos = current_position(lua);
            let mut arg = Some(Value::Object(serde_json::Map::new()));
            inject_viewport_id(viewport_id(&self_t)?, &mut arg);
            let result = rt.screenshot(pos, arg.as_ref()).map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "check_layout",
        lua.create_function(move |lua, self_t: LuaTable| {
            let pos = current_position(lua);
            let result = rt
                .check_layout(pos, Some(viewport_id(&self_t)?))
                .map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    // show_highlight(self, rect, color)
    let rt = Arc::clone(&runtime);
    methods.set(
        "show_highlight",
        lua.create_function(
            move |lua, (self_t, rect, color): (LuaTable, LuaValue, String)| {
                let pos = current_position(lua);
                let vp_id = viewport_id(&self_t)?;
                let rect_json = json_from_lua(lua, rect)?;
                let rect = super::parse::parse_rect(&rect_json).map_err(host_error)?;
                let result = rt
                    .show_highlight_rect(pos, Some(vp_id), rect, color)
                    .map_err(host_error)?;
                lua.to_value(&result)
            },
        )?,
    )?;

    // hide_highlight(self)
    let rt = Arc::clone(&runtime);
    methods.set(
        "hide_highlight",
        lua.create_function(move |lua, self_t: LuaTable| {
            let pos = current_position(lua);
            let _ = viewport_id(&self_t)?;
            rt.hide_highlight_all(pos).map_err(host_error)?;
            Ok(LuaValue::Nil)
        })?,
    )?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "show_debug_overlay",
        lua.create_function(
            move |lua, (self_t, mode, options): (LuaTable, Option<LuaValue>, Option<LuaValue>)| {
                let pos = current_position(lua);
                let mode = mode.map(|value| json_from_lua(lua, value)).transpose()?;
                let mut options = optional_json_from_lua(lua, options)?;
                inject_viewport_id(viewport_id(&self_t)?, &mut options);
                let result = rt
                    .show_debug_overlay(
                        pos,
                        Some(viewport_id(&self_t)?),
                        mode.as_ref(),
                        options.as_ref().and_then(Value::as_object),
                        None,
                    )
                    .map_err(host_error)?;
                lua.to_value(&result)
            },
        )?,
    )?;

    let rt = Arc::clone(&runtime);
    methods.set(
        "hide_debug_overlay",
        lua.create_function(move |lua, self_t: LuaTable| {
            let pos = current_position(lua);
            let _ = viewport_id(&self_t)?;
            let result = rt.hide_debug_overlay(pos).map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    let metatable = lua.create_table()?;
    metatable.set("__index", methods)?;
    lua.set_named_registry_value("viewport_mt", metatable)?;
    Ok(())
}

fn register_globals(lua: &Lua, runtime: Arc<ScriptRuntime>) -> mlua::Result<()> {
    let globals = lua.globals();

    let runtime_c = Arc::clone(&runtime);
    globals.set(
        "log",
        lua.create_function(move |lua, value: LuaValue| {
            let rendered = match json_from_lua(lua, value.clone()) {
                Ok(Value::String(value)) => value,
                Ok(value) if !value.is_null() => value.to_string(),
                Ok(_) => "null".to_string(),
                Err(_) => format!("{value:?}"),
            };
            runtime_c.log(rendered);
            Ok(())
        })?,
    )?;

    let runtime_c = Arc::clone(&runtime);
    globals.set(
        "configure",
        lua.create_function(move |lua, options: LuaValue| {
            let pos = current_position(lua);
            let options = json_from_lua(lua, options)?;
            runtime_c
                .configure(pos, options.as_object())
                .map_err(host_error)?;
            Ok(())
        })?,
    )?;

    create_widget_metatable(lua, Arc::clone(&runtime))?;
    create_viewport_metatable(lua, Arc::clone(&runtime))?;
    register_widget_functions(lua, Arc::clone(&runtime))?;
    register_wait_functions(lua, Arc::clone(&runtime))?;
    register_screenshot_functions(lua, Arc::clone(&runtime))?;
    register_layout_functions(lua, Arc::clone(&runtime))?;
    register_viewport_functions(lua, Arc::clone(&runtime))?;
    register_assertion_functions(lua, Arc::clone(&runtime))?;
    register_fixture_functions(lua, Arc::clone(&runtime))?;

    for name in [
        "widget_list",
        "widget_get",
        "widget_at_point",
        "screenshot",
        "check_layout",
        "show_debug_overlay",
        "hide_debug_overlay",
        "viewports_list",
        "viewport_set_inner_size",
        "focus_window",
    ] {
        globals.set(name, LuaValue::Nil)?;
    }

    let runtime_c = Arc::clone(&runtime);
    globals.set(
        "root",
        lua.create_function(move |lua, _: ()| {
            let pos = current_position(lua);
            let result = runtime_c.root_viewport(pos).map_err(host_error)?;
            let lua_val = lua.to_value(&result)?;
            attach_viewport_mt(lua, &lua_val)?;
            Ok(lua_val)
        })?,
    )?;

    let runtime_c = Arc::clone(&runtime);
    globals.set(
        "viewports",
        lua.create_function(move |lua, _: ()| {
            let pos = current_position(lua);
            let result = runtime_c.viewports_list(pos, None).map_err(host_error)?;
            let lua_val = lua.to_value(&result)?;
            attach_viewport_mt_array(lua, &lua_val)?;
            Ok(lua_val)
        })?,
    )?;

    Ok(())
}

fn register_widget_functions(lua: &Lua, runtime: Arc<ScriptRuntime>) -> mlua::Result<()> {
    let globals = lua.globals();

    let runtime_c = Arc::clone(&runtime);
    globals.set(
        "widget_list",
        lua.create_function(move |lua, options: Option<LuaValue>| {
            let pos = current_position(lua);
            let options = optional_json_from_lua(lua, options)?;
            let result = runtime_c
                .widget_list(pos, options.as_ref().and_then(Value::as_object))
                .map_err(host_error)?;
            let lua_val = lua.to_value(&result)?;
            attach_widget_mt_array(lua, &lua_val)?;
            Ok(lua_val)
        })?,
    )?;

    let runtime_c = Arc::clone(&runtime);
    globals.set(
        "widget_get",
        lua.create_function(move |lua, (target, options): (String, Option<LuaValue>)| {
            let pos = current_position(lua);
            let target = Value::String(target);
            let options = optional_json_from_lua(lua, options)?;
            let result = runtime_c
                .widget_get(pos, &target, options.as_ref().and_then(Value::as_object))
                .map_err(host_error)?;
            let lua_val = lua.to_value(&result)?;
            attach_widget_mt(lua, &lua_val)?;
            Ok(lua_val)
        })?,
    )?;

    let runtime_c = Arc::clone(&runtime);
    globals.set(
        "widget_at_point",
        lua.create_function(move |lua, (point, arg): (LuaValue, Option<LuaValue>)| {
            let pos = current_position(lua);
            let point = json_from_lua(lua, point)?;
            let options = widget_at_point_options_from_lua(lua, arg)?;
            let result = runtime_c
                .widget_at_point(pos, &point, options.as_ref().and_then(Value::as_object))
                .map_err(host_error)?;
            let lua_val = lua.to_value(&result)?;
            attach_widget_mt_array(lua, &lua_val)?;
            Ok(lua_val)
        })?,
    )?;

    Ok(())
}

fn register_wait_functions(lua: &Lua, runtime: Arc<ScriptRuntime>) -> mlua::Result<()> {
    let globals = lua.globals();

    let runtime_c = Arc::clone(&runtime);
    globals.set(
        "wait_for_frames",
        lua.create_function(move |lua, count: LuaValue| {
            let pos = current_position(lua);
            let count = json_from_lua(lua, count)?;
            let result = runtime_c.wait_for_frames(pos, &count).map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    Ok(())
}

fn register_screenshot_functions(lua: &Lua, runtime: Arc<ScriptRuntime>) -> mlua::Result<()> {
    let globals = lua.globals();

    let runtime_c = Arc::clone(&runtime);
    globals.set(
        "screenshot",
        lua.create_function(move |lua, target: Option<LuaValue>| {
            let pos = current_position(lua);
            let target = optional_json_from_lua(lua, target)?;
            let result = runtime_c
                .screenshot(pos, target.as_ref())
                .map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    Ok(())
}

fn register_layout_functions(lua: &Lua, runtime: Arc<ScriptRuntime>) -> mlua::Result<()> {
    let globals = lua.globals();

    let runtime_c = Arc::clone(&runtime);
    globals.set(
        "show_debug_overlay",
        lua.create_function(
            move |lua, (mode, options): (Option<LuaValue>, Option<LuaValue>)| {
                let pos = current_position(lua);
                let mode = mode.map(|v| json_from_lua(lua, v)).transpose()?;
                let options = optional_json_from_lua(lua, options)?;
                let result = runtime_c
                    .show_debug_overlay(
                        pos,
                        None,
                        mode.as_ref(),
                        options.as_ref().and_then(Value::as_object),
                        None,
                    )
                    .map_err(host_error)?;
                lua.to_value(&result)
            },
        )?,
    )?;

    let runtime_c = Arc::clone(&runtime);
    globals.set(
        "hide_debug_overlay",
        lua.create_function(move |lua, _: ()| {
            let pos = current_position(lua);
            let result = runtime_c.hide_debug_overlay(pos).map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    Ok(())
}

fn register_viewport_functions(lua: &Lua, runtime: Arc<ScriptRuntime>) -> mlua::Result<()> {
    let globals = lua.globals();

    let runtime_c = Arc::clone(&runtime);
    globals.set(
        "viewports_list",
        lua.create_function(move |lua, arg: Option<LuaValue>| {
            let pos = current_position(lua);
            let arg = optional_json_from_lua(lua, arg)?;
            let result = runtime_c
                .viewports_list(pos, arg.as_ref())
                .map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    let runtime_c = Arc::clone(&runtime);
    globals.set(
        "viewport_set_inner_size",
        lua.create_function(move |lua, size: LuaValue| {
            let pos = current_position(lua);
            let size = json_from_lua(lua, size)?;
            let result = runtime_c
                .viewport_set_inner_size(pos, &size, None)
                .map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    let runtime_c = Arc::clone(&runtime);
    globals.set(
        "focus_window",
        lua.create_function(move |lua, viewport: String| {
            let pos = current_position(lua);
            let result = runtime_c.focus_window(pos, viewport).map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    Ok(())
}

fn register_assertion_functions(lua: &Lua, runtime: Arc<ScriptRuntime>) -> mlua::Result<()> {
    let globals = lua.globals();

    let runtime_c = Arc::clone(&runtime);
    globals.set(
        "assert",
        lua.create_function(move |lua, (condition, message): (bool, Option<String>)| {
            let pos = current_position(lua);
            runtime_c
                .assert_condition(pos, condition, message)
                .map_err(host_error)?;
            Ok(())
        })?,
    )?;

    let runtime_c = Arc::clone(&runtime);
    globals.set(
        "assert_widget_exists",
        lua.create_function(move |lua, id: String| {
            let pos = current_position(lua);
            let target = Value::String(id);
            runtime_c
                .assert_widget_exists(pos, &target, None)
                .map_err(host_error)?;
            Ok(())
        })?,
    )?;

    Ok(())
}

fn register_fixture_functions(lua: &Lua, runtime: Arc<ScriptRuntime>) -> mlua::Result<()> {
    let globals = lua.globals();

    let runtime_c = Arc::clone(&runtime);
    globals.set(
        "fixture",
        lua.create_function(move |lua, name: String| {
            let pos = current_position(lua);
            let result = runtime_c.fixture(pos, name).map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    let runtime_c = Arc::clone(&runtime);
    globals.set(
        "fixtures",
        lua.create_function(move |lua, _: ()| {
            let pos = current_position(lua);
            let result = runtime_c.fixtures(pos).map_err(host_error)?;
            lua.to_value(&result)
        })?,
    )?;

    Ok(())
}

fn predicate_matches(
    lua: &Lua,
    runtime: &ScriptRuntime,
    function: Function,
    value: Value,
) -> ScriptResult<bool> {
    let lua_value = lua
        .to_value(&value)
        .map_err(|error| lua_error_info(runtime, &error))?;
    function
        .call::<bool>(lua_value)
        .map_err(|error| lua_error_info(runtime, &error))
}

fn json_from_lua(lua: &Lua, value: LuaValue) -> mlua::Result<Value> {
    lua.from_value(value)
}

fn optional_json_from_lua(lua: &Lua, value: Option<LuaValue>) -> mlua::Result<Option<Value>> {
    value.map(|value| json_from_lua(lua, value)).transpose()
}

fn current_position(lua: &Lua) -> ScriptPosition {
    lua.inspect_stack(1, |debug| ScriptPosition {
        line: debug.current_line(),
        column: None,
    })
    .or_else(|| {
        lua.inspect_stack(0, |debug| ScriptPosition {
            line: debug.current_line(),
            column: None,
        })
    })
    .unwrap_or_default()
}

/// Parse `"press"` / `"release"` into a `pressed: bool`.
fn parse_raw_action(action: &str) -> Result<bool, ScriptErrorInfo> {
    match action {
        "press" => Ok(true),
        "release" => Ok(false),
        _ => Err(ScriptErrorInfo {
            error_type: "type_error".to_string(),
            message: "action must be \"press\" or \"release\"".to_string(),
            location: None,
            backtrace: None,
            code: None,
            details: None,
        }),
    }
}

fn host_error(info: ScriptErrorInfo) -> LuaError {
    LuauHostError { info }.into_lua_err()
}

fn build_success_outcome(
    runtime: &ScriptRuntime,
    script_value: ScriptValue,
    timing: ScriptTiming,
) -> ScriptEvalOutcome {
    ScriptEvalOutcome {
        success: true,
        value: script_value.value,
        images: script_value.images,
        logs: runtime.logs(),
        assertions: runtime.assertions(),
        timing,
        error: None,
        content: script_value.content,
    }
}

fn build_error_outcome(
    runtime: &ScriptRuntime,
    info: ScriptErrorInfo,
    timing: ScriptTiming,
) -> ScriptEvalOutcome {
    ScriptEvalOutcome {
        success: false,
        value: None,
        images: None,
        logs: runtime.logs(),
        assertions: runtime.assertions(),
        timing,
        error: Some(info),
        content: Vec::new(),
    }
}

fn values_to_script_value(
    lua: &Lua,
    runtime: &ScriptRuntime,
    values: MultiValue,
) -> Result<ScriptValue, ScriptErrorInfo> {
    let value = match values.len() {
        0 => return Ok(ScriptValue::default()),
        1 => lua
            .from_value(values.into_vec().into_iter().next().expect("one value"))
            .map_err(|error| serialization_error(runtime, &error))?,
        _ => {
            let mut array = Vec::with_capacity(values.len());
            for value in values {
                array.push(
                    lua.from_value(value)
                        .map_err(|error| serialization_error(runtime, &error))?,
                );
            }
            Value::Array(array)
        }
    };
    let mut collector = ImageReferenceCollector::default();
    collect_image_refs(&value, &mut collector);
    let images = build_image_blocks(runtime, &collector);
    let image_infos = if images.infos.is_empty() {
        None
    } else {
        Some(images.infos)
    };
    Ok(ScriptValue {
        value: Some(value),
        images: image_infos,
        content: images.blocks,
    })
}

fn serialization_error(runtime: &ScriptRuntime, error: &LuaError) -> ScriptErrorInfo {
    let mut info = lua_error_info(runtime, error);
    info.error_type = "type_error".to_string();
    info
}

struct ImageBlocks {
    infos: Vec<ScriptImageInfo>,
    blocks: Vec<ContentBlock>,
}

fn build_image_blocks(runtime: &ScriptRuntime, collector: &ImageReferenceCollector) -> ImageBlocks {
    let mut infos = Vec::new();
    let mut blocks = Vec::new();
    for image in runtime.images() {
        if !collector.contains(&image.id) {
            continue;
        }
        let content_index = infos.len() + 1;
        infos.push(ScriptImageInfo {
            id: image.id.clone(),
            content_index,
            kind: image.kind.as_str().to_string(),
            viewport_id: Some(image.viewport_id.clone()),
            target: image
                .target
                .clone()
                .and_then(|target| serde_json::to_value(target).ok()),
            rect: image.rect.and_then(|rect| serde_json::to_value(rect).ok()),
            metadata: None,
        });
        blocks.push(ContentBlock::image(image.data.clone(), "image/jpeg"));
    }
    ImageBlocks { infos, blocks }
}

fn lua_error_info(runtime: &ScriptRuntime, error: &LuaError) -> ScriptErrorInfo {
    if let Some(mut info) = find_host_error(error) {
        if info.backtrace.is_none() {
            info.backtrace = collect_backtrace(error);
        }
        return info;
    }
    if let Some(timeout_ms) = find_timeout_error(error) {
        return ScriptErrorInfo {
            error_type: "timeout".to_string(),
            message: format!("Script timed out after {timeout_ms}ms"),
            location: parse_error_location(error, runtime),
            backtrace: collect_backtrace(error),
            code: None,
            details: None,
        };
    }

    let message = match error {
        LuaError::SyntaxError { message, .. } => message.clone(),
        LuaError::RuntimeError(message) => message.clone(),
        LuaError::CallbackError { cause, .. } => cause.to_string(),
        _ => error.to_string(),
    };
    let error_type = match error {
        LuaError::SyntaxError { .. } => "parse",
        _ => "runtime",
    };
    ScriptErrorInfo {
        error_type: error_type.to_string(),
        message,
        location: parse_error_location(error, runtime),
        backtrace: collect_backtrace(error),
        code: None,
        details: None,
    }
}

fn find_host_error(error: &LuaError) -> Option<ScriptErrorInfo> {
    match error {
        LuaError::ExternalError(error) => error
            .downcast_ref::<LuauHostError>()
            .map(|error| error.info.clone()),
        LuaError::CallbackError { cause, .. }
        | LuaError::WithContext { cause, .. }
        | LuaError::BadArgument { cause, .. } => find_host_error(cause),
        _ => error
            .downcast_ref::<LuauHostError>()
            .map(|error| error.info.clone()),
    }
}

fn find_timeout_error(error: &LuaError) -> Option<u64> {
    match error {
        LuaError::ExternalError(error) => error
            .downcast_ref::<LuauTimeoutError>()
            .map(|error| error.timeout_ms),
        LuaError::CallbackError { cause, .. }
        | LuaError::WithContext { cause, .. }
        | LuaError::BadArgument { cause, .. } => find_timeout_error(cause),
        _ => error
            .downcast_ref::<LuauTimeoutError>()
            .map(|error| error.timeout_ms),
    }
}

fn parse_error_location(
    error: &LuaError,
    runtime: &ScriptRuntime,
) -> Option<super::types::ScriptLocation> {
    match error {
        LuaError::SyntaxError { message, .. } => {
            parse_location_text(message, runtime.source_name())
        }
        LuaError::RuntimeError(message) => parse_location_text(message, runtime.source_name()),
        LuaError::CallbackError { traceback, cause } => {
            parse_location_text(traceback, runtime.source_name())
                .or_else(|| parse_error_location(cause, runtime))
        }
        _ => parse_location_text(&error.to_string(), runtime.source_name()),
    }
}

fn parse_location_text(text: &str, source_name: &str) -> Option<super::types::ScriptLocation> {
    for line in text.lines() {
        if let Some(location) = parse_location_fragment(line, source_name) {
            return Some(location);
        }
    }
    parse_location_fragment(text, source_name)
}

fn parse_location_fragment(text: &str, source_name: &str) -> Option<super::types::ScriptLocation> {
    let candidates = [format!("@{source_name}:"), format!("{source_name}:")];
    for prefix in candidates {
        let Some(start) = text.find(&prefix) else {
            continue;
        };
        let remainder = &text[start + prefix.len()..];
        let digits = remainder
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>();
        if digits.is_empty() {
            continue;
        }
        let line = digits.parse().ok()?;
        return Some(super::types::ScriptLocation { line, column: None });
    }
    None
}

fn collect_backtrace(error: &LuaError) -> Option<Vec<String>> {
    match error {
        LuaError::CallbackError { traceback, cause } => {
            let mut frames = traceback
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            if let Some(nested) = collect_backtrace(cause) {
                frames.extend(nested);
            }
            if frames.is_empty() {
                None
            } else {
                Some(frames)
            }
        }
        _ => None,
    }
}
