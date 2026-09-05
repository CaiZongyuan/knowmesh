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

#[derive(Debug, Serialize, JsonSchema)]
pub struct RuntimeCopyReport {
    pub table_counts: std::collections::BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RebuildInput {
    pub dry_run: bool,
    pub yes: bool,
    pub discard_runtime: bool,
    pub keep_backups: usize,
}

impl Default for RebuildInput {
    fn default() -> Self {
        Self {
            dry_run: false,
            yes: false,
            discard_runtime: false,
            keep_backups: 3,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RebuildReport {
    pub dry_run: bool,
    pub projection: ReconcileReport,
    pub logical_sha256: String,
    pub runtime_table_counts: std::collections::BTreeMap<String, u64>,
    pub discarded_runtime_tables: Vec<String>,
    pub backup_paths: Vec<std::path::PathBuf>,
    pub retained_candidate_paths: Vec<std::path::PathBuf>,
    pub warnings: Vec<SnapshotWarning>,
}

pub trait RebuildCandidate: Send {
    fn publish(self: Box<Self>, current: &CanonicalSnapshot) -> AppResult<RebuildReport>;
}

pub trait RebuildBackend: Send + Sync {
    fn preview(
        &self,
        snapshot: &CanonicalSnapshot,
        input: &RebuildInput,
    ) -> AppResult<RebuildReport>;
    fn prepare(
        &self,
        snapshot: &CanonicalSnapshot,
        input: &RebuildInput,
    ) -> AppResult<Box<dyn RebuildCandidate>>;
}
