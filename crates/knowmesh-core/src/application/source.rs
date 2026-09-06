use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    canonical::{
        snapshot::CanonicalSnapshot,
        source::{ImportInput, ImportReport, ImportedContent, SourceLibrary, SourcePlan},
        transaction::WorkspaceWriter,
        workspace::Workspace,
    },
    domain::SourceId,
    error::{AppError, AppResult, ErrorType},
    ports::{ImpactPreviewBackend, ProjectionStore, ReconcileReport},
};

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveInput {
    pub source_id: SourceId,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub yes: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SourceWriteReport {
    #[serde(flatten)]
    pub import: ImportReport,
    pub projection: Option<ReconcileReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impact: Option<super::impact::ImpactReport>,
}

pub fn preview_add(
    workspace: &Workspace,
    input: &ImportInput,
    imported: Option<ImportedContent>,
) -> AppResult<SourceWriteReport> {
    CanonicalSnapshot::scan(workspace)?;
    let plan = SourceLibrary::new(workspace).plan_add(input, imported)?;
    Ok(SourceWriteReport {
        import: plan.report(true),
        projection: None,
        impact: None,
    })
}

pub fn add(
    workspace: &Workspace,
    store: &mut dyn ProjectionStore,
    input: &ImportInput,
    imported: Option<ImportedContent>,
) -> AppResult<SourceWriteReport> {
    if input.dry_run {
        return preview_add(workspace, input, imported);
    }
    let writer = WorkspaceWriter::acquire(&workspace.root)?;
    let before = CanonicalSnapshot::scan(workspace)?;
    let plan = SourceLibrary::new(workspace).plan_add(input, imported)?;
    commit(workspace, store, &writer, before, plan)
}

pub fn preview_remove(workspace: &Workspace, input: &RemoveInput) -> AppResult<SourceWriteReport> {
    removal_preview(workspace, input, None)
}

pub fn preview_remove_with_impact(
    workspace: &Workspace,
    input: &RemoveInput,
    backend: &dyn ImpactPreviewBackend,
) -> AppResult<SourceWriteReport> {
    removal_preview(workspace, input, Some(backend))
}

fn removal_preview(
    workspace: &Workspace,
    input: &RemoveInput,
    backend: Option<&dyn ImpactPreviewBackend>,
) -> AppResult<SourceWriteReport> {
    let snapshot = CanonicalSnapshot::scan(workspace)?;
    let plan = SourceLibrary::new(workspace).plan_remove(&input.source_id)?;
    Ok(SourceWriteReport {
        import: plan.report(true),
        projection: None,
        impact: backend
            .map(|backend| super::impact::preview(workspace, &snapshot, &input.source_id, backend))
            .transpose()?,
    })
}

pub fn remove(
    workspace: &Workspace,
    store: &mut dyn ProjectionStore,
    input: &RemoveInput,
) -> AppResult<SourceWriteReport> {
    if input.dry_run {
        return preview_remove(workspace, input);
    }
    if !input.yes {
        return Err(AppError::new(
            ErrorType::Confirmation,
            "CONFIRMATION_REQUIRED",
            "Source removal requires explicit confirmation.",
        )
        .with_hint("Review `source remove --dry-run`, then repeat with --yes."));
    }
    let writer = WorkspaceWriter::acquire(&workspace.root)?;
    let before = CanonicalSnapshot::scan(workspace)?;
    let plan = SourceLibrary::new(workspace).plan_remove(&input.source_id)?;
    commit(workspace, store, &writer, before, plan)
}

fn commit(
    workspace: &Workspace,
    store: &mut dyn ProjectionStore,
    writer: &WorkspaceWriter,
    before: CanonicalSnapshot,
    plan: SourcePlan,
) -> AppResult<SourceWriteReport> {
    let import = plan.report(false);
    let previous = store.reconcile(&before)?;
    let projection = if plan.changes.is_empty() {
        previous
    } else {
        let id = writer.prepare(plan.changes)?;
        writer.apply(&id)?;
        let snapshot = CanonicalSnapshot::scan_committed(workspace, &id)?;
        let projection = store.reconcile(&snapshot)?;
        writer.mark_indexed(&id)?;
        projection
    };
    Ok(SourceWriteReport {
        import,
        projection: Some(projection),
        impact: None,
    })
}
