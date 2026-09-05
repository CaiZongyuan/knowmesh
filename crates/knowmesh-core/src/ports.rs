use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    canonical::snapshot::{CanonicalSnapshot, FileManifest, SnapshotWarning},
    domain::WorkspaceId,
    error::AppResult,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReconcileReport {
    pub generation: u64,
    pub changed: bool,
    pub source_count: usize,
    pub node_count: usize,
    pub claim_count: usize,
    pub relation_count: usize,
    pub evidence_count: usize,
    pub synthesis_count: usize,
}

pub trait ProjectionStore: Send {
    fn reconcile(&mut self, snapshot: &CanonicalSnapshot) -> AppResult<ReconcileReport>;
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectionState {
    pub workspace_id: WorkspaceId,
    pub schema_hash: String,
    pub snapshot_sha256: String,
    pub projection: ReconcileReport,
    pub files: Vec<FileManifest>,
    pub warnings: Option<Vec<SnapshotWarning>>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DatabaseDiagnostics {
    pub sqlite_version: String,
    pub schema_version: u32,
    pub journal_mode: String,
    pub foreign_keys: bool,
    pub busy_timeout_ms: u64,
    pub integrity: String,
    pub foreign_key_violations: usize,
}

pub trait IndexStore: ProjectionStore {
    fn projection_state(&self) -> AppResult<ProjectionState>;
    fn diagnostics(&self) -> AppResult<DatabaseDiagnostics>;
}
