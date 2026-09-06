#[path = "../../../tests/support/mod.rs"]
mod support;

use std::fs;

use knowmesh_core::{
    application::proposal::{PreparedProposal, prepare, prepare_accepted},
    canonical::{node::NodeDocument, snapshot::CanonicalSnapshot, workspace::Workspace},
    domain::{
        NodeId, Timestamp,
        proposal::{
            Decision, DecisionChange, PatchOp, Proposal, ProposalInput, ProposalItem, ProposalKind,
            ReviewInput, ReviewPolicy,
        },
    },
};
use serde_json::json;

fn now() -> Timestamp {
    "2026-09-06T01:00:00Z".parse().unwrap()
}

fn build(workspace: &Workspace, items: Vec<ProposalItem>) -> PreparedProposal {
    let snapshot = CanonicalSnapshot::scan(workspace).unwrap();
    prepare(
        workspace,
        &ProposalInput {
            kind: ProposalKind::Manual,
            base_generation: 7,
            schema_hash: snapshot.schema_hash,
            source_revision_id: None,
            compiler_run_id: None,
            summary: "Proposed edits.".into(),
            items,
        },
        "author",
        now(),
    )
    .unwrap()
}

fn review(prepared: &PreparedProposal, decisions: &[Decision]) -> Proposal {
    prepared
        .proposal
        .review(
            &ReviewInput {
                expected_revision: prepared.proposal.revision,
                accept_all: false,
                decisions: prepared
                    .proposal
                    .items
                    .iter()
                    .zip(decisions)
                    .map(|(item, decision)| DecisionChange {
                        item_id: item.id.clone(),
                        decision: *decision,
                        reason: None,
                        human_verified: false,
                    })
                    .collect(),
            },
            &ReviewPolicy::default(),
            "reviewer",
            now(),
        )
        .unwrap()
}

