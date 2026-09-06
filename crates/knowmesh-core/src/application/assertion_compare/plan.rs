use std::collections::{BTreeMap, BTreeSet};

use super::{
    BlockedConflict, ClaimComparisonContext, ComparisonReport, ComparisonVerdict, ConflictPlan,
    PROMPT, error, hash,
};
use crate::{
    application::assertion_dedup::AssertionChange,
    domain::{
        Claim, ClaimId, ConflictGroup, ConflictGroupId, ConflictGroupStatus, EvidenceStatus,
        Timestamp, sha256,
    },
    error::AppResult,
};

impl ClaimComparisonContext<'_> {
    pub fn plan(
        &self,
        report: &ComparisonReport,
        created_at: Timestamp,
    ) -> AppResult<ConflictPlan> {
        let pairs: Vec<_> = report
            .comparisons
            .iter()
            .map(|comparison| comparison.pair.clone())
            .collect();
        let pairs = self.checked_pairs(&pairs).map_err(|_| stale())?;
        if report.version != 1
            || report.context_sha256 != self.context_sha256
            || report.prompt_sha256 != sha256(PROMPT.as_bytes())
            || report.input_sha256 != hash(&self.model_input(&pairs))?
        {
            return Err(stale());
        }
        for comparison in &report.comparisons {
            let left = self.claims[&comparison.pair.left_id];
            let right = self.claims[&comparison.pair.right_id];
            if comparison.pair.left_id >= comparison.pair.right_id
                || comparison.left_semantic_sha256
                    != left.assertion.semantic_hash(&left.subject_node_id)?
                || comparison.right_semantic_sha256
                    != right.assertion.semantic_hash(&right.subject_node_id)?
                || comparison.reason.trim().is_empty()
                || comparison.reason.chars().take(2049).count() > 2048
            {
                return Err(stale());
            }
        }
        let mut plan = ConflictPlan {
            context_sha256: self.context_sha256.clone(),
            report_sha256: hash(report)?,
            groups: vec![],
            claim_changes: vec![],
            possible_duplicates: vec![],
            undetermined: vec![],
            blocked_conflicts: vec![],
            existing_group_ids: vec![],
            requires_review: true,
        };
        let mut changes = BTreeMap::<ClaimId, AssertionChange<Claim>>::new();
        let mut existing_groups = BTreeSet::new();
        let mut comparisons: Vec<_> = report.comparisons.iter().collect();
        comparisons.sort_by(|left, right| left.pair.cmp(&right.pair));
        for comparison in comparisons {
            match comparison.verdict {
                ComparisonVerdict::Independent => continue,
                ComparisonVerdict::PossibleDuplicate => {
                    plan.possible_duplicates.push(comparison.clone());
                    continue;
                }
                ComparisonVerdict::Undetermined => {
                    plan.undetermined.push(comparison.clone());
                    continue;
                }
                ComparisonVerdict::Conflicting => {}
            }
            let ids = [&comparison.pair.left_id, &comparison.pair.right_id];
            let mut members: Vec<_> = ids
                .iter()
                .map(|id| {
                    changes
                        .get(*id)
                        .map(|change| &change.record)
                        .unwrap_or(self.claims[*id])
                        .clone()
                })
                .collect();
            if let Some(group) = members[0].assertion.conflict_groups.iter().find(|group| {
                group.status == ConflictGroupStatus::Open && group.claim_ids.contains(ids[1])
            }) {
                existing_groups.insert(group.id.clone());
                continue;
            }
            let blocked = if members
                .iter()
                .any(|claim| claim.assertion.evidence.is_empty())
            {
                Some("EVIDENCE_REQUIRED")
            } else if members
                .iter()
                .any(|claim| claim.assertion.conflict_groups.len() >= 128)
            {
                Some("CONFLICT_GROUP_LIMIT")
            } else {
                None
            };
            if let Some(code) = blocked {
                plan.blocked_conflicts.push(BlockedConflict {
                    pair: comparison.pair.clone(),
                    code: code.into(),
                });
                continue;
            }
            let group = ConflictGroup {
                id: ConflictGroupId::new(),
                claim_ids: ids.into_iter().cloned().collect(),
                reason: comparison.reason.clone(),
                status: ConflictGroupStatus::Open,
                created_at,
                resolved_at: None,
            };
            for member in &mut members {
                member.assertion.evidence_status = EvidenceStatus::Conflicting;
                member.assertion.conflict_groups.push(group.clone());
                member
                    .assertion
                    .conflict_groups
                    .sort_by(|left, right| left.id.cmp(&right.id));
                member.assertion.validate()?;
            }
            for member in members {
                let original = self.claims[&member.assertion.id];
                changes.insert(
                    member.assertion.id.clone(),
                    AssertionChange {
                        before_semantic_sha256: Some(
                            original
                                .assertion
                                .semantic_hash(&original.subject_node_id)?,
                        ),
                        record: member,
                    },
                );
            }
            plan.groups.push(group);
        }
        plan.claim_changes = changes.into_values().collect();
        plan.existing_group_ids = existing_groups.into_iter().collect();
        Ok(plan)
    }
}

fn stale() -> crate::error::AppError {
    error(
        "CLAIM_COMPARISON_STALE",
        "The comparison report does not match the current Claims, prompt, or comparison input.",
    )
}
