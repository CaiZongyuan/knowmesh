use knowmesh_core::{
    application::proposal::{
        apply::ApplyReport,
        idempotency::{IdempotencyRequest, MutationResult, ProposalMutation},
    },
    domain::{ProposalId, Timestamp},
    error::{AppError, AppResult},
};
use rusqlite::{Connection, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use super::{conflict, create_in_transaction, load, save_in_transaction};
use crate::{SqliteStore, database_error};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultReference {
    version: u32,
    proposal_id: ProposalId,
    revision: u32,
    error: Option<AppError>,
}

pub(super) fn read(
    db: &Connection,
    request: &IdempotencyRequest,
) -> AppResult<Option<MutationResult>> {
    if request.operation == "proposal.apply" {
        return Err(invalid());
    }
    let Some(reference) = read_reference(db, request)? else {
        return Ok(None);
    };
    let record =
        load(db, &reference.proposal_id, Some(reference.revision)).map_err(|_| invalid())?;
    Ok(Some(MutationResult {
        record,
        error: reference.error,
    }))
}

fn read_reference(
    db: &Connection,
    request: &IdempotencyRequest,
) -> AppResult<Option<ResultReference>> {
    request.validate()?;
    let mut statement = db.prepare("SELECT input_hash,state,response_json,status_code FROM idempotency_keys WHERE key=?1 AND operation=?2").map_err(database_error)?;
    let mut rows = statement
        .query(params![request.key, request.operation])
        .map_err(database_error)?;
    let Some(row) = rows.next().map_err(database_error)? else {
        return Ok(None);
    };
    if row.get::<_, String>(0).map_err(database_error)? != request.input_sha256 {
        return Err(conflict(
            "IDEMPOTENCY_KEY_REUSED",
            "The key is already bound to another input for this operation.",
        ));
    }
    if row.get::<_, String>(1).map_err(database_error)? != "completed" {
        return Err(conflict(
            "IDEMPOTENCY_IN_PROGRESS",
            "The idempotent operation has not completed.",
        )
        .retryable(true));
    }
    let text = row
        .get_ref(2)
        .map_err(database_error)?
        .as_str()
        .map_err(|_| invalid())?;
    if text.len() > 64 * 1024 {
        return Err(invalid());
    }
    let reference: ResultReference = serde_json::from_str(text).map_err(|_| invalid())?;
    if reference.version != 1
        || row.get::<_, u16>(3).map_err(database_error)?
            != reference.error.as_ref().map_or(200, AppError::http_status)
    {
        return Err(invalid());
    }
    Ok(Some(reference))
}

pub(super) fn read_apply(
    db: &Connection,
    request: &IdempotencyRequest,
) -> AppResult<Option<ApplyReport>> {
    if request.operation != "proposal.apply" {
        return Err(invalid());
    }
    let Some(reference) = read_reference(db, request)? else {
        return Ok(None);
    };
    let receipt = super::apply::receipt(db, &reference.proposal_id)?.ok_or_else(invalid)?;
    if reference.error.is_some() || receipt.report.applied_revision != Some(reference.revision) {
        return Err(invalid());
    }
    Ok(Some(receipt.report))
}

pub(super) fn write_apply(
    db: &Connection,
    request: &IdempotencyRequest,
    report: &ApplyReport,
    at: Timestamp,
) -> AppResult<()> {
    request.validate()?;
    if let Some(previous) = read_apply(db, request)? {
        if serde_json::to_value(previous).map_err(|_| invalid())?
            != serde_json::to_value(report).map_err(|_| invalid())?
        {
            return Err(invalid());
        }
        return Ok(());
    }
    let reference = ResultReference {
        version: 1,
        proposal_id: report.proposal_id.clone(),
        revision: report.applied_revision.ok_or_else(invalid)?,
        error: None,
    };
    let response = serde_json::to_string(&reference).map_err(|_| invalid())?;
    db.execute("INSERT INTO idempotency_keys(key,operation,input_hash,state,response_json,status_code,created_at) VALUES(?1,?2,?3,'completed',?4,200,?5)",params![request.key,request.operation,request.input_sha256,response,at.to_string()]).map_err(database_error)?;
    Ok(())
}

pub(super) fn commit(
    store: &mut SqliteStore,
    mutation: &ProposalMutation,
) -> AppResult<MutationResult> {
    mutation.record.validate()?;
    if let Some(request) = &mutation.idempotency {
        request.validate()?;
        if request.operation == "proposal.apply" {
            return Err(invalid());
        }
        if (request.operation == "proposal.create") != mutation.expected_revision.is_none() {
            return Err(invalid());
        }
    }
    let tx = store
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    if let Some(request) = &mutation.idempotency
        && let Some(cached) = read(&tx, request)?
    {
        return Ok(cached);
    }
    match mutation.expected_revision {
        None => create_in_transaction(&tx, &mutation.record)?,
        Some(expected) => save_in_transaction(&tx, expected, &mutation.record)?,
    }
    if let Some(request) = &mutation.idempotency {
        let reference = ResultReference {
            version: 1,
            proposal_id: mutation.record.proposal.id.clone(),
            revision: mutation.record.proposal.revision,
            error: mutation.error.clone(),
        };
        let response = serde_json::to_string(&reference).map_err(|_| invalid())?;
        if response.len() > 64 * 1024 {
            return Err(invalid());
        }
        tx.execute("INSERT INTO idempotency_keys(key,operation,input_hash,state,response_json,status_code,created_at) VALUES(?1,?2,?3,'completed',?4,?5,?6)",
            params![request.key,request.operation,request.input_sha256,response,mutation.error.as_ref().map_or(200, AppError::http_status),mutation.record.proposal.updated_at.to_string()],
        ).map_err(database_error)?;
    }
    tx.commit().map_err(database_error)?;
    Ok(MutationResult {
        record: mutation.record.clone(),
        error: mutation.error.clone(),
    })
}

fn invalid() -> AppError {
    conflict(
        "IDEMPOTENCY_RESULT_INVALID",
        "The idempotency result is malformed or its immutable Proposal revision is unavailable.",
    )
}