#[test]
fn selected_preview_applies_only_accepted_items_and_preserves_review_identity() {
    let (_temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let node = &before.nodes[0];
    let prepared = build(
        &workspace,
        vec![
            ProposalItem::new(
                PatchOp::AddAlias,
                node.metadata.id.to_string(),
                json!({"alias":"Accepted alias"}),
            )
            .unwrap(),
            ProposalItem::new(
                PatchOp::UpdateNodeSummary,
                node.metadata.id.to_string(),
                json!({"summary":"Rejected summary"}),
            )
            .unwrap(),
        ],
    );
    let reviewed = review(&prepared, &[Decision::Accepted, Decision::Rejected]);
    let frozen = serde_json::to_value(&reviewed).unwrap();
    let selected = prepare_accepted(
        &workspace,
        &reviewed,
        reviewed.revision,
        7,
        &prepared.base_snapshot_sha256,
    )
    .unwrap();
    assert_eq!(selected.proposal_id(), &reviewed.id);
    assert_eq!(selected.proposal_revision(), reviewed.revision);
    assert_eq!(selected.base_snapshot_sha256(), before.content_sha256);
    let changed = selected
        .preview()
        .nodes()
        .iter()
        .find(|entry| entry.metadata.id == node.metadata.id)
        .unwrap();
    assert!(changed.metadata.aliases.contains(&"Accepted alias".into()));
    assert_eq!(changed.summary, node.summary);
    assert_eq!(selected.documents().len(), 1);
    assert_eq!(serde_json::to_value(&reviewed).unwrap(), frozen);
    assert_eq!(
        CanonicalSnapshot::scan(&workspace).unwrap().content_sha256,
        before.content_sha256
    );
    assert!(!workspace.index_path().unwrap().exists());
}

#[test]
fn rejecting_a_created_node_invalidates_a_previously_valid_dependent_relation() {
    let (_temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let mut node = before
        .nodes
        .iter()
        .find(|node| node.metadata.node_type == "Dataset")
        .unwrap()
        .metadata
        .clone();
    node.id = NodeId::new();
    let mut relation = before.relations[0].relation.assertion.clone();
    relation.id = knowmesh_core::domain::RelationId::new();
    relation.target_node_id = node.id.clone();
    let prepared = build(
        &workspace,
        vec![
            ProposalItem::new(
                PatchOp::CreateNode,
                node.id.to_string(),
                json!({"metadata":node,"summary":"A proposed dataset."}),
            )
            .unwrap(),
            ProposalItem::new(
                PatchOp::AddRelation,
                before.relations[0].relation.source_node_id.to_string(),
                json!({"relation":relation}),
            )
            .unwrap(),
        ],
    );
    let reviewed = review(&prepared, &[Decision::Rejected, Decision::Accepted]);
    assert_eq!(
        prepare_accepted(
            &workspace,
            &reviewed,
            reviewed.revision,
            7,
            &prepared.base_snapshot_sha256
        )
        .unwrap_err()
        .code,
        "PROPOSAL_ACCEPTED_ITEMS_INVALID"
    );
    let reviewed = review(&prepared, &[Decision::Accepted, Decision::Accepted]);
    assert_eq!(
        prepare_accepted(
            &workspace,
            &reviewed,
            reviewed.revision,
            7,
            &prepared.base_snapshot_sha256
        )
        .unwrap()
        .documents()
        .len(),
        2
    );
}

#[test]
fn stale_revision_generation_or_external_content_cannot_reuse_approval() {
    let (_temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let node = &before.nodes[0];
    let prepared = build(
        &workspace,
        vec![
            ProposalItem::new(
                PatchOp::AddAlias,
                node.metadata.id.to_string(),
                json!({"alias":"Approved alias"}),
            )
            .unwrap(),
        ],
    );
    assert_eq!(
        prepare_accepted(
            &workspace,
            &prepared.proposal,
            1,
            7,
            &prepared.base_snapshot_sha256
        )
        .unwrap_err()
        .code,
        "PROPOSAL_REVIEW_REQUIRED"
    );
    let reviewed = review(&prepared, &[Decision::Accepted]);
    assert_eq!(
        prepare_accepted(&workspace, &reviewed, 1, 7, &prepared.base_snapshot_sha256)
            .unwrap_err()
            .code,
        "PROPOSAL_REVISION_MISMATCH"
    );
    assert_eq!(
        prepare_accepted(
            &workspace,
            &reviewed,
            reviewed.revision,
            8,
            &prepared.base_snapshot_sha256
        )
        .unwrap_err()
        .code,
        "STALE_PROPOSAL"
    );
    assert_eq!(
        prepare_accepted(&workspace, &reviewed, reviewed.revision, 7, &"0".repeat(64))
            .unwrap_err()
            .code,
        "STALE_PROPOSAL"
    );
    let path = workspace.root.join(&node.canonical_path);
    let mut doc = NodeDocument::parse(&fs::read_to_string(&path).unwrap()).unwrap();
    doc.metadata.aliases.push("External edit".into());
    fs::write(path, doc.render().unwrap()).unwrap();
    assert_eq!(
        prepare_accepted(
            &workspace,
            &reviewed,
            reviewed.revision,
            7,
            &prepared.base_snapshot_sha256
        )
        .unwrap_err()
        .code,
        "STALE_PROPOSAL"
    );
}

#[test]
fn changed_hashes_and_nonbuilder_approvals_require_fresh_validation_and_review() {
    let (_temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let prepared = build(
        &workspace,
        vec![
            ProposalItem::new(
                PatchOp::AddAlias,
                before.nodes[0].metadata.id.to_string(),
                json!({"alias":"Approved alias"}),
            )
            .unwrap(),
        ],
    );
    let mut changed = review(&prepared, &[Decision::Accepted]);
    changed.items[0].payload = json!({"alias":"Unreviewed replacement"});
    assert_eq!(
        prepare_accepted(
            &workspace,
            &changed,
            changed.revision,
            7,
            &prepared.base_snapshot_sha256
        )
        .unwrap_err()
        .code,
        "PROPOSAL_REVIEW_STALE"
    );
    let mut unchecked = prepared.proposal.clone();
    unchecked.items[0].before_sha256 = None;
    let unchecked = unchecked
        .review(
            &ReviewInput {
                expected_revision: 1,
                accept_all: true,
                decisions: vec![],
            },
            &ReviewPolicy::default(),
            "reviewer",
            now(),
        )
        .unwrap();
    assert_eq!(
        prepare_accepted(
            &workspace,
            &unchecked,
            unchecked.revision,
            7,
            &prepared.base_snapshot_sha256
        )
        .unwrap_err()
        .code,
        "PROPOSAL_REVALIDATION_REQUIRED"
    );
}

#[test]
fn accepted_preview_uses_workspace_policy_instead_of_the_review_callers_policy() {
    for human_required in [false, true] {
        let (_temp, workspace) = support::fixture();
        let path = workspace.root.join("schemas/research.yaml");
        let mut pack =
            knowmesh_core::canonical::schema::SchemaPack::parse(&fs::read(&path).unwrap()).unwrap();
        pack.policies.review_mode = knowmesh_core::canonical::schema::ReviewMode::Strict;
        pack.policies.human_verification_required = human_required;
        fs::write(path, serde_yaml::to_string(&pack).unwrap()).unwrap();
        let before = CanonicalSnapshot::scan(&workspace).unwrap();
        let prepared = build(
            &workspace,
            vec![
                ProposalItem::new(
                    PatchOp::AddAlias,
                    before.nodes[0].metadata.id.to_string(),
                    json!({"alias":"Approved alias"}),
                )
                .unwrap(),
            ],
        );
        let bulk = prepared
            .proposal
            .review(
                &ReviewInput {
                    expected_revision: 1,
                    accept_all: true,
                    decisions: vec![],
                },
                &ReviewPolicy::default(),
                "reviewer",
                now(),
            )
            .unwrap();
        assert_eq!(
            prepare_accepted(
                &workspace,
                &bulk,
                bulk.revision,
                7,
                &prepared.base_snapshot_sha256
            )
            .unwrap_err()
            .code,
            "STRICT_REVIEW_REQUIRED"
        );
        let explicit = review(&prepared, &[Decision::Accepted]);
        let result = prepare_accepted(
            &workspace,
            &explicit,
            explicit.revision,
            7,
            &prepared.base_snapshot_sha256,
        );
        if human_required {
            assert_eq!(result.unwrap_err().code, "HUMAN_VERIFICATION_REQUIRED");
        } else {
            result.unwrap();
        }
    }
}
