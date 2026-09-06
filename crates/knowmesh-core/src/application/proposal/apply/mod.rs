mod types;
pub use types::*;

use super::{ProposalRecord, prepare_accepted};
use crate::{
    canonical::{
        schema::Schema,
        snapshot::CanonicalSnapshot,
        source::SourceLibrary,
        transaction::{FileChange, TransactionManifest, WorkspaceWriter, recovery_required},
        workspace::Workspace,
    },
    domain::{Timestamp, sha256},
    error::{AppError, AppResult, ErrorType},
    ports::{IndexStore, ProposalStore},
};

pub fn execute(
    workspace: &Workspace,
    store: &mut dyn ProposalStore,
    input: &ApplyInput,
    actor: &str,
    now: Timestamp,
) -> AppResult<ApplyReport> {
    if input.expected_revision == 0 {
        return Err(conflict(
            "PROPOSAL_REVISION_MISMATCH",
            "An expected Proposal revision is required.",
        ));
    }
    if !input.dry_run && !input.yes {
        return Err(AppError::new(
            ErrorType::Confirmation,
            "CONFIRMATION_REQUIRED",
            "Applying a Proposal requires explicit confirmation.",
        ));
    }
    if input.dry_run {
        let record = store.proposal_get(&input.proposal_id, None)?;
        let state = store.projection_state()?;
        if state.workspace_id != workspace.config.workspace.id {
            return Err(workspace_mismatch());
        }
        let preview = prepare_accepted(
            workspace,
            &record.proposal,
            input.expected_revision,
            state.projection.generation,
            &record.base_snapshot_sha256,
        )?;
        return Ok(ApplyReport {
            dry_run: true,
            proposal_id: input.proposal_id.clone(),
            reviewed_revision: input.expected_revision,
            applied_revision: None,
            projection: None,
            changed_paths: preview.documents().keys().cloned().collect(),
            transaction_id: None,
        });
    }
    let writer = WorkspaceWriter::acquire(&workspace.root)?;
    let pending = writer.pending()?;
    if !pending.is_empty() {
        if pending.len() == 1
            && pending[0].proposal.as_ref().is_some_and(|context| {
                context.proposal_id == input.proposal_id
                    && context.reviewed_revision == input.expected_revision
            })
        {
            return resume(workspace, store, &writer, &pending[0]);
        }
        return Err(recovery_required());
    }
    if let Some(receipt) = store.proposal_application(&input.proposal_id)? {
        if receipt.context.reviewed_revision != input.expected_revision {
            return Err(conflict(
                "PROPOSAL_REVISION_MISMATCH",
                "The Apply receipt belongs to another reviewed revision.",
            ));
        }
        if receipt.context.workspace_id != workspace.config.workspace.id {
            return Err(workspace_mismatch());
        }
        return Ok(receipt.report);
    }
    let record = store.proposal_get(&input.proposal_id, None)?;
    let state = store.projection_state()?;
    if state.workspace_id != workspace.config.workspace.id {
        return Err(workspace_mismatch());
    }
    let preview = prepare_accepted(
        workspace,
        &record.proposal,
        input.expected_revision,
        state.projection.generation,
        &record.base_snapshot_sha256,
    )?;
    let context = ApplyContext {
        version: 1,
        workspace_id: workspace.config.workspace.id.clone(),
        proposal_id: record.proposal.id.clone(),
        reviewed_revision: record.proposal.revision,
        record_sha256: record_hash(&record)?,
        base_generation: record.proposal.base_generation,
        base_snapshot_sha256: record.base_snapshot_sha256.clone(),
        schema_hash: record.proposal.schema_hash.clone(),
        after_snapshot_sha256: preview.preview().content_sha256().to_owned(),
        files: preview
            .documents()
            .iter()
            .map(|(path, bytes)| ApplyFile {
                path: path.clone(),
                before_sha256: state
                    .files
                    .iter()
                    .find(|file| &file.path == path)
                    .map(|file| file.sha256.clone()),
                after_sha256: sha256(bytes),
            })
            .collect(),
        sources: preview
            .preview()
            .sources()
            .iter()
            .flat_map(|source| {
                source
                    .manifest
                    .revisions
                    .iter()
                    .filter(|revision| preview.verified_source_revisions().contains(&revision.id))
                    .map(|revision| ApplySource {
                        source_id: source.manifest.id.clone(),
                        storage: source.manifest.storage,
                        revision: revision.clone(),
                    })
            })
            .collect(),
        actor: actor.into(),
        requested_at: now.max(record.proposal.updated_at),
    };
    context.validate()?;
    let report = store.apply_proposal(&context, &mut || {
        verify_sources(workspace, &context)?;
        let before = CanonicalSnapshot::scan(workspace)?;
        if before.content_sha256 != context.base_snapshot_sha256 {
            return Err(conflict(
                "STALE_PROPOSAL",
                "Canonical content changed before the file transaction.",
            ));
        }
        if context.files.is_empty() {
            return Ok(CanonicalApplication {
                snapshot: before,
                transaction_id: None,
            });
        }
        let changes = context
            .files
            .iter()
            .map(|file| FileChange {
                path: file.path.clone(),
                before_sha256: file.before_sha256.clone(),
                content: Some(preview.documents()[&file.path].clone()),
            })
            .collect();
        let id = writer.prepare_proposal(changes, context.clone())?;
        writer.apply(&id)?;
        verify_sources(workspace, &context)?;
        Ok(CanonicalApplication {
            snapshot: CanonicalSnapshot::scan_committed(workspace, &id)?,
            transaction_id: Some(id),
        })
    })?;
    if let Some(id) = &report.transaction_id {
        writer.mark_indexed(id)?;
    }
    Ok(report)
}

