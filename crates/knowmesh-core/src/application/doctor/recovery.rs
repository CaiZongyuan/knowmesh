use std::path::{Path, PathBuf};

use crate::{
    canonical::{
        snapshot::CanonicalSnapshot,
        transaction::{self, TransactionManifest, WorkspaceWriter},
        workspace::{Workspace, WorkspaceConfig, resolve_workspace_inner},
    },
    error::{AppError, AppResult, ErrorType},
    ports::IndexStore,
};

use super::{
    DoctorReport, IndexAccess, RepairInput, from_error, inspect_context, sync, validate_repair,
};

pub fn resolve_root(
    explicit: Option<&Path>,
    environment: Option<&Path>,
    cwd: &Path,
) -> AppResult<PathBuf> {
    resolve_workspace_inner(explicit, environment, cwd, true)
}

pub fn inspect_root(root: &Path, access: IndexAccess<'_>) -> AppResult<DoctorReport> {
    match Workspace::load(root) {
        Ok(workspace) => super::inspect(&workspace, access),
        Err(error) => inspect_context(root, None, access, vec![from_error(error)]),
    }
}

pub(super) fn preflight(
    root: &Path,
    access: &IndexAccess<'_>,
) -> AppResult<Option<TransactionManifest>> {
    let mut transactions = transaction::pending(root)?;
    if transactions.len() > 1 {
        return Err(transaction::recovery_required());
    }
    let Some(transaction) = transactions.pop() else {
        return Ok(None);
    };
    transaction::verify_recovery(root, &transaction)?;
    let config = WorkspaceConfig::parse(&transaction::recovery_content(
        root,
        &transaction,
        Path::new("knowmesh.yaml"),
        1024 * 1024,
    )?)?;
    match access {
        IndexAccess::Failed(error) => return Err(error.clone()),
        IndexAccess::Ready(store)
            if store.projection_state()?.workspace_id != config.workspace.id =>
        {
            return Err(AppError::new(
                ErrorType::Configuration,
                "WORKSPACE_ID_MISMATCH",
                "The recovery database belongs to another workspace.",
            ));
        }
        _ => {}
    }
    Ok(Some(transaction))
}

pub fn repair_root(
    root: &Path,
    access: IndexAccess<'_>,
    input: &RepairInput,
    open_store: impl FnOnce(&Workspace) -> AppResult<Box<dyn IndexStore>>,
) -> AppResult<DoctorReport> {
    validate_repair(input)?;
    if input.dry_run {
        let mut report = inspect_root(root, access)?;
        report.dry_run = true;
        return Ok(report);
    }
    let writer = WorkspaceWriter::acquire(root)?;
    let transaction = preflight(root, &access)?;
    let (workspace, store, snapshot, projection) = if let Some(transaction) = &transaction
        && transaction.proposal.is_some()
    {
        let workspace = Workspace::load(root)?;
        let mut store = open_store(&workspace)?;
        let applied = crate::application::proposal::apply::resume(
            &workspace,
            store.as_mut(),
            &writer,
            transaction,
        )?;
        let snapshot = CanonicalSnapshot::scan(&workspace)?;
        (
            workspace,
            store,
            snapshot,
            applied
                .projection
                .ok_or_else(transaction::recovery_required)?,
        )
    } else {
        if let Some(transaction) = &transaction {
            writer.apply(&transaction.id)?;
        }
        let workspace = Workspace::load(root)?;
        let snapshot = match &transaction {
            Some(transaction) => CanonicalSnapshot::scan_committed(&workspace, &transaction.id)?,
            None => CanonicalSnapshot::scan(&workspace)?,
        };
        let mut store = open_store(&workspace)?;
        let projection = store.reconcile(&snapshot)?;
        if let Some(transaction) = &transaction {
            writer.mark_indexed(&transaction.id)?;
        }
        (workspace, store, snapshot, projection)
    };
    let mut report = super::inspect(&workspace, IndexAccess::Ready(store.as_ref()))?;
    let mut recovery = sync::recovery_status(&workspace)?;
    if let Some(transaction) = transaction {
        recovery.recovered_transaction_ids.push(transaction.id);
        recovery.projection = Some(projection.clone());
    }
    report.recovery = Some(recovery);
    report.sync = Some(sync::SyncReport {
        dry_run: false,
        fast_path: false,
        projection: Some(projection),
        files: snapshot.files,
        warnings: snapshot.warnings,
    });
    Ok(report)
}
