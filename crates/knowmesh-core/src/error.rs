use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorType {
    Validation,
    NotFound,
    Configuration,
    Io,
    Network,
    Internal,
    Policy,
    Conflict,
    Model,
    Confirmation,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, thiserror::Error)]
#[error("{message}")]
pub struct AppError {
    #[serde(rename = "type")]
    pub error_type: ErrorType,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Box<Value>>,
}

impl AppError {
    pub fn new(error_type: ErrorType, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error_type,
            code: code.into(),
            message: message.into(),
            hint: None,
            retryable: false,
            param: None,
            details: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
    pub fn with_param(mut self, param: impl Into<String>) -> Self {
        self.param = Some(param.into());
        self
    }
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(Box::new(details));
        self
    }
    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    pub fn exit_code(&self) -> u8 {
        match self.error_type {
            ErrorType::Validation => 2,
            ErrorType::NotFound | ErrorType::Configuration => 3,
            ErrorType::Io | ErrorType::Network => 4,
            ErrorType::Internal => 5,
            ErrorType::Policy => 6,
            ErrorType::Conflict => 7,
            ErrorType::Model => 8,
            ErrorType::Confirmation => 10,
            ErrorType::Cancelled => 130,
        }
    }
}
