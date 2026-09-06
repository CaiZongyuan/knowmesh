use knowmesh_core::{
    application::proposal::{MAX_PROPOSAL_RECORD_BYTES, ProposalRecord},
    domain::{
        ProposalId,
        proposal::{Decision, ProposalState},
        sha256,
    },
    error::{AppError, AppResult, ErrorType},
    ports::ProposalStore,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;

use crate::{SqliteStore, database_error};

impl ProposalStore for SqliteStore {
    fn proposal_create(&mut self, record: &ProposalRecord) -> AppResult<()> {
        record.validate()?;
        let proposal = &record.proposal;
        if proposal.revision != 1 || proposal.state != ProposalState::Draft {
            return Err(conflict(
                "PROPOSAL_INITIAL_REVISION_REQUIRED",
                "New Proposals must begin with an unreviewed first revision.",
            ));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM proposals WHERE id=?1)",
                [proposal.id.as_str()],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        if exists {
            return Err(conflict(
                "PROPOSAL_ALREADY_EXISTS",
                "The Proposal ID already exists.",
            ));
        }
        current_baseline(&tx, record)?;
        append(&tx, record)?;
        tx.commit().map_err(database_error)
    }

    fn proposal_get(&self, id: &ProposalId, revision: Option<u32>) -> AppResult<ProposalRecord> {
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(database_error)?;
        load(&tx, id, revision)
    }

    fn proposal_save(&mut self, expected_revision: u32, record: &ProposalRecord) -> AppResult<()> {
        record.validate()?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let previous = load(&tx, &record.proposal.id, None)?;
        if previous.proposal.revision != expected_revision {
            return Err(revision_mismatch());
        }
        if &previous == record {
            return Ok(());
        }
        if expected_revision.checked_add(1) != Some(record.proposal.revision) {
            return Err(revision_mismatch());
        }
        let old = &previous.proposal;
        let next = &record.proposal;
        if next.state == ProposalState::Applied {
            return Err(conflict(
                "PROPOSAL_APPLY_COORDINATOR_REQUIRED",
                "Only a coordinated canonical Apply may finalize a Proposal.",
            ));
        }
        if matches!(old.state, ProposalState::Applied | ProposalState::Rejected) {
            return Err(conflict(
                "PROPOSAL_FINALIZED",
                "A finalized Proposal cannot be changed.",
            ));
        }
        if old.state == ProposalState::Stale
            && !matches!(next.state, ProposalState::Stale | ProposalState::Rejected)
            && next
                .items
                .iter()
                .any(|item| item.decision != Decision::Pending)
        {
            return Err(conflict(
                "PROPOSAL_REVALIDATION_REQUIRED",
                "A stale Proposal must be revalidated into an unreviewed revision before approval.",
            ));
        }
        if old.kind != next.kind
            || old.created_at != next.created_at
            || old.created_by != next.created_by
            || old.source_revision_id != next.source_revision_id
            || old.compiler_run_id != next.compiler_run_id
            || next.updated_at < old.updated_at
            || (previous.base_snapshot_sha256 != record.base_snapshot_sha256
                && next
                    .items
                    .iter()
                    .any(|item| item.decision != Decision::Pending))
        {
            return Err(conflict(
                "PROPOSAL_HISTORY_INVALID",
                "A revision cannot rewrite Proposal identity or reuse reviews after changing its baseline.",
            ));
        }
        if !matches!(next.state, ProposalState::Stale | ProposalState::Rejected) {
            current_baseline(&tx, record)?;
        }
        append(&tx, record)?;
        tx.commit().map_err(database_error)
    }
}

fn current_baseline(db: &Connection, record: &ProposalRecord) -> AppResult<()> {
    let (generation, hash, schema, complete): (u64, String, String, bool) = db.query_row(
        "SELECT indexed_generation,snapshot_sha256,schema_hash,snapshot_warnings_json IS NOT NULL FROM workspace_state WHERE singleton=1", [],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).map_err(database_error)?;
    if !complete
        || generation != record.proposal.base_generation
        || hash != record.base_snapshot_sha256
        || schema != record.proposal.schema_hash
    {
        return Err(conflict(
            "STALE_PROPOSAL",
            "The Proposal baseline does not match the complete indexed snapshot.",
        ));
    }
    if let Some(id) = &record.proposal.source_revision_id {
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM source_revisions WHERE id=?1)",
                [id.as_str()],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        if !exists {
            return Err(conflict(
                "SOURCE_REVISION_NOT_FOUND",
                "The Proposal source revision is absent from the index.",
            ));
        }
    }
    Ok(())
}

