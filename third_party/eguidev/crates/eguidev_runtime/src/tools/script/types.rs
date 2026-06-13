use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::Value;
use tmcp::schema::{CallToolResult, ContentBlock};

use crate::types::{Rect, WidgetRef};

pub(super) type ScriptResult<T> = Result<T, ScriptErrorInfo>;

/// Scalar value that can be injected into the global Luau `args` table.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum ScriptArgValue {
    /// String-valued script arg.
    String(String),
    /// Integer-valued script arg.
    Int(i64),
    /// Floating-point script arg.
    Float(f64),
    /// Boolean-valued script arg.
    Bool(bool),
}

impl<'de> Deserialize<'de> for ScriptArgValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        match value {
            Value::String(value) => Ok(Self::String(value)),
            Value::Bool(value) => Ok(Self::Bool(value)),
            Value::Number(number) => {
                if let Some(value) = number.as_i64() {
                    return Ok(Self::Int(value));
                }
                if number.is_u64() {
                    return Err(de::Error::custom("script arg integers must fit in i64"));
                }
                number
                    .as_f64()
                    .map(Self::Float)
                    .ok_or_else(|| de::Error::custom("script arg number is not representable"))
            }
            Value::Null => Err(de::Error::custom("script args do not accept null values")),
            Value::Array(_) => Err(de::Error::custom("script args do not accept arrays")),
            Value::Object(_) => Err(de::Error::custom("script args do not accept objects")),
        }
    }
}

/// Deterministic map of script args exposed to Luau as the global `args` table.
pub type ScriptArgs = BTreeMap<String, ScriptArgValue>;

/// Options for evaluating a Luau script.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct ScriptEvalOptions {
    /// Optional source name used in diagnostics and error messages.
    pub source_name: Option<String>,
    /// Optional JSON object exposed to the script as the global `args` table.
    #[serde(default)]
    pub args: ScriptArgs,
}

/// Request payload for the `script_eval` MCP tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ScriptEvalRequest {
    /// Luau source code to execute.
    pub script: String,
    /// Optional evaluation timeout in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Optional extra evaluation options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<ScriptEvalOptions>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ScriptPosition {
    pub(super) line: Option<usize>,
    pub(super) column: Option<usize>,
}

/// Source location reported for a script error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptLocation {
    /// One-based line number.
    pub line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// One-based column number when available.
    pub column: Option<usize>,
}

/// Assertion outcome recorded during script execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptAssertion {
    /// Whether the assertion passed.
    pub passed: bool,
    /// Assertion message recorded by the runtime.
    pub message: String,
    /// Human-readable source location for the assertion.
    pub location: String,
}

/// Timing information for a script evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptTiming {
    /// Time spent compiling the script in milliseconds.
    pub compile_ms: u64,
    /// Time spent executing the script in milliseconds.
    pub exec_ms: u64,
    /// Total wall-clock evaluation time in milliseconds.
    pub total_ms: u64,
}

impl ScriptTiming {
    pub(crate) fn zero() -> Self {
        Self {
            compile_ms: 0,
            exec_ms: 0,
            total_ms: 0,
        }
    }
}

/// Error details reported by the script runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptErrorInfo {
    #[serde(rename = "type")]
    /// Broad error category such as `parse`, `runtime`, or `assertion`.
    pub error_type: String,
    /// Error message presented to the caller.
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional source location for the error.
    pub location: Option<ScriptLocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional backtrace rendered by the script runtime.
    pub backtrace: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional machine-readable error code for structured tool failures.
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional extra error details supplied by the runtime.
    pub details: Option<Value>,
}

/// Metadata for an image captured during script execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptImageInfo {
    /// Stable image identifier assigned by the runtime.
    pub id: String,
    /// Index of the image content block in the MCP result payload.
    pub content_index: usize,
    /// Image kind such as `viewport` or `widget`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Viewport that produced the image when known.
    pub viewport_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Serialized widget target metadata when the image came from a widget.
    pub target: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Captured rectangle metadata when available.
    pub rect: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Extra metadata attached by the runtime.
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ScriptValue {
    pub(super) value: Option<Value>,
    pub(super) images: Option<Vec<ScriptImageInfo>>,
    pub(super) content: Vec<ContentBlock>,
}

