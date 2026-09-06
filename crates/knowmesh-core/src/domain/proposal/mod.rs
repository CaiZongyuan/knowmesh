mod review;
mod types;

use std::collections::BTreeSet;

pub use types::*;

use crate::{
    domain::{
        ClaimId, NodeId, ProposalId, ProposalItemId, RelationId, SourceId, SynthesisId, Timestamp,
        sha256, valid_sha256,
    },
    error::{AppError, AppResult, ErrorType},
};

impl PatchOp {
    fn validate_target(self, id: &str) -> AppResult<()> {
        match self {
            Self::CreateNode
            | Self::UpdateNodeSummary
            | Self::AddAlias
            | Self::AddClaim
            | Self::AddRelation
            | Self::RecordClaimConflict => id.parse::<NodeId>().map(|_| ()),
            Self::SupersedeClaim | Self::RetractClaim => id.parse::<ClaimId>().map(|_| ()),
            Self::SupersedeRelation | Self::RetractRelation => id.parse::<RelationId>().map(|_| ()),
            Self::AddEvidence => id
                .parse::<ClaimId>()
                .map(|_| ())
                .or_else(|_| id.parse::<RelationId>().map(|_| ())),
            Self::CreateSynthesis => id.parse::<SynthesisId>().map(|_| ()),
            Self::UpdateSourceMetadata => id.parse::<SourceId>().map(|_| ()),
        }
        .map_err(|error| error.with_param("target_id"))
    }
}

impl std::str::FromStr for PatchOp {
    type Err = AppError;
    fn from_str(value: &str) -> AppResult<Self> {
        serde_json::from_value(serde_json::Value::String(value.into())).map_err(|_| {
            error(
                ErrorType::Validation,
                "INVALID_PATCH_OPERATION",
                "Unknown Proposal patch operation.",
            )
        })
    }
}

impl ProposalItem {
    pub fn new(op: PatchOp, target_id: String, payload: serde_json::Value) -> AppResult<Self> {
        let item = Self {
            id: ProposalItemId::new(),
            op,
            target_id,
            payload,
            before_sha256: None,
            evidence_ids: vec![],
            compiler_confidence: None,
            risk: Risk::Medium,
            decision: Decision::Pending,
            decision_reason: None,
            issues: vec![],
            reviewed_sha256: None,
            reviewed_at: None,
            reviewed_by: None,
            review_method: None,
            human_verified: false,
        };
        item.validate()?;
        Ok(item)
    }

    pub fn content_sha256(&self) -> AppResult<String> {
        let bytes = serde_json::to_vec(&(
            "proposal-item-v1",
            &self.id,
            self.op,
            &self.target_id,
            &self.payload,
            &self.before_sha256,
            &self.evidence_ids,
            self.compiler_confidence,
            self.risk,
            &self.issues,
        ))
        .map_err(|_| invalid())?;
        Ok(sha256(&bytes))
    }

    pub fn validate(&self) -> AppResult<()> {
        self.op.validate_target(&self.target_id)?;
        if !self.payload.is_object()
            || serde_json::to_vec(&self.payload)
                .map_err(|_| invalid())?
                .len()
                > 1024 * 1024
            || self
                .before_sha256
                .as_ref()
                .is_some_and(|hash| !valid_sha256(hash))
            || self
                .compiler_confidence
                .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
            || self.evidence_ids.len() > 1024
            || self.evidence_ids.iter().collect::<BTreeSet<_>>().len() != self.evidence_ids.len()
            || self.issues.len() > 128
            || self
                .issues
                .iter()
                .any(|issue| !text(&issue.code, 128) || !text(&issue.message, 4096))
            || self
                .decision_reason
                .as_ref()
                .is_some_and(|reason| !text(reason, 4096))
        {
            return Err(invalid());
        }
        if self.decision == Decision::Pending {
            if self.reviewed_sha256.is_some()
                || self.reviewed_at.is_some()
                || self.reviewed_by.is_some()
                || self.review_method.is_some()
                || self.human_verified
            {
                return Err(invalid());
            }
        } else {
            if self
                .reviewed_sha256
                .as_ref()
                .is_none_or(|hash| !valid_sha256(hash))
                || self.reviewed_at.is_none()
                || self.review_method.is_none()
                || self
                    .reviewed_by
                    .as_ref()
                    .is_none_or(|actor| !text(actor, 256))
            {
                return Err(invalid());
            }
            if self.decision == Decision::Accepted && self.issues.iter().any(|issue| issue.blocking)
            {
                return Err(error(
                    ErrorType::Validation,
                    "PROPOSAL_ITEM_BLOCKED",
                    "An accepted item cannot contain blocking validation issues.",
                ));
            }
        }
        Ok(())
    }

