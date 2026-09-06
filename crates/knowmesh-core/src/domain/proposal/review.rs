use std::collections::{BTreeMap, BTreeSet};

use super::{
    Decision, DecisionChange, Proposal, ProposalItem, ProposalRevision, ProposalState, ReviewInput,
    ReviewMethod, ReviewPolicy, conflict, derived_state, error, invalid, review_hash, text,
};
use crate::{
    domain::Timestamp,
    error::{AppResult, ErrorType},
};

impl Proposal {
    pub fn review(
        &self,
        input: &ReviewInput,
        policy: &ReviewPolicy,
        actor: &str,
        now: Timestamp,
    ) -> AppResult<Self> {
        self.check_mutable(input.expected_revision, false)?;
        if !text(actor, 256)
            || now < self.created_at
            || input.decisions.len() > 10_000
            || (input.accept_all && !input.decisions.is_empty())
        {
            return Err(invalid());
        }
        if input.accept_all
            && (policy.strict || !policy.allow_accept_all || policy.human_verification_required)
        {
            return Err(error(
                ErrorType::Policy,
                "STRICT_REVIEW_REQUIRED",
                "The current policy requires explicit per-item review.",
            ));
        }
        let bulk: Vec<_> = self
            .items
            .iter()
            .filter(|item| item.decision == Decision::Pending)
            .map(|item| DecisionChange {
                item_id: item.id.clone(),
                decision: Decision::Accepted,
                reason: None,
                human_verified: false,
            })
            .collect();
        let decisions = if input.accept_all {
            &bulk
        } else {
            &input.decisions
        };
        let method = if input.accept_all {
            ReviewMethod::Bulk
        } else {
            ReviewMethod::Explicit
        };
        let mut next = self.clone();
        let mut seen = BTreeSet::new();
        let indices: BTreeMap<_, _> = self
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| (&item.id, index))
            .collect();
        let review_context = self.review_context_sha256()?;
        for decision in decisions {
            if !seen.insert(&decision.item_id) {
                return Err(invalid());
            }
            let index = indices.get(&decision.item_id).ok_or_else(|| {
                error(
                    ErrorType::NotFound,
                    "PROPOSAL_ITEM_NOT_FOUND",
                    "The decision refers to an item outside this Proposal.",
                )
            })?;
            let item = &mut next.items[*index];
            if decision
                .reason
                .as_ref()
                .is_some_and(|reason| !text(reason, 4096))
            {
                return Err(invalid());
            }
            if decision.decision == Decision::Accepted {
                if item.issues.iter().any(|issue| issue.blocking) {
                    return Err(error(
                        ErrorType::Validation,
                        "PROPOSAL_ITEM_BLOCKED",
                        "Repair blocking item issues in a new revision before accepting it.",
                    ));
                }
                if policy.human_verification_required && !decision.human_verified {
                    return Err(error(
                        ErrorType::Policy,
                        "HUMAN_VERIFICATION_REQUIRED",
                        "This item requires explicit human verification.",
                    ));
                }
            }
            let human_verified = decision.decision == Decision::Accepted && decision.human_verified;
            let next_method = (decision.decision != Decision::Pending).then_some(method);
            if item.decision == decision.decision
                && item.decision_reason == decision.reason
                && item.human_verified == human_verified
                && item.review_method == next_method
                && (item.decision == Decision::Pending
                    || item.reviewed_by.as_deref() == Some(actor))
            {
                continue;
            }
            item.reset_review();
            item.decision = decision.decision;
            item.decision_reason = decision.reason.clone();
            if decision.decision != Decision::Pending {
                item.reviewed_at = Some(now.max(self.updated_at));
                item.reviewed_by = Some(actor.into());
                item.review_method = next_method;
                item.human_verified = human_verified;
                item.reviewed_sha256 = Some(review_hash(&review_context, item)?);
            }
        }
        if next.items == self.items {
            return Ok(self.clone());
        }
        next.advance(actor, now)?;
        next.state = derived_state(&next.items);
        next.validate()?;
        Ok(next)
    }

    pub fn revise(&self, input: &ProposalRevision, actor: &str, now: Timestamp) -> AppResult<Self> {
        self.check_mutable(input.expected_revision, true)?;
        let mut next = self.clone();
        let previous: BTreeMap<_, _> = self.items.iter().map(|item| (&item.id, item)).collect();
        let reset_all = input.base_generation != self.base_generation
            || input.schema_hash != self.schema_hash
            || self.state == ProposalState::Stale
            || input.items.iter().map(|item| &item.id).collect::<Vec<_>>()
                != self.items.iter().map(|item| &item.id).collect::<Vec<_>>();
        let mut items = vec![];
        for replacement in &input.items {
            let mut item = replacement.clone();
            if !reset_all
                && let Some(old) = previous.get(&item.id)
                && old.content_sha256()? == item.content_sha256()?
            {
                item = (*old).clone();
            } else {
                item.reset_review();
            }
            items.push(item);
        }
        next.items = items;
        next.base_generation = input.base_generation;
        next.schema_hash = input.schema_hash.clone();
        next.summary = input.summary.clone();
        next.state_reason = None;
        next.state = derived_state(&next.items);
        if next == *self {
            return Ok(self.clone());
        }
        next.advance(actor, now)?;
        next.validate()?;
        Ok(next)
    }

    pub fn require_approved(&self, policy: &ReviewPolicy) -> AppResult<Vec<&ProposalItem>> {
        self.validate()?;
        if self.state == ProposalState::Stale {
            return Err(conflict(
                "STALE_PROPOSAL",
                "Revalidate the stale Proposal before Apply.",
            ));
        }
        if self.state != ProposalState::Approved {
            return Err(conflict(
                "PROPOSAL_REVIEW_REQUIRED",
                "All Proposal items must be accepted or rejected before Apply.",
            ));
        }
        let selected: Vec<_> = self
            .items
            .iter()
            .filter(|item| item.decision == Decision::Accepted)
            .collect();
        for item in &selected {
            if (policy.strict || !policy.allow_accept_all || policy.human_verification_required)
                && item.review_method != Some(ReviewMethod::Explicit)
            {
                return Err(error(
                    ErrorType::Policy,
                    "STRICT_REVIEW_REQUIRED",
                    "Bulk approval does not satisfy the current policy.",
                ));
            }
            if policy.human_verification_required && !item.human_verified {
                return Err(error(
                    ErrorType::Policy,
                    "HUMAN_VERIFICATION_REQUIRED",
                    "The current policy requires human verification.",
                ));
            }
        }
        Ok(selected)
    }
}
