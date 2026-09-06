use knowmesh_core::{
    application::proposal::{
        ProposalRecord,
        apply::{ApplyContext, ApplyReceipt, ApplyReport, CanonicalApplication, record_hash},
    },
    domain::{
        ProposalId,
        proposal::{ProposalState, ReviewPolicy},
        sha256,
    },
    error::AppResult,
};
use rusqlite::{Connection, TransactionBehavior, params};

use super::{append, conflict, current_baseline, json_text, load, revision_mismatch};
use crate::{SqliteStore, database_error, reconcile::reconcile_in_transaction};

pub(crate) fn receipt(db: &Connection, id: &ProposalId) -> AppResult<Option<ApplyReceipt>> {
    let mut statement = db.prepare("SELECT receipt_json,content_sha256,reviewed_revision FROM proposal_applications WHERE proposal_id=?1").map_err(database_error)?;
    let mut rows = statement.query([id.as_str()]).map_err(database_error)?;
    let Some(row) = rows.next().map_err(database_error)? else {
        return Ok(None);
    };
    let bytes = row
        .get_ref(0)
        .map_err(database_error)?
        .as_str()
        .map_err(|_| invalid())?
        .as_bytes();
    if bytes.len() > 8 * 1024 * 1024
        || sha256(bytes) != row.get::<_, String>(1).map_err(database_error)?
    {
        return Err(invalid());
    }
    let receipt: ApplyReceipt = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    receipt.validate()?;
    let current = load(db, id, None)?;
    let reviewed = load(db, id, Some(receipt.context.reviewed_revision))?;
    if receipt.context.proposal_id != *id
        || receipt.context.reviewed_revision != row.get::<_, u32>(2).map_err(database_error)?
        || current.proposal.state != ProposalState::Applied
        || Some(current.proposal.revision) != receipt.report.applied_revision
        || current.proposal.applied_generation
            != receipt
                .report
                .projection
                .as_ref()
                .map(|projection| projection.generation)
        || record_hash(&reviewed)? != receipt.context.record_sha256
    {
        return Err(invalid());
    }
    Ok(Some(receipt))
}

pub(crate) fn commit(
    store: &mut SqliteStore,
    context: &ApplyContext,
    canonical: &mut dyn FnMut() -> AppResult<CanonicalApplication>,
) -> AppResult<ApplyReport> {
    context.validate()?;
    let tx = store
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let workspace_id: String = tx
        .query_row(
            "SELECT workspace_id FROM workspace_state WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(database_error)?;
    if workspace_id != context.workspace_id.as_str() {
        return Err(conflict(
            "WORKSPACE_ID_MISMATCH",
            "The Apply context belongs to another workspace.",
        ));
    }
    if let Some(receipt) = receipt(&tx, &context.proposal_id)? {
        if receipt.context != *context {
            return Err(conflict(
                "PROPOSAL_APPLY_CONTEXT_MISMATCH",
                "The journal differs from the committed Apply context.",
            ));
        }
        return Ok(receipt.report);
    }
    let reviewed = load(&tx, &context.proposal_id, None)?;
    if reviewed.proposal.revision != context.reviewed_revision {
        return Err(revision_mismatch());
    }
    reviewed
        .proposal
        .require_approved(&ReviewPolicy::default())?;
    if record_hash(&reviewed)? != context.record_sha256
        || reviewed.base_snapshot_sha256 != context.base_snapshot_sha256
        || reviewed.proposal.base_generation != context.base_generation
        || reviewed.proposal.schema_hash != context.schema_hash
        || context.requested_at < reviewed.proposal.updated_at
    {
        return Err(conflict(
            "PROPOSAL_APPLY_CONTEXT_MISMATCH",
            "The Apply context differs from the reviewed Proposal record.",
        ));
    }
    current_baseline(&tx, &reviewed)?;
    // Keep the revision comparison and all database updates in one write transaction.
    let committed = canonical()?;
    committed.snapshot.validate()?;
    if committed.snapshot.workspace_id != context.workspace_id
        || committed.snapshot.proposal_apply_context()
            != (!context.files.is_empty()).then_some(context)
        || committed.snapshot.schema_hash != context.schema_hash
        || committed.snapshot.content_sha256 != context.after_snapshot_sha256
        || context.files.iter().any(|file| {
            !committed
                .snapshot
                .files
                .iter()
                .any(|actual| actual.path == file.path && actual.sha256 == file.after_sha256)
        })
    {
        return Err(conflict(
            "PROPOSAL_APPLY_CONTENT_MISMATCH",
            "The canonical result differs from the approved preview.",
        ));
    }
    let projection = reconcile_in_transaction(&tx, &committed.snapshot)?;
    let applied = ProposalRecord {
        proposal: reviewed.proposal.mark_applied(
            context.reviewed_revision,
            projection.generation,
            &context.actor,
            context.requested_at,
        )?,
        base_snapshot_sha256: reviewed.base_snapshot_sha256,
    };
    append(&tx, &applied)?;
    let report = ApplyReport {
        dry_run: false,
        proposal_id: context.proposal_id.clone(),
        reviewed_revision: context.reviewed_revision,
        applied_revision: Some(applied.proposal.revision),
        projection: Some(projection),
        changed_paths: context.files.iter().map(|file| file.path.clone()).collect(),
        transaction_id: committed.transaction_id,
    };
    let receipt = ApplyReceipt {
        context: context.clone(),
        report: report.clone(),
    };
    receipt.validate()?;
    let encoded = json_text(&receipt)?;
    if encoded.len() > 8 * 1024 * 1024 {
        return Err(invalid());
    }
    tx.execute("INSERT INTO proposal_applications(proposal_id,reviewed_revision,receipt_json,content_sha256) VALUES(?1,?2,?3,?4)", params![context.proposal_id.as_str(),context.reviewed_revision,encoded,sha256(encoded.as_bytes())]).map_err(database_error)?;
    tx.commit().map_err(database_error)?;
    Ok(report)
}

fn invalid() -> knowmesh_core::error::AppError {
    conflict(
        "PROPOSAL_RECEIPT_INVALID",
        "The stored Apply receipt is corrupt or disagrees with its Proposal history.",
    )
}
