use std::collections::{BTreeMap, BTreeSet};

use super::{error, issue, payload::Payload};
use crate::{
    application::evidence_verify::{EvidenceInput, EvidenceVerifier, VerificationOptions},
    canonical::{snapshot::CanonicalSnapshot, source::SourceLibrary, workspace::Workspace},
    domain::{AssertionDependency, Evidence, EvidenceId, SourceRevisionId, proposal::Proposal},
    ingest::BuiltinSourceParser,
    ports::SourceParser,
};

pub(super) fn verify(
    workspace: &Workspace,
    before: &CanonicalSnapshot,
    proposal: &mut Proposal,
    payloads: &[Option<Payload>],
) -> BTreeSet<SourceRevisionId> {
    let mut known: BTreeMap<EvidenceId, Evidence> = before
        .evidence
        .iter()
        .map(|entry| (entry.evidence.id.clone(), entry.evidence.clone()))
        .collect();
    let mut claims: BTreeMap<_, Vec<_>> = before
        .claims
        .iter()
        .map(|entry| {
            (
                entry.claim.assertion.id.clone(),
                entry
                    .claim
                    .assertion
                    .evidence
                    .iter()
                    .map(|evidence| evidence.id.clone())
                    .collect(),
            )
        })
        .collect();
    for (index, payload) in payloads.iter().enumerate() {
        let Some(payload) = payload else {
            continue;
        };
        for evidence in payload.evidence() {
            if let Err(error) = evidence.validate() {
                issue(&mut proposal.items[index], error);
                continue;
            }
            if let Some(previous) = known.get(&evidence.id) {
                if previous != evidence {
                    issue(
                        &mut proposal.items[index],
                        error(
                            "EVIDENCE_ID_CONFLICT",
                            "A supplied Evidence ID has conflicting content.",
                        ),
                    );
                }
            } else {
                known.insert(evidence.id.clone(), evidence.clone());
            }
        }
        if let Payload::AddClaim(value) = payload {
            claims.entry(value.claim.id.clone()).or_insert_with(|| {
                value
                    .claim
                    .evidence
                    .iter()
                    .map(|evidence| evidence.id.clone())
                    .collect()
            });
        }
    }
    let mut required =
        BTreeMap::<SourceRevisionId, BTreeMap<EvidenceId, (Evidence, BTreeSet<usize>)>>::new();
    for (index, payload) in payloads.iter().enumerate() {
        let Some(payload) = payload else {
            continue;
        };
        let mut ids: BTreeSet<_> = proposal.items[index].evidence_ids.iter().cloned().collect();
        ids.extend(
            payload
                .evidence()
                .iter()
                .map(|evidence| evidence.id.clone()),
        );
        if let Payload::CreateSynthesis(value) = payload {
            ids.extend(value.metadata.evidence_ids.iter().cloned());
        }
        if ids.len() > 1024 {
            issue(
                &mut proposal.items[index],
                error(
                    "EVIDENCE_LIMIT_EXCEEDED",
                    "An item may directly reference at most 1024 Evidence records.",
                ),
            );
            continue;
        }
        proposal.items[index].evidence_ids = ids.iter().cloned().collect();
        // Verify transitive dependencies without expanding the item's bounded metadata.
        match payload {
            Payload::CreateSynthesis(value) => {
                if let Some(snapshot) = &value.metadata.dependency_snapshot {
                    for dependency in &snapshot.assertions {
                        match dependency {
                            AssertionDependency::Claim { id, .. } => {
                                if let Some(evidence) = claims.get(id) {
                                    ids.extend(evidence.iter().cloned());
                                }
                            }
                            AssertionDependency::Relation { id, .. } => {
                                if let Some(relation) = before
                                    .relations
                                    .iter()
                                    .find(|entry| &entry.relation.assertion.id == id)
                                {
                                    ids.extend(
                                        relation
                                            .relation
                                            .assertion
                                            .evidence
                                            .iter()
                                            .map(|evidence| evidence.id.clone()),
                                    );
                                }
                            }
                        }
                    }
                }
            }
            Payload::RecordConflict(value) => {
                for id in &value.group.claim_ids {
                    if let Some(evidence) = claims.get(id) {
                        ids.extend(evidence.iter().cloned());
                    }
                }
            }
            Payload::AddEvidence(_) => {
                if let Ok(id) = proposal.items[index].target_id.parse()
                    && let Some(evidence) = claims.get(&id)
                {
                    ids.extend(evidence.iter().cloned());
                }
                for relation in &before.relations {
                    if relation.relation.assertion.id.as_str() == proposal.items[index].target_id {
                        ids.extend(
                            relation
                                .relation
                                .assertion
                                .evidence
                                .iter()
                                .map(|evidence| evidence.id.clone()),
                        );
                    }
                }
            }
            _ => {}
        }
        for id in ids {
            let Some(evidence) = known.get(&id) else {
                issue(
                    &mut proposal.items[index],
                    error(
                        "EVIDENCE_NOT_FOUND",
                        "A Proposal item references missing Evidence.",
                    ),
                );
                continue;
            };
            let entry = required
                .entry(evidence.source_revision_id.clone())
                .or_default()
                .entry(id)
                .or_insert_with(|| (evidence.clone(), BTreeSet::new()));
            entry.1.insert(index);
        }
    }
    let parser = BuiltinSourceParser::default();
    let verified_revisions = required.keys().cloned().collect();
    for (revision_id, entries) in required {
        let parsed: crate::error::AppResult<_> = (|| {
            let source = before
                .sources
                .iter()
                .find(|source| {
                    source
                        .manifest
                        .revisions
                        .iter()
                        .any(|revision| revision.id == revision_id)
                })
                .ok_or_else(|| {
                    error(
                        "SOURCE_REVISION_NOT_FOUND",
                        "Evidence refers to an unknown source revision.",
                    )
                })?;
            let revision = source
                .manifest
                .revisions
                .iter()
                .find(|revision| revision.id == revision_id)
                .expect("located revision");
            let bytes = SourceLibrary::new(workspace).content_at(
                &source.manifest_path,
                &source.manifest,
                &revision_id,
            )?;
            let parsed = parser.parse(revision, &bytes)?;
            Ok((revision, parsed))
        })();
        let (revision, parsed) = match parsed {
            Ok(value) => value,
            Err(error) => {
                for (_, indices) in entries.values() {
                    for index in indices {
                        issue(&mut proposal.items[*index], error.clone());
                    }
                }
                continue;
            }
        };
        let verifier = EvidenceVerifier::new(
            revision,
            &parsed,
            VerificationOptions {
                repair_window_chars: 0,
                ..Default::default()
            },
        );
        for (evidence, indices) in entries.values() {
            let result = verifier
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|verifier| {
                    verifier
                        .verify(&EvidenceInput {
                            source_revision_id: evidence.source_revision_id.clone(),
                            quote: evidence.quote.clone(),
                            locator: evidence.locator.clone(),
                            stance: evidence.stance,
                            extraction_method: evidence.extraction_method,
                            confidence: evidence.confidence,
                        })
                        .map(|_| ())
                });
            if let Err(error) = result {
                for index in indices {
                    issue(&mut proposal.items[*index], error.clone());
                }
            }
        }
    }
    verified_revisions
}
