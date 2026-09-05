use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{AssertionDependency, DependencySnapshot, EvidenceId, SourceId, SourceRevisionId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    Current,
    NeedsReview,
    Unknown,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessReasonCode {
    DependencyChanged,
    DependencyMissing,
    IndexIncomplete,
    SnapshotMissing,
    SourceRemoved,
    SourceRevisionBehind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FreshnessReason {
    pub code: FreshnessReasonCode,
    pub dependency_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FreshnessReport {
    pub freshness: Freshness,
    pub freshness_reasons: Vec<FreshnessReason>,
    pub evidence_ids: Vec<EvidenceId>,
    pub current_evidence_ids: Vec<EvidenceId>,
}

#[derive(Debug, Clone)]
pub struct SourceState {
    pub current_revision_id: SourceRevisionId,
    pub removed: bool,
}

#[derive(Debug, Clone)]
pub struct AssertionState {
    pub dependency: AssertionDependency,
    pub evidence_ids: Vec<EvidenceId>,
}

#[derive(Debug, Default)]
pub struct FreshnessContext {
    pub index_complete: bool,
    pub sources: BTreeMap<SourceId, SourceState>,
    pub revisions: BTreeMap<SourceRevisionId, SourceId>,
    pub evidence: BTreeMap<EvidenceId, SourceRevisionId>,
    pub assertions: BTreeMap<String, AssertionState>,
}

pub fn assertion_freshness(evidence: &[EvidenceId], context: &FreshnessContext) -> FreshnessReport {
    let mut evaluation = Evaluation::default();
    evaluation.evidence.extend(evidence.iter().cloned());
    evaluation.finish(context)
}

pub fn synthesis_freshness(
    evidence: &[EvidenceId],
    snapshot: Option<&DependencySnapshot>,
    context: &FreshnessContext,
) -> FreshnessReport {
    let mut evaluation = Evaluation::default();
    evaluation.evidence.extend(evidence.iter().cloned());
    match snapshot {
        None => evaluation.add(FreshnessReasonCode::SnapshotMissing, None),
        Some(snapshot) => {
            for dependency in &snapshot.assertions {
                match context.assertions.get(dependency.id()) {
                    Some(current) => {
                        if &current.dependency != dependency {
                            evaluation.add(
                                FreshnessReasonCode::DependencyChanged,
                                Some(dependency.id()),
                            );
                        }
                        evaluation
                            .evidence
                            .extend(current.evidence_ids.iter().cloned());
                    }
                    None => evaluation.add(
                        FreshnessReasonCode::DependencyMissing,
                        Some(dependency.id()),
                    ),
                }
            }
            for head in &snapshot.source_heads {
                if context.revisions.get(&head.revision_id) != Some(&head.source_id) {
                    evaluation.add(
                        FreshnessReasonCode::DependencyMissing,
                        Some(head.revision_id.as_str()),
                    );
                }
                match context.sources.get(&head.source_id) {
                    Some(source) => {
                        if source.current_revision_id != head.revision_id {
                            evaluation.add(
                                FreshnessReasonCode::DependencyChanged,
                                Some(head.source_id.as_str()),
                            );
                        }
                        if source.removed {
                            evaluation.add(
                                FreshnessReasonCode::SourceRemoved,
                                Some(head.source_id.as_str()),
                            );
                        }
                    }
                    None => evaluation.add(
                        FreshnessReasonCode::DependencyMissing,
                        Some(head.source_id.as_str()),
                    ),
                }
            }
        }
    }
    evaluation.finish(context)
}

#[derive(Default)]
struct Evaluation {
    reasons: BTreeMap<FreshnessReasonCode, BTreeSet<String>>,
    evidence: BTreeSet<EvidenceId>,
}

impl Evaluation {
    fn add(&mut self, code: FreshnessReasonCode, id: Option<&str>) {
        let dependencies = self.reasons.entry(code).or_default();
        if let Some(id) = id {
            dependencies.insert(id.to_owned());
        }
    }

    fn finish(mut self, context: &FreshnessContext) -> FreshnessReport {
        if !context.index_complete {
            self.add(FreshnessReasonCode::IndexIncomplete, None);
        }
        let evidence = std::mem::take(&mut self.evidence);
        let mut current_evidence = Vec::new();
        for id in &evidence {
            let Some(revision) = context.evidence.get(id) else {
                self.add(FreshnessReasonCode::DependencyMissing, Some(id.as_str()));
                continue;
            };
            let Some(source_id) = context.revisions.get(revision) else {
                self.add(
                    FreshnessReasonCode::DependencyMissing,
                    Some(revision.as_str()),
                );
                continue;
            };
            let Some(source) = context.sources.get(source_id) else {
                self.add(
                    FreshnessReasonCode::DependencyMissing,
                    Some(source_id.as_str()),
                );
                continue;
            };
            if &source.current_revision_id != revision {
                self.add(
                    FreshnessReasonCode::SourceRevisionBehind,
                    Some(revision.as_str()),
                );
            }
            if source.removed {
                self.add(FreshnessReasonCode::SourceRemoved, Some(source_id.as_str()));
            }
            if context.index_complete && !source.removed && &source.current_revision_id == revision
            {
                current_evidence.push(id.clone());
            }
        }
        let freshness = if self.reasons.keys().any(|code| {
            matches!(
                code,
                FreshnessReasonCode::DependencyMissing
                    | FreshnessReasonCode::IndexIncomplete
                    | FreshnessReasonCode::SnapshotMissing
            )
        }) {
            Freshness::Unknown
        } else if self.reasons.is_empty() {
            Freshness::Current
        } else {
            Freshness::NeedsReview
        };
        FreshnessReport {
            freshness,
            freshness_reasons: self
                .reasons
                .into_iter()
                .map(|(code, dependency_ids)| FreshnessReason {
                    code,
                    dependency_ids: dependency_ids.into_iter().collect(),
                })
                .collect(),
            evidence_ids: evidence.into_iter().collect(),
            current_evidence_ids: current_evidence,
        }
    }
}