    fn reset_review(&mut self) {
        self.decision = Decision::Pending;
        self.decision_reason = None;
        self.reviewed_sha256 = None;
        self.reviewed_at = None;
        self.reviewed_by = None;
        self.review_method = None;
        self.human_verified = false;
    }
}

impl Proposal {
    pub fn new(input: ProposalInput, actor: &str, now: Timestamp) -> AppResult<Self> {
        let mut items = input.items;
        for item in &mut items {
            item.reset_review();
        }
        let proposal = Self {
            version: 1,
            id: ProposalId::new(),
            kind: input.kind,
            state: ProposalState::Draft,
            revision: 1,
            base_generation: input.base_generation,
            schema_hash: input.schema_hash,
            source_revision_id: input.source_revision_id,
            compiler_run_id: input.compiler_run_id,
            summary: input.summary,
            items,
            created_by: actor.into(),
            updated_by: actor.into(),
            created_at: now,
            updated_at: now,
            state_reason: None,
            applied_at: None,
            applied_generation: None,
        };
        proposal.validate()?;
        Ok(proposal)
    }

    pub fn validate(&self) -> AppResult<()> {
        if self.version != 1
            || self.revision == 0
            || !valid_sha256(&self.schema_hash)
            || !text(&self.created_by, 256)
            || !text(&self.updated_by, 256)
            || !text(&self.summary, 16 * 1024)
            || self.updated_at < self.created_at
            || !(1..=10_000).contains(&self.items.len())
            || self
                .state_reason
                .as_ref()
                .is_some_and(|reason| !text(reason, 4096))
        {
            return Err(invalid());
        }
        let mut ids = BTreeSet::new();
        let mut bytes = 0usize;
        let review_context = self.review_context_sha256()?;
        for item in &self.items {
            item.validate()?;
            if item.decision != Decision::Pending
                && item.reviewed_sha256.as_ref() != Some(&review_hash(&review_context, item)?)
            {
                return Err(conflict(
                    "PROPOSAL_REVIEW_STALE",
                    "The item or its Proposal context changed after review.",
                ));
            }
            if !ids.insert(&item.id)
                || item
                    .reviewed_at
                    .is_some_and(|at| at < self.created_at || at > self.updated_at)
            {
                return Err(invalid());
            }
            bytes = bytes.saturating_add(serde_json::to_vec(item).map_err(|_| invalid())?.len());
            if bytes > 16 * 1024 * 1024 {
                return Err(invalid());
            }
        }
        if self.state == ProposalState::Applied {
            if self
                .applied_at
                .is_none_or(|at| at < self.created_at || at > self.updated_at)
                || self.applied_generation.is_none()
                || derived_state(&self.items) != ProposalState::Approved
            {
                return Err(invalid());
            }
        } else if self.applied_at.is_some() || self.applied_generation.is_some() {
            return Err(invalid());
        }
        if matches!(
            self.state,
            ProposalState::Draft | ProposalState::Reviewing | ProposalState::Approved
        ) && self.state != derived_state(&self.items)
        {
            return Err(invalid());
        }
        if self.state == ProposalState::Stale && self.state_reason.is_none()
            || self.state == ProposalState::Rejected
                && self.state_reason.is_none()
                && derived_state(&self.items) != ProposalState::Rejected
            || !matches!(self.state, ProposalState::Stale | ProposalState::Rejected)
                && self.state_reason.is_some()
        {
            return Err(invalid());
        }
        Ok(())
    }

    fn review_context_sha256(&self) -> AppResult<String> {
        let bytes = serde_json::to_vec(&(
            "proposal-review-context-v1",
            &self.id,
            self.kind,
            self.base_generation,
            &self.schema_hash,
            &self.source_revision_id,
            &self.compiler_run_id,
            &self.created_by,
            self.created_at,
            self.items.iter().map(|item| &item.id).collect::<Vec<_>>(),
        ))
        .map_err(|_| invalid())?;
        Ok(sha256(&bytes))
    }

