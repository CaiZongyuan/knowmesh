mod evidence;
mod records;

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    domain::{Claim, ClaimId, EvidenceId, Relation, RelationId, claim_conflict_groups},
    error::{AppError, AppResult, ErrorType},
};
use evidence::EvidencePool;
use records::{Assertion, index, json_hash as hash};

pub const DEDUP_VERSION: &str = "1";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AssertionChange<T> {
    pub before_semantic_sha256: Option<String>,
    pub record: T,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DedupReport {
    pub claim_changes: Vec<AssertionChange<Claim>>,
    pub relation_changes: Vec<AssertionChange<Relation>>,
    pub claim_aliases: BTreeMap<ClaimId, ClaimId>,
    pub relation_aliases: BTreeMap<RelationId, RelationId>,
    pub evidence_aliases: BTreeMap<EvidenceId, EvidenceId>,
}

pub fn deduplicate(
    existing_claims: &[Claim],
    existing_relations: &[Relation],
    incoming_claims: &[Claim],
    incoming_relations: &[Relation],
) -> AppResult<DedupReport> {
    if existing_claims
        .len()
        .saturating_add(existing_relations.len())
        > 100_000
        || incoming_claims
            .len()
            .saturating_add(incoming_relations.len())
            > 10_000
    {
        return Err(error(
            "ASSERTION_DEDUP_LIMIT",
            "Deduplication supports at most 100000 existing and 10000 incoming assertions per batch.",
        ));
    }
    let mut by_subject = BTreeMap::<_, Vec<_>>::new();
    for claim in existing_claims {
        by_subject
            .entry(&claim.subject_node_id)
            .or_default()
            .push(&claim.assertion);
    }
    let mut groups = BTreeSet::new();
    for claims in by_subject.values() {
        for group in claim_conflict_groups(claims.iter().copied())? {
            if !groups.insert(&group.id) {
                return Err(error(
                    "CONFLICT_GROUP_ID_CONFLICT",
                    "A conflict group ID cannot belong to multiple subject Nodes.",
                ));
            }
        }
    }
    let existing = existing_claims
        .iter()
        .flat_map(|claim| &claim.assertion.evidence)
        .chain(
            existing_relations
                .iter()
                .flat_map(|relation| &relation.assertion.evidence),
        );
    let incoming = incoming_claims
        .iter()
        .flat_map(|claim| &claim.assertion.evidence)
        .chain(
            incoming_relations
                .iter()
                .flat_map(|relation| &relation.assertion.evidence),
        );
    let mut evidence = EvidencePool::new(existing, incoming)?;
    let (claim_changes, claim_aliases) = process(existing_claims, incoming_claims, &mut evidence)?;
    let (relation_changes, relation_aliases) =
        process(existing_relations, incoming_relations, &mut evidence)?;
    Ok(DedupReport {
        claim_changes,
        relation_changes,
        claim_aliases,
        relation_aliases,
        evidence_aliases: evidence.aliases,
    })
}

type Processed<T> = (
    Vec<AssertionChange<T>>,
    BTreeMap<<T as Assertion>::Id, <T as Assertion>::Id>,
);

fn process<T: Assertion>(
    existing: &[T],
    incoming: &[T],
    evidence: &mut EvidencePool<'_>,
) -> AppResult<Processed<T>> {
    let existing = index(existing)?;
    let incoming = index(incoming)?;
    let mut active = BTreeMap::new();
    for record in existing.values().filter(|record| record.active()) {
        if let Some(previous) = active.insert(record.key()?, record.id().clone()) {
            return Err(error(
                "AMBIGUOUS_ASSERTION_DUPLICATE",
                "More than one active assertion has the same identity; resolve the existing duplicates first.",
            ).with_details(serde_json::json!({"assertion_ids": [previous.to_string(), record.id().to_string()]})));
        }
    }
    let mut changes = BTreeMap::<T::Id, AssertionChange<T>>::new();
    let mut aliases = BTreeMap::new();
    for record in incoming.values() {
        if !record.candidate_allowed() {
            return Err(error(
                "INVALID_DEDUP_CANDIDATE",
                "Incoming assertions must be active and must not predeclare conflict groups.",
            ));
        }
        let key = record.key()?;
        if let Some(previous) = existing.get(record.id()) {
            if !previous.active() {
                return Err(error(
                    "ASSERTION_LIFECYCLE_CONFLICT",
                    "Deduplication cannot reactivate an existing historical assertion ID.",
                ));
            }
            if previous.key()? != key {
                return Err(error(
                    "ASSERTION_ID_CONFLICT",
                    "An assertion ID cannot identify different semantics or endpoints.",
                ));
            }
        }
        let target_id = if let Some(id) = active.get(&key) {
            let current = changes
                .get(id)
                .map(|change| &change.record)
                .or_else(|| existing.get(id).copied())
                .ok_or_else(|| {
                    error(
                        "ASSERTION_DEDUP_INVALID",
                        "A duplicate target is absent from the assertion context.",
                    )
                })?;
            let mut merged = current.clone();
            merged.set_evidence(evidence.merge(current.evidence(), record.evidence())?);
            merged.validate()?;
            if merged != *current {
                let before = existing
                    .get(id)
                    .map(|record| record.semantic_hash())
                    .transpose()?;
                changes.insert(
                    id.clone(),
                    AssertionChange {
                        before_semantic_sha256: before,
                        record: merged,
                    },
                );
            }
            id.clone()
        } else {
            let mut created = (*record).clone();
            created.set_evidence(evidence.merge(&[], record.evidence())?);
            created.validate()?;
            let id = created.id().clone();
            changes.insert(
                id.clone(),
                AssertionChange {
                    before_semantic_sha256: None,
                    record: created,
                },
            );
            active.insert(key, id.clone());
            id
        };
        aliases.insert(record.id().clone(), target_id);
    }
    Ok((changes.into_values().collect(), aliases))
}

fn error(code: &str, message: &str) -> AppError {
    AppError::new(ErrorType::Validation, code, message)
}
