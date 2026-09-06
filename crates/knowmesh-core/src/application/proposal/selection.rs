use std::{collections::BTreeMap, path::PathBuf};

use super::prepare_snapshot;
use crate::{
    canonical::{
        schema::{ReviewMode, Schema},
        snapshot::{CanonicalPreview, CanonicalSnapshot},
        workspace::Workspace,
    },
    domain::{
        ProposalId,
        proposal::{Proposal, ProposalInput, ReviewPolicy},
    },
    error::{AppError, AppResult, ErrorType},
};

#[derive(Debug)]
pub struct AcceptedPreview {
    proposal_id: ProposalId,
    proposal_revision: u32,
    base_snapshot_sha256: String,
    preview: CanonicalPreview,
    documents: BTreeMap<PathBuf, Vec<u8>>,
}

impl AcceptedPreview {
    pub fn proposal_id(&self) -> &ProposalId {
        &self.proposal_id
    }
    pub fn proposal_revision(&self) -> u32 {
        self.proposal_revision
    }
    pub fn base_snapshot_sha256(&self) -> &str {
        &self.base_snapshot_sha256
    }
    pub fn preview(&self) -> &CanonicalPreview {
        &self.preview
    }
    pub fn documents(&self) -> &BTreeMap<PathBuf, Vec<u8>> {
        &self.documents
    }
}

/// Preview a reviewed subset using the generation and base hash loaded by the coordinator.
pub fn prepare_accepted(
    workspace: &Workspace,
    proposal: &Proposal,
    expected_revision: u32,
    indexed_generation: u64,
    base_snapshot_sha256: &str,
) -> AppResult<AcceptedPreview> {
    proposal.validate()?;
    if expected_revision != proposal.revision {
        return Err(conflict(
            "PROPOSAL_REVISION_MISMATCH",
            "Fetch the current Proposal revision before Apply.",
        ));
    }
    let before = CanonicalSnapshot::scan(workspace)?;
    let schema = Schema::load(workspace)?;
    let policies = &schema.policies;
    let selected = proposal.require_approved(&ReviewPolicy {
        strict: policies.review_mode == ReviewMode::Strict,
        allow_accept_all: policies.allow_accept_all,
        human_verification_required: policies.human_verification_required,
    })?;
    if indexed_generation != proposal.base_generation
        || base_snapshot_sha256 != before.content_sha256
        || proposal.schema_hash != before.schema_hash
        || schema.hash != before.schema_hash
    {
        return Err(conflict(
            "STALE_PROPOSAL",
            "The canonical baseline, Schema, or index generation changed. Revalidate and review a new revision.",
        ));
    }
    let prepared = prepare_snapshot(
        workspace,
        &ProposalInput {
            kind: proposal.kind,
            base_generation: proposal.base_generation,
            schema_hash: proposal.schema_hash.clone(),
            source_revision_id: proposal.source_revision_id.clone(),
            compiler_run_id: proposal.compiler_run_id.clone(),
            summary: proposal.summary.clone(),
            items: selected.iter().map(|item| (*item).clone()).collect(),
        },
        &proposal.updated_by,
        proposal.updated_at,
        before,
    )?;
    let invalid: Vec<_> = prepared
        .proposal
        .items
        .iter()
        .filter(|item| item.issues.iter().any(|issue| issue.blocking))
        .map(|item| {
            serde_json::json!({
                "item_id":item.id, "issues":item.issues,
            })
        })
        .collect();
    if !invalid.is_empty() {
        return Err(conflict(
            "PROPOSAL_ACCEPTED_ITEMS_INVALID",
            "The accepted subset has invalid payloads, Evidence, or missing dependencies.",
        )
        .with_details(serde_json::json!({"items":invalid})));
    }
    for (reviewed, checked) in selected.iter().zip(&prepared.proposal.items) {
        if reviewed.content_sha256()? != checked.content_sha256()? {
            return Err(conflict(
                "PROPOSAL_REVALIDATION_REQUIRED",
                "Validation changed reviewed item content. Prepare and review a new revision.",
            ));
        }
    }
    let preview = prepared.preview.ok_or_else(|| {
        conflict(
            "PROPOSAL_ACCEPTED_ITEMS_INVALID",
            "The accepted subset has no valid canonical preview.",
        )
    })?;
    Ok(AcceptedPreview {
        proposal_id: proposal.id.clone(),
        proposal_revision: proposal.revision,
        base_snapshot_sha256: prepared.base_snapshot_sha256,
        preview,
        documents: prepared.documents,
    })
}

fn conflict(code: &str, message: &str) -> AppError {
    AppError::new(ErrorType::Conflict, code, message)
}