    fn check_mutable(&self, expected_revision: u32, allow_stale: bool) -> AppResult<()> {
        self.validate()?;
        if matches!(self.state, ProposalState::Applied | ProposalState::Rejected) {
            return Err(conflict(
                "PROPOSAL_FINALIZED",
                "The finalized Proposal cannot be edited or reviewed.",
            ));
        }
        if self.state == ProposalState::Stale && !allow_stale {
            return Err(conflict(
                "STALE_PROPOSAL",
                "Revalidate and revise the stale Proposal before reviewing it.",
            ));
        }
        if expected_revision != self.revision {
            return Err(conflict(
                "PROPOSAL_REVISION_MISMATCH",
                "The Proposal revision changed; fetch the current revision.",
            )
            .with_details(serde_json::json!({"current_revision": self.revision})));
        }
        Ok(())
    }

    fn advance(&mut self, actor: &str, now: Timestamp) -> AppResult<()> {
        if !text(actor, 256) || now < self.created_at {
            return Err(invalid());
        }
        self.revision = self.revision.checked_add(1).ok_or_else(|| {
            conflict(
                "PROPOSAL_REVISION_LIMIT",
                "The Proposal revision cannot advance further.",
            )
        })?;
        self.updated_by = actor.into();
        self.updated_at = now.max(self.updated_at);
        Ok(())
    }

    /// Finalize metadata after the Application coordinator has committed canonical data.
    pub fn mark_applied(
        &self,
        expected_revision: u32,
        generation: u64,
        actor: &str,
        now: Timestamp,
    ) -> AppResult<Self> {
        self.validate()?;
        if self.state == ProposalState::Applied {
            return Ok(self.clone());
        }
        self.check_mutable(expected_revision, false)?;
        if self.state != ProposalState::Approved {
            return Err(conflict(
                "PROPOSAL_REVIEW_REQUIRED",
                "The Proposal requires complete review before Apply.",
            ));
        }
        if generation < self.base_generation {
            return Err(invalid());
        }
        let mut next = self.clone();
        next.advance(actor, now)?;
        next.state = ProposalState::Applied;
        next.applied_at = Some(next.updated_at);
        next.applied_generation = Some(generation);
        next.validate()?;
        Ok(next)
    }

    pub fn reject(
        &self,
        expected_revision: u32,
        reason: &str,
        actor: &str,
        now: Timestamp,
    ) -> AppResult<Self> {
        self.validate()?;
        if self.state == ProposalState::Rejected && self.state_reason.as_deref() == Some(reason) {
            return Ok(self.clone());
        }
        self.check_mutable(expected_revision, true)?;
        if !text(reason, 4096) {
            return Err(invalid());
        }
        let mut next = self.clone();
        next.advance(actor, now)?;
        next.state = ProposalState::Rejected;
        next.state_reason = Some(reason.into());
        next.validate()?;
        Ok(next)
    }

    pub fn mark_stale(
        &self,
        expected_revision: u32,
        reason: &str,
        actor: &str,
        now: Timestamp,
    ) -> AppResult<Self> {
        self.check_mutable(expected_revision, true)?;
        if !text(reason, 4096) {
            return Err(invalid());
        }
        if self.state == ProposalState::Stale && self.state_reason.as_deref() == Some(reason) {
            return Ok(self.clone());
        }
        let mut next = self.clone();
        next.advance(actor, now)?;
        next.state = ProposalState::Stale;
        next.state_reason = Some(reason.into());
        next.validate()?;
        Ok(next)
    }
}

fn derived_state(items: &[ProposalItem]) -> ProposalState {
    if items.iter().all(|item| item.decision == Decision::Pending) {
        ProposalState::Draft
    } else if items.iter().any(|item| item.decision == Decision::Pending) {
        ProposalState::Reviewing
    } else if items.iter().any(|item| item.decision == Decision::Accepted) {
        ProposalState::Approved
    } else {
        ProposalState::Rejected
    }
}

fn review_hash(context: &str, item: &ProposalItem) -> AppResult<String> {
    let bytes = serde_json::to_vec(&(
        context,
        item.content_sha256()?,
        item.decision,
        &item.decision_reason,
        item.reviewed_at,
        &item.reviewed_by,
        item.review_method,
        item.human_verified,
    ))
    .map_err(|_| invalid())?;
    Ok(sha256(&bytes))
}

fn text(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max_bytes && !value.contains('\0')
}
fn error(kind: ErrorType, code: &str, message: &str) -> AppError {
    AppError::new(kind, code, message)
}
fn conflict(code: &str, message: &str) -> AppError {
    error(ErrorType::Conflict, code, message)
}
fn invalid() -> AppError {
    error(
        ErrorType::Validation,
        "INVALID_PROPOSAL",
        "Proposal metadata, item shape, or review state is invalid.",
    )
}
