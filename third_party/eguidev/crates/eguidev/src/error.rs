#![allow(missing_docs)]

use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ToolError {
    pub(crate) code: ErrorCode,
    pub(crate) message: String,
    pub(crate) details: Option<Value>,
}

impl ToolError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn details(&self) -> Option<&Value> {
        self.details.as_ref()
    }

    pub fn into_parts(self) -> (ErrorCode, String, Option<Value>) {
        (self.code, self.message, self.details)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    NotFound,
    Ambiguous,
    InvalidRef,
    TargetNotFocusable,
    FocusNotAcquired,
    TargetDetached,
    DuplicateWidgetId,
    Timeout,
    Internal,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Ambiguous => "ambiguous",
            Self::InvalidRef => "invalid_ref",
            Self::TargetNotFocusable => "target_not_focusable",
            Self::FocusNotAcquired => "focus_not_acquired",
            Self::TargetDetached => "target_detached",
            Self::DuplicateWidgetId => "duplicate_widget_id",
            Self::Timeout => "timeout",
            Self::Internal => "internal",
        }
    }
}
