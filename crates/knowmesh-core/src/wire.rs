use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    domain::{RunId, WorkspaceId},
    error::AppError,
};

pub const API_CONTRACT_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Metadata {
    pub schema_version: String,
    pub command: String,
    pub trace_id: RunId,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<WorkspaceId>,
}

impl Metadata {
    pub fn new(command: impl Into<String>, trace_id: RunId, duration_ms: u64) -> Self {
        Self {
            schema_version: "1".into(),
            command: command.into(),
            trace_id,
            duration_ms,
            workspace_id: None,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Success<T> {
    pub ok: bool,
    pub data: T,
    pub meta: Metadata,
}

impl<T> Success<T> {
    pub fn new(data: T, meta: Metadata) -> Self {
        Self {
            ok: true,
            data,
            meta,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Failure {
    pub ok: bool,
    pub error: AppError,
    pub meta: Metadata,
}

impl Failure {
    pub fn new(error: AppError, meta: Metadata) -> Self {
        Self {
            ok: false,
            error,
            meta,
        }
    }
}
