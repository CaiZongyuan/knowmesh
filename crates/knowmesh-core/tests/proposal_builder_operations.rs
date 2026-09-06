#[path = "../../../tests/support/mod.rs"]
mod support;

use std::{collections::BTreeMap, fs};

use knowmesh_core::{
    application::proposal::{PreparedProposal, prepare},
    canonical::{node::NodeDocument, snapshot::CanonicalSnapshot, workspace::Workspace},
    domain::{
        AssertionDependency, ClaimId, ConflictGroup, ConflictGroupId, ConflictGroupStatus,
        EvidenceId, EvidenceStance, EvidenceStatus, LifecycleStatus, NodeId, RelationId, SourceId,
        SynthesisId, Timestamp,
        proposal::{PatchOp, ProposalInput, ProposalIssue, ProposalItem, ProposalKind},
    },
};
use serde_json::{Value, json};

fn now() -> Timestamp {
    "2026-09-06T01:00:00Z".parse().unwrap()
}

fn item(op: PatchOp, id: impl ToString, payload: Value) -> ProposalItem {
    ProposalItem::new(op, id.to_string(), payload).unwrap()
}

fn build(workspace: &Workspace, items: Vec<ProposalItem>) -> PreparedProposal {
    let snapshot = CanonicalSnapshot::scan(workspace).unwrap();
    prepare(
        workspace,
        &ProposalInput {
            kind: ProposalKind::Manual,
            base_generation: 1,
            schema_hash: snapshot.schema_hash,
            source_revision_id: None,
            compiler_run_id: None,
            summary: "Review proposed canonical edits.".into(),
            items,
        },
        "local-user",
        now(),
    )
    .unwrap()
}

fn unblocked(prepared: &PreparedProposal) {
    for item in &prepared.proposal.items {
        assert!(
            !item.issues.iter().any(|issue| issue.blocking),
            "{:?}: {:?}",
            item.op,
            item.issues
        );
    }
    assert!(prepared.preview.is_some());
}

fn blocked(prepared: &PreparedProposal, index: usize, code: &str) {
    assert!(
        prepared.proposal.items[index]
            .issues
            .iter()
            .any(|issue| issue.blocking && issue.code == code),
        "{:?}",
        prepared.proposal.items[index].issues
    );
}

