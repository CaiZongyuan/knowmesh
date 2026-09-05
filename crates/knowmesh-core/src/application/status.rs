use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    canonical::{schema::Schema, snapshot::SnapshotWarning, workspace::Workspace},
    domain::WorkspaceId,
    error::{AppError, AppResult, ErrorType},
    ports::{IndexStore, ReconcileReport},
};

use super::sync;

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusInput {
    #[serde(default)]
    pub no_sync: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SyncSkipped {
    Requested,
    RecoveryRequired,
    WriterActive,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StatusReport {
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub schema_hash: String,
    pub indexed_schema_hash: String,
    pub projection: ReconcileReport,
    pub recovery_required: bool,
    pub sync_skipped: Option<SyncSkipped>,
    pub fast_path: bool,
    pub warnings: Vec<SnapshotWarning>,
}

pub fn get(
    workspace: &Workspace,
    store: &mut dyn IndexStore,
    input: &StatusInput,
) -> AppResult<StatusReport> {
    let schema = Schema::load(workspace)?;
    let recovery = sync::recovery_status(workspace)?;
    let mut fast_path = false;
    let skipped = if recovery.recovery_required {
        Some(SyncSkipped::RecoveryRequired)
    } else if input.no_sync {
        Some(SyncSkipped::Requested)
    } else {
        match sync::fast_synchronize(workspace, store) {
            Ok(report) => {
                fast_path = report.fast_path;
                None
            }
            Err(error) if error.code == "WORKSPACE_LOCKED" => Some(SyncSkipped::WriterActive),
            Err(error) if error.code == "TRANSACTION_RECOVERY_REQUIRED" => {
                Some(SyncSkipped::RecoveryRequired)
            }
            Err(error) => return Err(error),
        }
    };
    let state = store.projection_state()?;
    if state.workspace_id != workspace.config.workspace.id {
        return Err(AppError::new(
            ErrorType::Configuration,
            "WORKSPACE_ID_MISMATCH",
            "The index belongs to another workspace.",
        ));
    }
    let recovery = sync::recovery_status(workspace)?;
    Ok(StatusReport {
        workspace_id: workspace.config.workspace.id.clone(),
        name: workspace.config.workspace.name.clone(),
        schema_hash: schema.hash,
        indexed_schema_hash: state.schema_hash,
        projection: state.projection,
        recovery_required: recovery.recovery_required,
        sync_skipped: skipped,
        fast_path,
        warnings: state.warnings.unwrap_or_default(),
    })
}