pub(crate) fn resume(
    workspace: &Workspace,
    store: &mut dyn IndexStore,
    writer: &WorkspaceWriter,
    transaction: &TransactionManifest,
) -> AppResult<ApplyReport> {
    let context = transaction
        .proposal
        .as_ref()
        .ok_or_else(recovery_required)?;
    if context.workspace_id != workspace.config.workspace.id {
        return Err(workspace_mismatch());
    }
    if Schema::load(workspace)?.hash != context.schema_hash {
        return Err(conflict(
            "STALE_PROPOSAL",
            "Schema changed while the Proposal transaction was interrupted.",
        ));
    }
    let report = store.apply_proposal(context, &mut || {
        verify_sources(workspace, context)?;
        writer.apply(&transaction.id)?;
        verify_sources(workspace, context)?;
        Ok(CanonicalApplication {
            snapshot: CanonicalSnapshot::scan_committed(workspace, &transaction.id)?,
            transaction_id: Some(transaction.id.clone()),
        })
    })?;
    // A committed receipt may skip the callback; verify the retained journal before cleanup.
    verify_sources(workspace, context)?;
    writer.apply(&transaction.id)?;
    if CanonicalSnapshot::scan_committed(workspace, &transaction.id)?.content_sha256
        != context.after_snapshot_sha256
    {
        return Err(conflict(
            "PROPOSAL_APPLY_CONTENT_MISMATCH",
            "Recovered canonical content differs from the approved result.",
        ));
    }
    writer.mark_indexed(&transaction.id)?;
    Ok(report)
}

pub fn record_hash(record: &ProposalRecord) -> AppResult<String> {
    record.validate()?;
    Ok(sha256(&serde_json::to_vec(record).map_err(|_| {
        conflict(
            "INVALID_PROPOSAL_RECORD",
            "Could not encode the Proposal record.",
        )
    })?))
}

fn workspace_mismatch() -> AppError {
    AppError::new(
        ErrorType::Configuration,
        "WORKSPACE_ID_MISMATCH",
        "The Proposal index belongs to another workspace.",
    )
}

fn verify_sources(workspace: &Workspace, context: &ApplyContext) -> AppResult<()> {
    if context.sources.is_empty() {
        return Ok(());
    }
    let library = SourceLibrary::new(workspace);
    let sources = library.list(true)?;
    for binding in &context.sources {
        let source = sources
            .iter()
            .find(|source| source.manifest.id == binding.source_id)
            .ok_or_else(|| {
                conflict(
                    "SOURCE_REVISION_CHANGED",
                    "An Apply source binding is missing.",
                )
            })?;
        if source.manifest.storage != binding.storage
            || !source.manifest.revisions.contains(&binding.revision)
        {
            return Err(conflict(
                "SOURCE_REVISION_CHANGED",
                "An immutable Apply source binding changed.",
            ));
        }
        library.content_at(&source.path, &source.manifest, &binding.revision.id)?;
    }
    Ok(())
}
fn conflict(code: &str, message: &str) -> AppError {
    AppError::new(ErrorType::Conflict, code, message)
}