#[test]
fn new_nodes_are_created_before_relations_and_source_metadata_dependencies() {
    let (_temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let model = before
        .nodes
        .iter()
        .find(|node| node.metadata.node_type == "Model")
        .unwrap();
    let mut node = before
        .nodes
        .iter()
        .find(|node| node.metadata.node_type == "Dataset")
        .unwrap()
        .metadata
        .clone();
    node.id = NodeId::new();
    node.name = "Dataset [C] <script>\n## Notes".into();
    let mut relation = before.relations[0].relation.assertion.clone();
    relation.id = RelationId::new();
    relation.target_node_id = node.id.clone();
    let source = &before.sources[0].manifest;
    let prepared = build(
        &workspace,
        vec![
            item(
                PatchOp::AddRelation,
                &model.metadata.id,
                json!({"relation":relation}),
            ),
            item(
                PatchOp::UpdateSourceMetadata,
                &source.id,
                json!({
                    "title":"Updated source description", "kind":source.kind,
                    "authors":source.authors, "identifiers":source.identifiers,
                    "language":source.language, "tags":["updated"], "represented_nodes":[node.id]
                }),
            ),
            item(
                PatchOp::CreateNode,
                &node.id,
                json!({"metadata":node, "summary":"A new dataset."}),
            ),
        ],
    );
    unblocked(&prepared);
    let after = prepared.preview.as_ref().unwrap();
    assert_eq!(after.nodes().len(), 3);
    assert_eq!(after.relations().len(), 2);
    assert_eq!(after.sources()[0].manifest.revisions, source.revisions);
    assert_eq!(
        after.sources()[0].manifest.current_revision_id,
        source.current_revision_id
    );
    assert_eq!(after.sources()[0].manifest.storage, source.storage);
    assert_eq!(
        after.sources()[0].manifest.represented_nodes,
        vec![node.id.clone()]
    );
    let created = after
        .nodes()
        .iter()
        .find(|entry| entry.metadata.id == node.id)
        .unwrap();
    assert_eq!(created.summary, "A new dataset.");
    let rendered = std::str::from_utf8(&prepared.documents()[&created.canonical_path]).unwrap();
    let document = NodeDocument::parse(rendered).unwrap();
    let mut headings = Vec::new();
    let mut heading = None;
    for event in pulldown_cmark::Parser::new(document.body()) {
        match event {
            pulldown_cmark::Event::Start(pulldown_cmark::Tag::Heading { level, .. }) => {
                heading = Some((level, String::new()))
            }
            pulldown_cmark::Event::Text(text) => {
                if let Some((_, value)) = &mut heading {
                    value.push_str(&text);
                }
            }
            pulldown_cmark::Event::End(pulldown_cmark::TagEnd::Heading(_)) => {
                headings.push(heading.take().unwrap())
            }
            pulldown_cmark::Event::InlineHtml(_) => panic!("Node title injected HTML"),
            _ => {}
        }
    }
    assert_eq!(
        headings,
        vec![
            (
                pulldown_cmark::HeadingLevel::H1,
                "Dataset [C] <script> ## Notes".into()
            ),
            (pulldown_cmark::HeadingLevel::H2, "Summary".into()),
        ]
    );
    assert_eq!(
        CanonicalSnapshot::scan(&workspace).unwrap().content_sha256,
        before.content_sha256
    );
    assert!(!workspace.index_path().unwrap().exists());
}

#[test]
fn lifecycle_edits_preserve_old_assertions_and_accept_new_replacements() {
    let (_temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let claim = &before.claims[0].claim;
    let relation = &before.relations[0].relation;
    let mut replacement_claim = claim.assertion.clone();
    replacement_claim.id = ClaimId::new();
    replacement_claim.statement = "Replacement claim with independently retained evidence.".into();
    let mut replacement_relation = relation.assertion.clone();
    replacement_relation.id = RelationId::new();
    replacement_relation
        .qualifiers
        .insert("scope".into(), json!("new evaluation"));
    let mut retract_claim = replacement_claim.clone();
    retract_claim.id = ClaimId::new();
    let mut retract_relation = replacement_relation.clone();
    retract_relation.id = RelationId::new();
    let prepared = build(
        &workspace,
        vec![
            item(
                PatchOp::SupersedeClaim,
                &claim.assertion.id,
                json!({"replacement_id":replacement_claim.id}),
            ),
            item(
                PatchOp::SupersedeRelation,
                &relation.assertion.id,
                json!({"replacement_id":replacement_relation.id}),
            ),
            item(
                PatchOp::RetractClaim,
                &retract_claim.id,
                json!({"reason":"Withdraw the candidate."}),
            ),
            item(
                PatchOp::RetractRelation,
                &retract_relation.id,
                json!({"reason":"Withdraw the candidate."}),
            ),
            item(
                PatchOp::AddClaim,
                &claim.subject_node_id,
                json!({"claim":replacement_claim}),
            ),
            item(
                PatchOp::AddRelation,
                &relation.source_node_id,
                json!({"relation":replacement_relation}),
            ),
            item(
                PatchOp::AddClaim,
                &claim.subject_node_id,
                json!({"claim":retract_claim}),
            ),
            item(
                PatchOp::AddRelation,
                &relation.source_node_id,
                json!({"relation":retract_relation}),
            ),
        ],
    );
    unblocked(&prepared);
    let after = prepared.preview.as_ref().unwrap();
    let old = after
        .claims()
        .iter()
        .find(|entry| entry.claim.assertion.id == claim.assertion.id)
        .unwrap();
    let mut expected = claim.assertion.clone();
    expected.lifecycle_status = LifecycleStatus::Superseded;
    assert_eq!(old.claim.assertion, expected);
    let old = after
        .relations()
        .iter()
        .find(|entry| entry.relation.assertion.id == relation.assertion.id)
        .unwrap();
    let mut expected = relation.assertion.clone();
    expected.lifecycle_status = LifecycleStatus::Superseded;
    assert_eq!(old.relation.assertion, expected);
    assert_eq!(
        after
            .claims()
            .iter()
            .find(|entry| entry.claim.assertion.id == retract_claim.id)
            .unwrap()
            .claim
            .assertion
            .lifecycle_status,
        LifecycleStatus::Retracted
    );
    assert_eq!(
        after
            .relations()
            .iter()
            .find(|entry| entry.relation.assertion.id == retract_relation.id)
            .unwrap()
            .relation
            .assertion
            .lifecycle_status,
        LifecycleStatus::Retracted
    );
}

#[test]
fn evidence_append_preserves_identity_and_marks_opposed_stances_conflicting() {
    let (_temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let original = &before.evidence[0].evidence;
    let mut additional = original.clone();
    additional.id = EvidenceId::new();
    additional.stance = EvidenceStance::Contradicts;
    let prepared = build(
        &workspace,
        vec![
            item(
                PatchOp::AddEvidence,
                &before.claims[0].claim.assertion.id,
                json!({"evidence":[original, additional]}),
            ),
            item(
                PatchOp::AddEvidence,
                &before.relations[0].relation.assertion.id,
                json!({"evidence":[original, additional]}),
            ),
        ],
    );
    unblocked(&prepared);
    let after = prepared.preview.as_ref().unwrap();
    assert_eq!(
        after.claims()[0].claim.assertion.evidence,
        vec![original.clone(), additional.clone()]
    );
    assert_eq!(
        after.relations()[0].relation.assertion.evidence,
        vec![original.clone(), additional]
    );
    assert_eq!(
        after.claims()[0].claim.assertion.evidence_status,
        EvidenceStatus::Conflicting
    );
    assert_eq!(
        after.relations()[0].relation.assertion.evidence_status,
        EvidenceStatus::Conflicting
    );
}

#[test]
fn conflict_groups_depend_on_new_claims_without_hiding_group_copies_in_them() {
    let (_temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let claim = &before.claims[0].claim;
    let mut other = claim.assertion.clone();
    other.id = ClaimId::new();
    other.statement = "Model A was not evaluated on Dataset B.".into();
    let mut claim_ids = vec![claim.assertion.id.clone(), other.id.clone()];
    claim_ids.sort();
    let group = ConflictGroup {
        id: ConflictGroupId::new(),
        claim_ids,
        reason: "Conflicting evaluation statements.".into(),
        status: ConflictGroupStatus::Open,
        created_at: now(),
        resolved_at: None,
    };
    let group_item = item(
        PatchOp::RecordClaimConflict,
        &claim.subject_node_id,
        json!({"group":group}),
    );
    let prepared = build(
        &workspace,
        vec![
            group_item.clone(),
            item(
                PatchOp::AddClaim,
                &claim.subject_node_id,
                json!({"claim":other}),
            ),
        ],
    );
    unblocked(&prepared);
    assert!(
        prepared.proposal.items[1].payload["claim"]
            .get("conflict_groups")
            .is_none()
    );
    for claim in prepared.preview.as_ref().unwrap().claims() {
        assert_eq!(claim.claim.assertion.conflict_groups, vec![group.clone()]);
        assert_eq!(
            claim.claim.assertion.evidence_status,
            EvidenceStatus::Conflicting
        );
    }
    let missing = build(&workspace, vec![group_item]);
    blocked(&missing, 0, "CONFLICT_CLAIM_MISSING");
    assert!(missing.documents().is_empty());
}

#[test]
fn synthesis_preserves_supplied_historical_dependency_hashes_and_source_heads() {
    let (_temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let mut metadata = before.syntheses[0].metadata.clone();
    metadata.id = SynthesisId::new();
    let AssertionDependency::Claim {
        semantic_sha256, ..
    } = &mut metadata.dependency_snapshot.as_mut().unwrap().assertions[0]
    else {
        unreachable!()
    };
    *semantic_sha256 = "a".repeat(64);
    let frozen = metadata.dependency_snapshot.clone();
    let prepared = build(
        &workspace,
        vec![item(
            PatchOp::CreateSynthesis,
            &metadata.id,
            json!({
                "metadata":metadata, "body":format!("# New synthesis\n\nA cited answer. [@{}]", metadata.evidence_ids[0])
            }),
        )],
    );
    unblocked(&prepared);
    assert_eq!(
        prepared.proposal.items[0].payload["metadata"]["dependency_snapshot"],
        serde_json::to_value(&frozen).unwrap()
    );
    assert_eq!(
        prepared
            .preview
            .as_ref()
            .unwrap()
            .syntheses()
            .iter()
            .find(|entry| entry.metadata.id == metadata.id)
            .unwrap()
            .metadata
            .dependency_snapshot,
        frozen
    );
}

#[test]
fn synthesis_requires_available_schema_and_complete_referentially_valid_snapshots() {
    let (_temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    for (variant, code) in [
        (0, "SYNTHESIS_SNAPSHOT_REQUIRED"),
        (1, "SCHEMA_PACK_NOT_FOUND"),
        (2, "SYNTHESIS_DEPENDENCY_NOT_FOUND"),
        (3, "SYNTHESIS_SOURCE_HEAD_INVALID"),
        (4, "SYNTHESIS_SOURCE_HEAD_MISSING"),
    ] {
        let mut metadata = before.syntheses[0].metadata.clone();
        metadata.id = SynthesisId::new();
        match variant {
            0 => metadata.dependency_snapshot = None,
            1 => metadata.schema = "absent@1".into(),
            2 => {
                metadata.dependency_snapshot.as_mut().unwrap().assertions[0] =
                    AssertionDependency::Claim {
                        id: ClaimId::new(),
                        semantic_sha256: "a".repeat(64),
                    }
            }
            3 => {
                metadata.dependency_snapshot.as_mut().unwrap().source_heads[0].source_id =
                    SourceId::new()
            }
            4 => metadata
                .dependency_snapshot
                .as_mut()
                .unwrap()
                .source_heads
                .clear(),
            _ => unreachable!(),
        }
        let prepared = build(
            &workspace,
            vec![item(
                PatchOp::CreateSynthesis,
                &metadata.id,
                json!({
                    "metadata":metadata, "body":format!("A cited answer. [@{}]", metadata.evidence_ids[0])
                }),
            )],
        );
        blocked(&prepared, 0, code);
        assert!(prepared.documents().is_empty());
    }
}

#[test]
fn evidence_overflow_returns_a_blocked_item_without_exceeding_review_metadata_limits() {
    let (_temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let evidence: Vec<_> = (0..1025)
        .map(|_| {
            let mut evidence = before.evidence[0].evidence.clone();
            evidence.id = EvidenceId::new();
            evidence
        })
        .collect();
    let prepared = build(
        &workspace,
        vec![item(
            PatchOp::AddEvidence,
            &before.claims[0].claim.assertion.id,
            json!({"evidence":evidence}),
        )],
    );
    blocked(&prepared, 0, "EVIDENCE_LIMIT_EXCEEDED");
    prepared.proposal.validate().unwrap();
    assert!(prepared.documents().is_empty());
    assert_eq!(
        prepared.proposal.items[0].payload["evidence"]
            .as_array()
            .unwrap()
            .len(),
        1025
    );
}

#[test]
fn full_warning_lists_still_allow_builder_to_block_invalid_payloads() {
    let (_temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let mut invalid = item(
        PatchOp::AddAlias,
        &before.nodes[0].metadata.id,
        json!({"unknown":"field"}),
    );
    invalid.issues = (0..128)
        .map(|index| ProposalIssue {
            code: format!("UPSTREAM_WARNING_{index}"),
            message: "An upstream warning.".into(),
            blocking: false,
        })
        .collect();
    let prepared = build(&workspace, vec![invalid]);
    assert!(
        prepared.proposal.items[0]
            .issues
            .iter()
            .any(|issue| issue.blocking)
    );
    prepared.proposal.validate().unwrap();
    assert!(prepared.documents().is_empty());
}

#[test]
fn source_metadata_cannot_modify_revision_storage_or_unrelated_identity_fields() {
    let (_temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let source = &before.sources[0].manifest;
    for field in [
        "revisions",
        "current_revision_id",
        "removed_at",
        "storage",
        "id",
        "path",
    ] {
        let mut payload = json!({"title":source.title, "kind":source.kind, "authors":source.authors,
            "identifiers":source.identifiers, "language":source.language, "tags":source.tags,
            "represented_nodes":source.represented_nodes});
        payload[field] = json!("untrusted replacement");
        let prepared = build(
            &workspace,
            vec![item(PatchOp::UpdateSourceMetadata, &source.id, payload)],
        );
        blocked(&prepared, 0, "INVALID_PROPOSAL_PAYLOAD");
        assert!(prepared.documents().is_empty());
    }
}

#[test]
fn conflict_group_evidence_is_verified_without_expanding_item_reference_metadata() {
    let (_temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let owner = &before.claims[0].claim.subject_node_id;
    let path = &before.claims[0].canonical_path;
    let mut doc =
        NodeDocument::parse(&fs::read_to_string(workspace.root.join(path)).unwrap()).unwrap();
    let mut other = doc.claims[0].clone();
    other.id = ClaimId::new();
    other.statement = "A second assertion.".into();
    doc.claims.push(other);
    for claim in &mut doc.claims {
        claim.evidence = (0..513)
            .map(|_| {
                let mut evidence = before.evidence[0].evidence.clone();
                evidence.id = EvidenceId::new();
                evidence
            })
            .collect();
    }
    fs::write(workspace.root.join(path), doc.render().unwrap()).unwrap();
    let mut claim_ids: Vec<_> = doc.claims.iter().map(|claim| claim.id.clone()).collect();
    claim_ids.sort();
    let group = ConflictGroup {
        id: ConflictGroupId::new(),
        claim_ids,
        reason: "Conflicting statements.".into(),
        status: ConflictGroupStatus::Open,
        created_at: now(),
        resolved_at: None,
    };
    let prepared = build(
        &workspace,
        vec![item(
            PatchOp::RecordClaimConflict,
            owner,
            json!({"group":group, "member_statuses": BTreeMap::<ClaimId,EvidenceStatus>::new()}),
        )],
    );
    unblocked(&prepared);
    assert!(prepared.proposal.items[0].evidence_ids.is_empty());
    assert_eq!(prepared.preview.as_ref().unwrap().evidence().len(), 1027);
}

#[test]
fn conflict_resolution_preserves_members_and_prevents_rewriting_closed_history() {
    let (_temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let owner = &before.claims[0].claim.subject_node_id;
    let path = &before.claims[0].canonical_path;
    let mut doc = NodeDocument::parse(&fs::read_to_string(workspace.root.join(path)).unwrap()).unwrap();
    let mut other = doc.claims[0].clone();
    other.id = ClaimId::new();
    other.statement = "A conflicting statement.".into();
    doc.claims.push(other);
    let mut claim_ids: Vec<_> = doc.claims.iter().map(|claim| claim.id.clone()).collect();
    claim_ids.sort();
    let group = ConflictGroup {
        id: ConflictGroupId::new(), claim_ids,
        reason: "The initial conflict review.".into(), status: ConflictGroupStatus::Open,
        created_at: before.nodes[0].metadata.created_at, resolved_at: None,
    };
    for claim in &mut doc.claims {
        claim.conflict_groups = vec![group.clone()];
        claim.evidence_status = EvidenceStatus::Conflicting;
    }
    fs::write(workspace.root.join(path), doc.render().unwrap()).unwrap();
    let mut resolved = group.clone();
    resolved.status = ConflictGroupStatus::Resolved;
    resolved.resolved_at = Some(now());
    let statuses: BTreeMap<_, _> = group.claim_ids.iter().cloned().map(|id| (id, EvidenceStatus::Uncertain)).collect();
    let prepared = build(&workspace, vec![item(PatchOp::RecordClaimConflict, owner, json!({"group":resolved, "member_statuses":statuses}))]);
    unblocked(&prepared);
    for claim in prepared.preview.as_ref().unwrap().claims() {
        assert_eq!(claim.claim.assertion.conflict_groups, vec![resolved.clone()]);
        assert_eq!(claim.claim.assertion.evidence_status, EvidenceStatus::Uncertain);
    }
    for (path, bytes) in prepared.documents() { fs::write(workspace.root.join(path), bytes).unwrap(); }
    let reopened = build(&workspace, vec![item(PatchOp::RecordClaimConflict, owner, json!({"group":group}))]);
    blocked(&reopened, 0, "CONFLICT_HISTORY_IMMUTABLE");
    assert!(reopened.documents().is_empty());
    resolved.created_at = now();
    let changed = build(&workspace, vec![item(PatchOp::RecordClaimConflict, owner, json!({"group":resolved}))]);
    blocked(&changed, 0, "CONFLICT_HISTORY_IMMUTABLE");
    assert!(changed.documents().is_empty());
}

#[test]
fn schema_and_global_identity_errors_block_only_the_invalid_local_operations() {
    let (_temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let owner = &before.claims[0].claim.subject_node_id;
    let mut relation = before.relations[0].relation.assertion.clone();
    relation.id = RelationId::new();
    relation.directed = !relation.directed;
    let mut node = before.nodes[0].metadata.clone();
    node.id = NodeId::new();
    node.schema = "absent@1".into();
    let mut evidence = before.evidence[0].evidence.clone();
    evidence.confidence = 0.5;
    let prepared = build(&workspace, vec![
        item(PatchOp::AddRelation, owner, json!({"relation":relation})),
        item(PatchOp::CreateNode, &node.id, json!({"metadata":node,"summary":"New summary."})),
        item(PatchOp::AddEvidence, &before.claims[0].claim.assertion.id, json!({"evidence":[evidence]})),
        item(PatchOp::AddClaim, owner, json!({"claim":before.claims[0].claim.assertion})),
        item(PatchOp::AddAlias, owner, json!({"alias":"Valid independent alias"})),
    ]);
    for (index, code) in ["RELATION_DIRECTION_MISMATCH", "SCHEMA_PACK_NOT_FOUND", "EVIDENCE_ID_CONFLICT", "CLAIM_ALREADY_EXISTS"].into_iter().enumerate() {
        blocked(&prepared, index, code);
    }
    assert!(prepared.proposal.items[4].issues.is_empty());
    assert_eq!(prepared.documents().len(), 1);
    assert_eq!(prepared.preview.as_ref().unwrap().claims().len(), 1);
}

#[test]
fn builder_verifies_existing_evidence_locations_without_rebinding_or_offset_repair() {
    let (_temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let mut evidence = before.evidence[0].evidence.clone();
    evidence.id = EvidenceId::new();
    evidence.locator.char_start = Some(0);
    evidence.locator.char_end = Some(evidence.quote.chars().count());
    let prepared = build(&workspace, vec![item(PatchOp::AddEvidence, &before.claims[0].claim.assertion.id, json!({"evidence":[evidence]}))]);
    assert!(prepared.proposal.items[0].issues.iter().any(|issue| issue.blocking));
    assert!(prepared.documents().is_empty());
    assert_eq!(prepared.proposal.items[0].payload["evidence"][0], serde_json::to_value(evidence).unwrap());
}

#[test]
fn compiler_proposals_require_current_schema_source_context_and_assertion_evidence() {
    let (_temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let mut claim = before.claims[0].claim.assertion.clone();
    claim.id = ClaimId::new();
    claim.statement = "An unreviewed manual statement.".into();
    claim.evidence.clear();
    claim.evidence_status = EvidenceStatus::Unreviewed;
    let items = vec![item(PatchOp::AddClaim, &before.claims[0].claim.subject_node_id, json!({"claim":claim}))];
    unblocked(&build(&workspace, items.clone()));
    let mut input = ProposalInput {
        kind: ProposalKind::Compile, base_generation:1, schema_hash:before.schema_hash,
        source_revision_id: None, compiler_run_id:None, summary:"Compiler candidates.".into(), items,
    };
    assert_eq!(prepare(&workspace, &input, "compiler", now()).unwrap_err().code, "PROPOSAL_SOURCE_REQUIRED");
    input.source_revision_id = Some(knowmesh_core::domain::SourceRevisionId::new());
    assert_eq!(prepare(&workspace, &input, "compiler", now()).unwrap_err().code, "SOURCE_REVISION_NOT_FOUND");
    input.source_revision_id = Some(before.sources[0].manifest.current_revision_id.clone());
    let prepared = prepare(&workspace, &input, "compiler", now()).unwrap();
    blocked(&prepared, 0, "EVIDENCE_REQUIRED");
    input.schema_hash = "0".repeat(64);
    assert_eq!(prepare(&workspace, &input, "compiler", now()).unwrap_err().code, "PROPOSAL_SCHEMA_MISMATCH");
}
