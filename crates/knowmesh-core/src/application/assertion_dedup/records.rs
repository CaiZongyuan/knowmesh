use std::collections::BTreeMap;

use serde::Serialize;

use super::{error, hash};
use crate::{
    domain::{
        Claim, ClaimId, Evidence, EvidenceStance, EvidenceStatus, LifecycleStatus, Relation,
        RelationId,
    },
    error::AppResult,
};

pub(super) trait Assertion: Clone + PartialEq {
    type Id: Ord + Clone + std::fmt::Display;
    fn id(&self) -> &Self::Id;
    fn validate(&self) -> AppResult<()>;
    fn active(&self) -> bool;
    fn candidate_allowed(&self) -> bool;
    fn key(&self) -> AppResult<String>;
    fn semantic_hash(&self) -> AppResult<String>;
    fn evidence(&self) -> &[Evidence];
    fn set_evidence(&mut self, evidence: Vec<Evidence>);
}

impl Assertion for Claim {
    type Id = ClaimId;
    fn id(&self) -> &ClaimId {
        &self.assertion.id
    }
    fn validate(&self) -> AppResult<()> {
        self.assertion.validate()
    }
    fn active(&self) -> bool {
        self.assertion.lifecycle_status == LifecycleStatus::Active
    }
    fn candidate_allowed(&self) -> bool {
        self.active() && self.assertion.conflict_groups.is_empty()
    }
    fn key(&self) -> AppResult<String> {
        hash(&(
            "claim-identity-v1",
            &self.subject_node_id,
            self.assertion.normalized_hash()?,
        ))
    }
    fn semantic_hash(&self) -> AppResult<String> {
        self.assertion.semantic_hash(&self.subject_node_id)
    }
    fn evidence(&self) -> &[Evidence] {
        &self.assertion.evidence
    }
    fn set_evidence(&mut self, evidence: Vec<Evidence>) {
        self.assertion.evidence = evidence;
        mark_conflicting(
            &self.assertion.evidence,
            &mut self.assertion.evidence_status,
        );
    }
}

impl Assertion for Relation {
    type Id = RelationId;
    fn id(&self) -> &RelationId {
        &self.assertion.id
    }
    fn validate(&self) -> AppResult<()> {
        self.assertion.validate()
    }
    fn active(&self) -> bool {
        self.assertion.lifecycle_status == LifecycleStatus::Active
    }
    fn candidate_allowed(&self) -> bool {
        self.active()
    }
    fn key(&self) -> AppResult<String> {
        let (source, target) =
            if !self.assertion.directed && self.source_node_id > self.assertion.target_node_id {
                (&self.assertion.target_node_id, &self.source_node_id)
            } else {
                (&self.source_node_id, &self.assertion.target_node_id)
            };
        hash(&(
            "relation-identity-v1",
            source,
            &self.assertion.predicate,
            target,
            self.assertion.directed,
            &self.assertion.qualifiers,
        ))
    }
    fn semantic_hash(&self) -> AppResult<String> {
        self.assertion.semantic_hash(&self.source_node_id)
    }
    fn evidence(&self) -> &[Evidence] {
        &self.assertion.evidence
    }
    fn set_evidence(&mut self, evidence: Vec<Evidence>) {
        self.assertion.evidence = evidence;
        mark_conflicting(
            &self.assertion.evidence,
            &mut self.assertion.evidence_status,
        );
    }
}

fn mark_conflicting(evidence: &[Evidence], status: &mut EvidenceStatus) {
    if evidence
        .iter()
        .any(|evidence| evidence.stance == EvidenceStance::Supports)
        && evidence
            .iter()
            .any(|evidence| evidence.stance == EvidenceStance::Contradicts)
    {
        *status = EvidenceStatus::Conflicting;
    }
}

pub(super) fn index<T: Assertion>(records: &[T]) -> AppResult<BTreeMap<T::Id, &T>> {
    let mut result = BTreeMap::new();
    for record in records {
        record.validate()?;
        if result.insert(record.id().clone(), record).is_some() {
            return Err(error(
                "DUPLICATE_ASSERTION_ID",
                "An assertion collection cannot repeat an identity.",
            ));
        }
    }
    Ok(result)
}

pub(super) fn json_hash(value: &impl Serialize) -> AppResult<String> {
    serde_json::to_vec(value)
        .map(|bytes| crate::domain::sha256(&bytes))
        .map_err(|_| {
            error(
                "ASSERTION_DEDUP_INVALID",
                "Could not encode assertion identity.",
            )
        })
}
