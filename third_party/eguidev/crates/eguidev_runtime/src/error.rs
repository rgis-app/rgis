use eguidev::internal::error as base_error;
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

    pub(crate) fn into_tmcp(self) -> tmcp::ToolError {
        let code = self.code.as_str();
        let message = self.message;
        let mut structured = serde_json::json!({
            "error": {
                "code": code,
                "message": message,
            }
        });
        if let Some(details) = self.details
            && let Some(error) = structured.get_mut("error")
            && let Some(map) = error.as_object_mut()
        {
            map.insert("details".to_string(), details);
        }
        tmcp::ToolError::new(code, message).with_structured(structured)
    }
}

impl From<base_error::ToolError> for ToolError {
    fn from(error: base_error::ToolError) -> Self {
        let (code, message, details) = error.into_parts();
        Self {
            code: code.into(),
            message,
            details,
        }
    }
}

impl From<ToolError> for tmcp::ToolError {
    fn from(error: ToolError) -> Self {
        error.into_tmcp()
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
    pub(crate) fn as_str(self) -> &'static str {
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

impl From<base_error::ErrorCode> for ErrorCode {
    fn from(code: base_error::ErrorCode) -> Self {
        match code {
            base_error::ErrorCode::NotFound => Self::NotFound,
            base_error::ErrorCode::Ambiguous => Self::Ambiguous,
            base_error::ErrorCode::InvalidRef => Self::InvalidRef,
            base_error::ErrorCode::TargetNotFocusable => Self::TargetNotFocusable,
            base_error::ErrorCode::FocusNotAcquired => Self::FocusNotAcquired,
            base_error::ErrorCode::TargetDetached => Self::TargetDetached,
            base_error::ErrorCode::DuplicateWidgetId => Self::DuplicateWidgetId,
            base_error::ErrorCode::Timeout => Self::Timeout,
            base_error::ErrorCode::Internal => Self::Internal,
        }
    }
}
