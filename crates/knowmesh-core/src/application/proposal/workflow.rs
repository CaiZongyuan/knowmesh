use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::idempotency::{IdempotencyRequest, MutationResult, ProposalMutation};
use super::{ProposalRecord, prepare_snapshot};
use crate::{
    canonical::{
        schema::{ReviewMode, Schema},
        snapshot::CanonicalSnapshot,
        transaction::{WorkspaceWriter, pending, recovery_required},
        workspace::Workspace,
    },
    domain::{
        ProposalId, Timestamp,
        proposal::{ProposalInput, ProposalRevision, ProposalState, ReviewInput, ReviewPolicy},
    },
    error::{AppError, AppResult, ErrorType},
    ports::{ProjectionState, ProposalStore},
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInput {
    pub proposal: ProposalInput,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetInput {
    pub proposal_id: ProposalId,
    #[serde(default)]
    pub revision: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewRequest {
    pub proposal_id: ProposalId,
    pub review: ReviewInput,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditInput {
    pub proposal_id: ProposalId,
    pub revision: ProposalRevision,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevalidateInput {
    pub proposal_id: ProposalId,
    pub expected_revision: u32,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RejectInput {
    pub proposal_id: ProposalId,
    pub expected_revision: u32,
    pub reason: String,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MutationReport {
    pub dry_run: bool,
    pub record: ProposalRecord,
}

#[derive(Debug, Clone, Copy)]
pub enum MutationRequest<'a> {
    Create(&'a CreateInput),
    Edit(&'a EditInput),
    Review(&'a ReviewRequest),
    Revalidate(&'a RevalidateInput),
    Reject(&'a RejectInput),
}

pub fn execute_request(
    workspace: &Workspace,
    store: &mut dyn ProposalStore,
    request: MutationRequest<'_>,
    key: Option<&str>,
    actor: &str,
    now: Timestamp,
) -> AppResult<MutationReport> {
    // JSON would encode nonfinite scalar confidence as null; validate before fingerprinting.
    if key.is_some() {
        let items = match request {
            MutationRequest::Create(input) => input.proposal.items.as_slice(),
            MutationRequest::Edit(input) => input.revision.items.as_slice(),
            _ => &[],
        };
        for item in items {
            item.validate_content()?;
        }
    }
    let (key, dry_run) = match request {
        MutationRequest::Create(input) => (
            key.map(|key| IdempotencyRequest::new(key, "proposal.create", input))
                .transpose()?,
            input.dry_run,
        ),
        MutationRequest::Edit(input) => (
            key.map(|key| IdempotencyRequest::new(key, "proposal.edit", input))
                .transpose()?,
            input.dry_run,
        ),
        MutationRequest::Review(input) => (
            key.map(|key| IdempotencyRequest::new(key, "proposal.review", input))
                .transpose()?,
            input.dry_run,
        ),
        MutationRequest::Revalidate(input) => (
            key.map(|key| IdempotencyRequest::new(key, "proposal.revalidate", input))
                .transpose()?,
            input.dry_run,
        ),
        MutationRequest::Reject(input) => (
            key.map(|key| IdempotencyRequest::new(key, "proposal.reject", input))
                .transpose()?,
            input.dry_run,
        ),
    };
    if let Some(key) = &key {
        check_workspace(workspace, store)?;
        if let Some(cached) = store.proposal_cached_mutation(key)? {
            return cached.report(dry_run);
        }
    }
    match request {
        MutationRequest::Create(input) => {
            create_inner(workspace, store, input, key.as_ref(), actor, now)
        }
        MutationRequest::Edit(input) => {
            edit_inner(workspace, store, input, key.as_ref(), actor, now)
        }
        MutationRequest::Review(input) => {
            review_inner(workspace, store, input, key.as_ref(), actor, now)
        }
        MutationRequest::Revalidate(input) => {
            revalidate_inner(workspace, store, input, key.as_ref(), actor, now)
        }
        MutationRequest::Reject(input) => {
            reject_inner(workspace, store, input, key.as_ref(), actor, now)
        }
    }
}

pub fn get(
    workspace: &Workspace,
    store: &dyn ProposalStore,
    input: &GetInput,
) -> AppResult<ProposalRecord> {
    check_workspace(workspace, store)?;
    store.proposal_get(&input.proposal_id, input.revision)
}

pub fn create(
    workspace: &Workspace,
    store: &mut dyn ProposalStore,
    input: &CreateInput,
    actor: &str,
    now: Timestamp,
) -> AppResult<MutationReport> {
    execute_request(
        workspace,
        store,
        MutationRequest::Create(input),
        None,
        actor,
        now,
    )
}

fn create_inner(
    workspace: &Workspace,
    store: &mut dyn ProposalStore,
    input: &CreateInput,
    key: Option<&IdempotencyRequest>,
    actor: &str,
    now: Timestamp,
) -> AppResult<MutationReport> {
    let _writer = guard(workspace, store, input.dry_run)?;
    let before = CanonicalSnapshot::scan(workspace)?;
    let state = store.projection_state()?;
    current_index(&before, &state)?;
    if input.proposal.base_generation != state.projection.generation {
        return Err(stale());
    }
    let prepared = prepare_snapshot(workspace, &input.proposal, actor, now, before)?;
    let record = ProposalRecord {
        proposal: prepared.proposal,
        base_snapshot_sha256: prepared.base_snapshot_sha256,
    };
    save(store, None, record, input.dry_run, key, None)
}

pub fn review(
    workspace: &Workspace,
    store: &mut dyn ProposalStore,
    input: &ReviewRequest,
    actor: &str,
    now: Timestamp,
) -> AppResult<MutationReport> {
    execute_request(
        workspace,
        store,
        MutationRequest::Review(input),
        None,
        actor,
        now,
    )
}

fn review_inner(
    workspace: &Workspace,
    store: &mut dyn ProposalStore,
    input: &ReviewRequest,
    key: Option<&IdempotencyRequest>,
    actor: &str,
    now: Timestamp,
) -> AppResult<MutationReport> {
    let _writer = guard(workspace, store, input.dry_run)?;
    let record = editable(store, &input.proposal_id, input.review.expected_revision)?;
    let before = CanonicalSnapshot::scan(workspace)?;
    let state = store.projection_state()?;
    if !record_current(&record, &before, &state) {
        if !input.dry_run {
            let next = ProposalRecord {
                proposal: record.proposal.mark_stale(
                    input.review.expected_revision,
                    "Canonical baseline, Schema, or index generation changed.",
                    actor,
                    now,
                )?,
                ..record
            };
            return save(
                store,
                Some(input.review.expected_revision),
                next,
                false,
                key,
                Some(stale()),
            );
        }
        return Err(stale());
    }
    let schema = Schema::load(workspace)?;
    if schema.hash != before.schema_hash {
        return Err(stale());
    }
    let prepared = prepare_snapshot(workspace, &proposal_input(&record), actor, now, before)?;
    for (stored, checked) in record.proposal.items.iter().zip(&prepared.proposal.items) {
        if stored.content_sha256()? != checked.content_sha256()? {
            return Err(conflict(
                "PROPOSAL_REVALIDATION_REQUIRED",
                "Validation changed item content. Revalidate the Proposal before reviewing it.",
            ));
        }
    }
    let next = ProposalRecord {
        proposal: record.proposal.review(
            &input.review,
            &ReviewPolicy {
                strict: schema.policies.review_mode == ReviewMode::Strict,
                allow_accept_all: schema.policies.allow_accept_all,
                human_verification_required: schema.policies.human_verification_required,
            },
            actor,
            now,
        )?,
        ..record
    };
    save(
        store,
        Some(input.review.expected_revision),
        next,
        input.dry_run,
        key,
        None,
    )
}

pub fn edit(
    workspace: &Workspace,
    store: &mut dyn ProposalStore,
    input: &EditInput,
    actor: &str,
    now: Timestamp,
) -> AppResult<MutationReport> {
    execute_request(
        workspace,
        store,
        MutationRequest::Edit(input),
        None,
        actor,
        now,
    )
}

fn edit_inner(
    workspace: &Workspace,
    store: &mut dyn ProposalStore,
    input: &EditInput,
    key: Option<&IdempotencyRequest>,
    actor: &str,
    now: Timestamp,
) -> AppResult<MutationReport> {
    let _writer = guard(workspace, store, input.dry_run)?;
    let record = editable(store, &input.proposal_id, input.revision.expected_revision)?;
    let before = CanonicalSnapshot::scan(workspace)?;
    let state = store.projection_state()?;
    current_index(&before, &state)?;
    if input.revision.base_generation != state.projection.generation
        || input.revision.schema_hash != before.schema_hash
    {
        return Err(stale());
    }
    let mut candidate = proposal_input(&record);
    candidate.base_generation = input.revision.base_generation;
    candidate.schema_hash = input.revision.schema_hash.clone();
    candidate.summary = input.revision.summary.clone();
    candidate.items = input.revision.items.clone();
    let prepared = prepare_snapshot(workspace, &candidate, actor, now, before)?;
    let next = revised(record, prepared, actor, now)?;
    save(
        store,
        Some(input.revision.expected_revision),
        next,
        input.dry_run,
        key,
        None,
    )
}

pub fn revalidate(
    workspace: &Workspace,
    store: &mut dyn ProposalStore,
    input: &RevalidateInput,
    actor: &str,
    now: Timestamp,
) -> AppResult<MutationReport> {
    execute_request(
        workspace,
        store,
        MutationRequest::Revalidate(input),
        None,
        actor,
        now,
    )
}

fn revalidate_inner(
    workspace: &Workspace,
    store: &mut dyn ProposalStore,
    input: &RevalidateInput,
    key: Option<&IdempotencyRequest>,
    actor: &str,
    now: Timestamp,
) -> AppResult<MutationReport> {
    let _writer = guard(workspace, store, input.dry_run)?;
    let record = editable(store, &input.proposal_id, input.expected_revision)?;
    let before = CanonicalSnapshot::scan(workspace)?;
    let state = store.projection_state()?;
    let generation = if input.dry_run {
        state
            .projection
            .generation
            .checked_add(u64::from(state.snapshot_sha256 != before.content_sha256))
            .ok_or_else(stale)?
    } else {
        store.reconcile(&before)?.generation
    };
    let mut candidate = proposal_input(&record);
    candidate.base_generation = generation;
    candidate.schema_hash = before.schema_hash.clone();
    for item in &mut candidate.items {
        item.before_sha256 = None;
    }
    let prepared = prepare_snapshot(workspace, &candidate, actor, now, before)?;
    let next = revised(record, prepared, actor, now)?;
    save(
        store,
        Some(input.expected_revision),
        next,
        input.dry_run,
        key,
        None,
    )
}

pub fn reject(
    workspace: &Workspace,
    store: &mut dyn ProposalStore,
    input: &RejectInput,
    actor: &str,
    now: Timestamp,
) -> AppResult<MutationReport> {
    execute_request(
        workspace,
        store,
        MutationRequest::Reject(input),
        None,
        actor,
        now,
    )
}

fn reject_inner(
    workspace: &Workspace,
    store: &mut dyn ProposalStore,
    input: &RejectInput,
    key: Option<&IdempotencyRequest>,
    actor: &str,
    now: Timestamp,
) -> AppResult<MutationReport> {
    let _writer = guard(workspace, store, input.dry_run)?;
    let record = store.proposal_get(&input.proposal_id, None)?;
    if record.proposal.revision != input.expected_revision {
        return Err(revision_mismatch());
    }
    let next = ProposalRecord {
        proposal: record
            .proposal
            .reject(input.expected_revision, &input.reason, actor, now)?,
        ..record
    };
    save(
        store,
        Some(input.expected_revision),
        next,
        input.dry_run,
        key,
        None,
    )
}

fn revised(
    record: ProposalRecord,
    prepared: super::PreparedProposal,
    actor: &str,
    now: Timestamp,
) -> AppResult<ProposalRecord> {
    let proposal = record.proposal.revise(
        &ProposalRevision {
            expected_revision: record.proposal.revision,
            base_generation: prepared.proposal.base_generation,
            schema_hash: prepared.proposal.schema_hash,
            summary: prepared.proposal.summary,
            items: prepared.proposal.items,
        },
        actor,
        now,
    )?;
    Ok(ProposalRecord {
        proposal,
        base_snapshot_sha256: prepared.base_snapshot_sha256,
    })
}

fn proposal_input(record: &ProposalRecord) -> ProposalInput {
    let p = &record.proposal;
    ProposalInput {
        kind: p.kind,
        base_generation: p.base_generation,
        schema_hash: p.schema_hash.clone(),
        source_revision_id: p.source_revision_id.clone(),
        compiler_run_id: p.compiler_run_id.clone(),
        summary: p.summary.clone(),
        items: p.items.clone(),
    }
}

fn save(
    store: &mut dyn ProposalStore,
    expected: Option<u32>,
    record: ProposalRecord,
    dry_run: bool,
    key: Option<&IdempotencyRequest>,
    error: Option<AppError>,
) -> AppResult<MutationReport> {
    record.validate()?;
    if dry_run {
        return MutationResult { record, error }.report(true);
    }
    store
        .proposal_commit_mutation(&ProposalMutation {
            record,
            expected_revision: expected,
            idempotency: key.cloned(),
            error,
        })?
        .report(false)
}

fn editable(
    store: &dyn ProposalStore,
    id: &ProposalId,
    expected: u32,
) -> AppResult<ProposalRecord> {
    let record = store.proposal_get(id, None)?;
    if record.proposal.revision != expected {
        return Err(revision_mismatch());
    }
    if matches!(
        record.proposal.state,
        ProposalState::Applied | ProposalState::Rejected
    ) {
        return Err(conflict(
            "PROPOSAL_FINALIZED",
            "A finalized Proposal cannot be edited or reviewed.",
        ));
    }
    Ok(record)
}

fn guard(
    workspace: &Workspace,
    store: &dyn ProposalStore,
    dry_run: bool,
) -> AppResult<Option<WorkspaceWriter>> {
    check_workspace(workspace, store)?;
    let writer = (!dry_run)
        .then(|| WorkspaceWriter::acquire(&workspace.root))
        .transpose()?;
    if !pending(&workspace.root)?.is_empty() {
        return Err(recovery_required());
    }
    Ok(writer)
}

fn check_workspace(workspace: &Workspace, store: &dyn ProposalStore) -> AppResult<()> {
    let actual = Workspace::load(&workspace.root)?;
    if actual.config.workspace.id != workspace.config.workspace.id
        || store.projection_state()?.workspace_id != workspace.config.workspace.id
    {
        return Err(AppError::new(
            ErrorType::Configuration,
            "WORKSPACE_ID_MISMATCH",
            "The Proposal store belongs to another workspace.",
        ));
    }
    Ok(())
}

fn record_current(
    record: &ProposalRecord,
    before: &CanonicalSnapshot,
    state: &ProjectionState,
) -> bool {
    current_index(before, state).is_ok()
        && record.proposal.base_generation == state.projection.generation
        && record.base_snapshot_sha256 == before.content_sha256
        && record.proposal.schema_hash == before.schema_hash
}

fn current_index(before: &CanonicalSnapshot, state: &ProjectionState) -> AppResult<()> {
    if state.warnings.is_none()
        || state.snapshot_sha256 != before.content_sha256
        || state.schema_hash != before.schema_hash
        || state.workspace_id != before.workspace_id
    {
        return Err(stale());
    }
    Ok(())
}

fn stale() -> AppError {
    conflict(
        "STALE_PROPOSAL",
        "Synchronize or revalidate the Proposal against the current canonical baseline before continuing.",
    )
}
fn revision_mismatch() -> AppError {
    conflict(
        "PROPOSAL_REVISION_MISMATCH",
        "Fetch the current Proposal revision before changing it.",
    )
}
fn conflict(code: &str, message: &str) -> AppError {
    AppError::new(ErrorType::Conflict, code, message)
}

pub fn descriptors() -> Vec<crate::application::operations::OperationDescriptor> {
    use crate::application::operations::{EffectLevel, OperationDescriptor};
    let mut mutations = vec![
        OperationDescriptor::read::<CreateInput, MutationReport>("proposal.create"),
        OperationDescriptor::read::<EditInput, MutationReport>("proposal.edit"),
        OperationDescriptor::read::<ReviewRequest, MutationReport>("proposal.review"),
        OperationDescriptor::read::<RevalidateInput, MutationReport>("proposal.revalidate"),
        OperationDescriptor::read::<RejectInput, MutationReport>("proposal.reject"),
    ];
    for descriptor in &mut mutations {
        descriptor.effect = EffectLevel::RuntimeWrite;
        descriptor.supports_dry_run = true;
        descriptor.supports_idempotency = true;
        descriptor.policy = "validated-proposal-revision".into();
    }
    mutations.push(OperationDescriptor::read::<GetInput, ProposalRecord>(
        "proposal.get",
    ));
    let mut apply = OperationDescriptor::read::<super::apply::ApplyInput, super::apply::ApplyReport>(
        "proposal.apply",
    );
    apply.effect = EffectLevel::CanonicalWrite;
    apply.supports_dry_run = true;
    apply.supports_idempotency = true;
    apply.policy = "confirmed-reviewed-proposal".into();
    mutations.push(apply);
    mutations
}
