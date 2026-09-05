use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    canonical::{
        snapshot::{CanonicalSnapshot, FileManifest, SnapshotWarning},
        transaction::{TransactionState, WorkspaceWriter, pending, recovery_required},
        workspace::Workspace,
    },
    error::AppResult,
    ports::{ProjectionStore, ReconcileReport},
};

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SyncInput {
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SyncReport {
    pub dry_run: bool,
    pub projection: Option<ReconcileReport>,
    pub files: Vec<FileManifest>,
    pub warnings: Vec<SnapshotWarning>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RecoveryTransaction {
    pub id: String,
    pub state: String,
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RecoveryReport {
    pub recovery_required: bool,
    pub transactions: Vec<RecoveryTransaction>,
    pub recovered_transaction_ids: Vec<String>,
    pub projection: Option<ReconcileReport>,
}

pub fn preview(workspace: &Workspace) -> AppResult<SyncReport> {
    let snapshot = CanonicalSnapshot::scan(workspace)?;
    Ok(SyncReport {
        dry_run: true,
        projection: None,
        files: snapshot.files,
        warnings: snapshot.warnings,
    })
}

pub fn synchronize(
    workspace: &Workspace,
    store: &mut dyn ProjectionStore,
) -> AppResult<SyncReport> {
    let _writer = WorkspaceWriter::acquire(&workspace.root)?;
    let snapshot = CanonicalSnapshot::scan(workspace)?;
    let projection = store.reconcile(&snapshot)?;
    Ok(SyncReport {
        dry_run: false,
        projection: Some(projection),
        files: snapshot.files,
        warnings: snapshot.warnings,
    })
}

pub fn recovery_status(workspace: &Workspace) -> AppResult<RecoveryReport> {
    let transactions = pending(&workspace.root)?
        .into_iter()
        .map(|tx| RecoveryTransaction {
            id: tx.id,
            state: match tx.state {
                TransactionState::Prepared => "prepared",
                TransactionState::CanonicalCommitted => "canonical_committed",
                TransactionState::Indexed => "indexed",
            }
            .into(),
            paths: tx.changes.into_iter().map(|change| change.path).collect(),
        })
        .collect::<Vec<_>>();
    Ok(RecoveryReport {
        recovery_required: !transactions.is_empty(),
        transactions,
        recovered_transaction_ids: vec![],
        projection: None,
    })
}

pub fn recover(
    workspace: &Workspace,
    store: &mut dyn ProjectionStore,
) -> AppResult<RecoveryReport> {
    let writer = WorkspaceWriter::acquire(&workspace.root)?;
    let transactions = writer.pending()?;
    if transactions.len() > 1 {
        return Err(recovery_required());
    }
    let mut recovered = Vec::new();
    let mut projection = None;
    for transaction in transactions {
        writer.apply(&transaction.id)?;
        let current = Workspace::load(&workspace.root)?;
        let snapshot = CanonicalSnapshot::scan_committed(&current, &transaction.id)?;
        projection = Some(store.reconcile(&snapshot)?);
        writer.mark_indexed(&transaction.id)?;
        recovered.push(transaction.id);
    }
    let mut report = recovery_status(workspace)?;
    report.recovered_transaction_ids = recovered;
    report.projection = projection;
    Ok(report)
}
