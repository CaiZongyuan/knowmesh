use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ClaimId, ClaimRecord, ConflictGroupId, EvidenceStatus, Timestamp, knowledge_error};
use crate::error::AppResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConflictGroupStatus {
    Open,
    Resolved,
    Dismissed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConflictGroup {
    pub id: ConflictGroupId,
    pub claim_ids: Vec<ClaimId>,
    pub reason: String,
    pub status: ConflictGroupStatus,
    pub created_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<Timestamp>,
}

impl ConflictGroup {
    pub fn validate(&self) -> AppResult<()> {
        if !(2..=128).contains(&self.claim_ids.len())
            || self.claim_ids.windows(2).any(|pair| pair[0] >= pair[1])
            || self.reason.trim().is_empty()
            || self.reason.chars().take(2049).count() > 2048
            || (self.status == ConflictGroupStatus::Open) != self.resolved_at.is_none()
            || self.resolved_at.is_some_and(|time| time < self.created_at)
        {
            return Err(knowledge_error(
                "INVALID_CONFLICT_GROUP",
                "Conflict groups require 2..=128 sorted unique Claims, a bounded reason, and consistent resolution timestamps.",
            ));
        }
        Ok(())
    }
}

/// Validate all copies and members of conflict groups owned by one subject Node.
pub fn claim_conflict_groups<'a>(
    claims: impl IntoIterator<Item = &'a ClaimRecord>,
) -> AppResult<Vec<&'a ConflictGroup>> {
    let mut records = BTreeMap::new();
    let mut groups = BTreeMap::new();
    for claim in claims {
        claim.validate()?;
        if records.insert(&claim.id, claim).is_some() {
            return Err(knowledge_error(
                "DUPLICATE_ASSERTION_ID",
                "Conflict members require unique Claim IDs.",
            ));
        }
        for group in &claim.conflict_groups {
            if let Some(previous) = groups.insert(&group.id, group)
                && previous != group
            {
                return Err(knowledge_error(
                    "CONFLICT_GROUP_ID_CONFLICT",
                    "Every copy of a conflict group must have identical fields.",
                ));
            }
        }
    }
    for group in groups.values() {
        let mut qualifiers = None;
        for id in &group.claim_ids {
            let member = records.get(id).ok_or_else(|| {
                knowledge_error(
                    "CONFLICT_CLAIM_MISSING",
                    "A conflict member is absent from its subject Node.",
                )
            })?;
            if !member
                .conflict_groups
                .iter()
                .any(|copy| copy.id == group.id)
            {
                return Err(knowledge_error(
                    "CONFLICT_GROUP_INCOMPLETE",
                    "Every conflict member must retain its shared conflict record.",
                ));
            }
            if qualifiers.is_some_and(|scope| scope != &member.qualifiers) {
                return Err(knowledge_error(
                    "CONFLICT_SCOPE_MISMATCH",
                    "Conflict members must have identical qualifier scopes.",
                ));
            }
            qualifiers = Some(&member.qualifiers);
            if group.status == ConflictGroupStatus::Open
                && member.evidence_status != EvidenceStatus::Conflicting
            {
                return Err(knowledge_error(
                    "CONFLICT_STATUS_MISMATCH",
                    "Members of an open conflict group must be marked conflicting.",
                ));
            }
        }
    }
    Ok(groups.into_values().collect())
}