/// Structured result of evaluating a Luau script directly against a `DevMcp` instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptEvalOutcome {
    /// Whether evaluation completed successfully.
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional JSON-serializable script return value.
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional metadata for images returned by the script.
    pub images: Option<Vec<ScriptImageInfo>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    /// Log lines emitted through the script `log(...)` helper.
    pub logs: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    /// Assertion outcomes recorded during evaluation.
    pub assertions: Vec<ScriptAssertion>,
    /// Timing information for the evaluation.
    pub timing: ScriptTiming,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Error details when evaluation fails.
    pub error: Option<ScriptErrorInfo>,
    #[serde(skip)]
    pub(super) content: Vec<ContentBlock>,
}

impl ScriptEvalOutcome {
    pub(crate) fn to_tool_result(&self) -> CallToolResult {
        let mut result = match CallToolResult::new().with_json_text(self) {
            Ok(result) => result,
            Err(error) => {
                let fallback = serde_json::json!({
                    "success": false,
                    "value": null,
                    "logs": [],
                    "assertions": [],
                    "timing": ScriptTiming::zero(),
                    "error": {
                        "type": "runtime",
                        "message": format!("Failed to serialize output: {error}"),
                    },
                });
                CallToolResult::new()
                    .with_json_text(&fallback)
                    .unwrap_or_else(|_| {
                        CallToolResult::new().with_text_content(
                            r#"{"success":false,"error":{"type":"runtime","message":"Failed to serialize output"}}"#,
                        )
                    })
            }
        };
        for block in &self.content {
            result = result.with_content(block.clone());
        }
        result
    }

    pub(crate) fn error_only(error: ScriptErrorInfo) -> Self {
        Self {
            success: false,
            value: None,
            images: None,
            logs: Vec::new(),
            assertions: Vec::new(),
            timing: ScriptTiming::zero(),
            error: Some(error),
            content: Vec::new(),
        }
    }
}

impl From<ScriptEvalOutcome> for CallToolResult {
    fn from(outcome: ScriptEvalOutcome) -> Self {
        outcome.to_tool_result()
    }
}

#[derive(Debug, Clone)]
pub(super) enum ScriptImageKind {
    Viewport,
    Widget,
}

impl ScriptImageKind {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::Viewport => "viewport",
            Self::Widget => "widget",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct ImageCapture {
    pub(super) id: String,
    pub(super) data: String,
    pub(super) kind: ScriptImageKind,
    pub(super) viewport_id: String,
    pub(super) target: Option<WidgetRef>,
    pub(super) rect: Option<Rect>,
}

#[derive(Debug, Default)]
pub(super) struct ImageReferenceCollector {
    pub(super) used: Vec<String>,
    seen: HashSet<String>,
}

impl ImageReferenceCollector {
    pub(super) fn record(&mut self, id: &str) {
        if self.seen.insert(id.to_string()) {
            self.used.push(id.to_string());
        }
    }

    pub(super) fn contains(&self, id: &str) -> bool {
        self.seen.contains(id)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ScriptArgValue, ScriptEvalOptions};

    #[test]
    fn script_eval_options_default_args_to_empty_map() {
        let options: ScriptEvalOptions =
            serde_json::from_value(json!({ "source_name": "test.luau" })).expect("options");
        assert_eq!(options.source_name.as_deref(), Some("test.luau"));
        assert!(options.args.is_empty());
    }

    #[test]
    fn script_eval_options_round_trip_scalar_args() {
        let input = json!({
            "source_name": "test.luau",
            "args": {
                "name": "Sky",
                "count": 4,
                "ratio": 1.5,
                "enabled": true
            }
        });
        let options: ScriptEvalOptions = serde_json::from_value(input.clone()).expect("options");
        assert_eq!(
            options.args["name"],
            ScriptArgValue::String("Sky".to_string())
        );
        assert_eq!(options.args["count"], ScriptArgValue::Int(4));
        assert_eq!(options.args["ratio"], ScriptArgValue::Float(1.5));
        assert_eq!(options.args["enabled"], ScriptArgValue::Bool(true));
        assert_eq!(serde_json::to_value(options).expect("serialize"), input);
    }

    #[test]
    fn script_eval_options_reject_invalid_arg_shapes() {
        for invalid in [
            json!({ "args": null }),
            json!({ "args": { "bad": [1, 2, 3] } }),
            json!({ "args": { "bad": { "nested": true } } }),
        ] {
            let error = serde_json::from_value::<ScriptEvalOptions>(invalid).expect_err("invalid");
            assert!(!error.to_string().is_empty());
        }
    }
}