fn load(db: &Connection, id: &ProposalId, revision: Option<u32>) -> AppResult<ProposalRecord> {
    let header: Option<(u32, String, u64, String)> = db
        .query_row(
            "SELECT revision,state,base_generation,schema_hash FROM proposals WHERE id=?1",
            [id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(database_error)?;
    let (current, state, generation, schema) = header.ok_or_else(|| {
        AppError::new(
            ErrorType::NotFound,
            "PROPOSAL_NOT_FOUND",
            "The Proposal is absent.",
        )
    })?;
    let revision = revision.unwrap_or(current);
    let mut statement = db.prepare("SELECT snapshot_json,content_sha256 FROM proposal_revisions WHERE proposal_id=?1 AND revision=?2").map_err(database_error)?;
    let mut rows = statement
        .query(params![id.as_str(), revision])
        .map_err(database_error)?;
    let row = rows.next().map_err(database_error)?.ok_or_else(|| {
        if revision == current {
            conflict("PROPOSAL_HISTORY_UNAVAILABLE", "The current Proposal has no complete revision snapshot. Its legacy rows have been preserved.")
        } else {
            AppError::new(ErrorType::NotFound, "PROPOSAL_REVISION_NOT_FOUND", "The requested Proposal revision is absent.")
        }
    })?;
    let bytes = row
        .get_ref(0)
        .map_err(database_error)?
        .as_str()
        .map_err(|_| corrupt())?
        .as_bytes();
    if bytes.len() > MAX_PROPOSAL_RECORD_BYTES
        || sha256(bytes) != row.get::<_, String>(1).map_err(database_error)?
    {
        return Err(corrupt());
    }
    let record: ProposalRecord = serde_json::from_slice(bytes).map_err(|_| corrupt())?;
    record.validate().map_err(|_| corrupt())?;
    if record.proposal.id != *id
        || record.proposal.revision != revision
        || revision > current
        || (revision == current
            && (enum_text(&record.proposal.state)? != state
                || record.proposal.base_generation != generation
                || record.proposal.schema_hash != schema))
    {
        return Err(corrupt());
    }
    Ok(record)
}

fn append(db: &Connection, record: &ProposalRecord) -> AppResult<()> {
    let p = &record.proposal;
    let snapshot = json_text(record)?;
    let digest = sha256(snapshot.as_bytes());
    let warnings: Vec<_> = p
        .items
        .iter()
        .filter(|item| !item.issues.is_empty())
        .map(|item| serde_json::json!({"item_id":item.id,"warnings":item.issues}))
        .collect();
    db.execute("INSERT INTO proposals(id,kind,state,revision,base_generation,source_revision_id,schema_hash,summary_json,warnings_json,compiler_run_id,created_by,created_at,updated_at,applied_at,applied_generation)
        VALUES(?1,?2,?3,?4,?5,(SELECT id FROM source_revisions WHERE id=?6),?7,?8,?9,?10,?11,?12,?13,?14,?15)
        ON CONFLICT(id) DO UPDATE SET state=excluded.state,revision=excluded.revision,base_generation=excluded.base_generation,source_revision_id=excluded.source_revision_id,schema_hash=excluded.schema_hash,summary_json=excluded.summary_json,warnings_json=excluded.warnings_json,updated_at=excluded.updated_at,applied_at=excluded.applied_at,applied_generation=excluded.applied_generation",
        params![p.id.as_str(),enum_text(&p.kind)?,enum_text(&p.state)?,p.revision,p.base_generation,p.source_revision_id.as_ref().map(|id| id.as_str()),p.schema_hash,json_text(&p.summary)?,json_text(&warnings)?,p.compiler_run_id.as_ref().map(|id| id.as_str()),p.created_by,p.created_at.to_string(),p.updated_at.to_string(),p.applied_at.map(|at| at.to_string()),p.applied_generation],
    ).map_err(database_error)?;
    db.execute("INSERT INTO proposal_revisions(proposal_id,revision,snapshot_json,content_sha256,created_at) VALUES(?1,?2,?3,?4,?5)", params![p.id.as_str(),p.revision,snapshot,digest,p.updated_at.to_string()]).map_err(database_error)?;
    db.execute(
        "DELETE FROM proposal_items WHERE proposal_id=?1",
        [p.id.as_str()],
    )
    .map_err(database_error)?;
    for (ordinal, item) in p.items.iter().enumerate() {
        db.execute("INSERT INTO proposal_items(id,proposal_id,ordinal,op,target_id,payload_json,before_sha256,evidence_ids_json,compiler_confidence,risk,decision,decision_reason,warnings_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![item.id.as_str(),p.id.as_str(),ordinal,enum_text(&item.op)?,item.target_id,json_text(&item.payload)?,item.before_sha256,json_text(&item.evidence_ids)?,item.compiler_confidence,enum_text(&item.risk)?,enum_text(&item.decision)?,item.decision_reason,json_text(&item.issues)?],
        ).map_err(database_error)?;
    }
    db.execute("INSERT INTO audit_events(event_id,event_type,actor,object_type,object_id,payload_json,created_at) VALUES(?1,'proposal.revision_created',?2,'proposal',?3,?4,?5)",
        params![format!("proposal:{}:{}",p.id,p.revision),p.updated_by,p.id.as_str(),json_text(&serde_json::json!({"revision":p.revision,"state":p.state,"content_sha256":digest}))?,p.updated_at.to_string()],
    ).map_err(database_error)?;
    Ok(())
}

fn json_text(value: &impl Serialize) -> AppResult<String> {
    serde_json::to_string(value).map_err(|_| corrupt())
}
fn enum_text(value: &impl Serialize) -> AppResult<String> {
    serde_json::to_value(value)
        .map_err(|_| corrupt())?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(corrupt)
}
fn conflict(code: &str, message: &str) -> AppError {
    AppError::new(ErrorType::Conflict, code, message)
}
fn revision_mismatch() -> AppError {
    conflict(
        "PROPOSAL_REVISION_MISMATCH",
        "The current revision changed or the next revision does not immediately follow it.",
    )
}
fn corrupt() -> AppError {
    conflict(
        "PROPOSAL_HISTORY_INVALID",
        "The stored Proposal revision is malformed or inconsistent with its hash/header.",
    )
}
