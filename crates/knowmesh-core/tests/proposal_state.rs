use knowmesh_core::domain::{
    NodeId,
    proposal::{
        Decision, DecisionChange, PatchOp, Proposal, ProposalInput, ProposalIssue, ProposalItem,
        ProposalKind, ProposalRevision, ProposalState, ReviewInput, ReviewPolicy,
    },
    sha256,
};
use serde_json::json;

fn time() -> knowmesh_core::domain::Timestamp {
    "2026-09-06T00:00:00Z".parse().unwrap()
}

fn item(alias: &str) -> ProposalItem {
    ProposalItem::new(
        PatchOp::AddAlias,
        NodeId::new().to_string(),
        json!({"alias":alias}),
    )
    .unwrap()
}

fn proposal(items: Vec<ProposalItem>) -> Proposal {
    Proposal::new(
        ProposalInput {
            kind: ProposalKind::Manual,
            base_generation: 1,
            schema_hash: sha256(b"schema"),
            source_revision_id: None,
            compiler_run_id: None,
            summary: "Review proposed aliases.".into(),
            items,
        },
        "local-user",
        time(),
    )
    .unwrap()
}

fn accept(item: &ProposalItem) -> DecisionChange {
    DecisionChange {
        item_id: item.id.clone(),
        decision: Decision::Accepted,
        reason: None,
        human_verified: false,
    }
}

#[test]
fn patch_operations_are_a_closed_set_and_targets_use_typed_ids() {
    assert!(serde_json::from_value::<PatchOp>(json!("write_file")).is_err());
    assert!(serde_json::from_value::<PatchOp>(json!("execute_sql")).is_err());
    for name in [
        "create_node",
        "update_node_summary",
        "add_alias",
        "add_claim",
        "supersede_claim",
        "retract_claim",
        "add_relation",
        "supersede_relation",
        "retract_relation",
        "add_evidence",
        "record_claim_conflict",
        "create_synthesis",
        "update_source_metadata",
    ] {
        let operation: PatchOp = serde_json::from_value(json!(name)).unwrap();
        assert_eq!(serde_json::to_value(operation).unwrap(), name);
    }
    assert_eq!(
        ProposalItem::new(
            PatchOp::AddAlias,
            "../../outside".into(),
            json!({"alias":"A"})
        )
        .unwrap_err()
        .code,
        "INVALID_ID"
    );
}

#[test]
fn explicit_review_creates_a_new_revision_and_requires_all_items_to_be_decided() {
    let original = proposal(vec![item("A"), item("B")]);
    let reviewed = original
        .review(
            &ReviewInput {
                expected_revision: 1,
                accept_all: false,
                decisions: vec![accept(&original.items[0])],
            },
            &ReviewPolicy::default(),
            "reviewer",
            time(),
        )
        .unwrap();
    assert_eq!(original.revision, 1);
    assert_eq!(original.items[0].decision, Decision::Pending);
    assert_eq!(reviewed.revision, 2);
    assert_eq!(reviewed.state, ProposalState::Reviewing);
    assert_eq!(
        reviewed
            .require_approved(&ReviewPolicy::default())
            .unwrap_err()
            .code,
        "PROPOSAL_REVIEW_REQUIRED"
    );
    let approved = reviewed
        .review(
            &ReviewInput {
                expected_revision: 2,
                accept_all: false,
                decisions: vec![accept(&reviewed.items[1])],
            },
            &ReviewPolicy::default(),
            "reviewer",
            time(),
        )
        .unwrap();
    assert_eq!(approved.state, ProposalState::Approved);
    assert_eq!(
        approved
            .require_approved(&ReviewPolicy::default())
            .unwrap()
            .len(),
        2
    );
    approved.validate().unwrap();
}

#[test]
fn relaxed_bulk_accept_preserves_rejections_and_strict_policy_rejects_bulk_review() {
    let original = proposal(vec![item("A"), item("B")]);
    let mut rejection = accept(&original.items[1]);
    rejection.decision = Decision::Rejected;
    let reviewed = original
        .review(
            &ReviewInput {
                expected_revision: 1,
                accept_all: false,
                decisions: vec![rejection],
            },
            &ReviewPolicy::default(),
            "reviewer",
            time(),
        )
        .unwrap();
    let input = ReviewInput {
        expected_revision: 2,
        accept_all: true,
        decisions: vec![],
    };
    let strict = ReviewPolicy {
        strict: true,
        ..Default::default()
    };
    assert_eq!(
        reviewed
            .review(&input, &strict, "reviewer", time())
            .unwrap_err()
            .code,
        "STRICT_REVIEW_REQUIRED"
    );
    let approved = reviewed
        .review(&input, &ReviewPolicy::default(), "reviewer", time())
        .unwrap();
    assert_eq!(approved.items[0].decision, Decision::Accepted);
    assert_eq!(approved.items[1].decision, Decision::Rejected);
    assert_eq!(
        approved
            .require_approved(&ReviewPolicy::default())
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        approved.require_approved(&strict).unwrap_err().code,
        "STRICT_REVIEW_REQUIRED"
    );
}

#[test]
fn blocking_issues_and_missing_human_verification_cannot_be_accepted() {
    let mut blocked = item("B");
    blocked.issues.push(ProposalIssue {
        code: "INVALID_EVIDENCE".into(),
        message: "The quote cannot be located.".into(),
        blocking: true,
    });
    let original = proposal(vec![item("A"), blocked]);
    let input = ReviewInput {
        expected_revision: 1,
        accept_all: false,
        decisions: original.items.iter().map(accept).collect(),
    };
    assert_eq!(
        original
            .review(&input, &ReviewPolicy::default(), "reviewer", time())
            .unwrap_err()
            .code,
        "PROPOSAL_ITEM_BLOCKED"
    );
    assert!(
        original
            .items
            .iter()
            .all(|item| item.decision == Decision::Pending)
    );
    let original = proposal(vec![item("A")]);
    let policy = ReviewPolicy {
        human_verification_required: true,
        ..Default::default()
    };
    let mut input = ReviewInput {
        expected_revision: 1,
        accept_all: false,
        decisions: vec![accept(&original.items[0])],
    };
    assert_eq!(
        original
            .review(&input, &policy, "reviewer", time())
            .unwrap_err()
            .code,
        "HUMAN_VERIFICATION_REQUIRED"
    );
    input.decisions[0].human_verified = true;
    original
        .review(&input, &policy, "reviewer", time())
        .unwrap()
        .require_approved(&policy)
        .unwrap();
}

#[test]
fn edits_reset_changed_reviews_and_expected_revisions_prevent_lost_updates() {
    let original = proposal(vec![item("A"), item("B")]);
    let approved = original
        .review(
            &ReviewInput {
                expected_revision: 1,
                accept_all: true,
                decisions: vec![],
            },
            &ReviewPolicy::default(),
            "reviewer",
            time(),
        )
        .unwrap();
    let mut items = approved.items.clone();
    items[0].payload = json!({"alias":"Changed"});
    let revised = approved
        .revise(
            &ProposalRevision {
                expected_revision: 2,
                base_generation: 1,
                schema_hash: approved.schema_hash.clone(),
                summary: approved.summary.clone(),
                items,
            },
            "editor",
            time(),
        )
        .unwrap();
    assert_eq!(revised.revision, 3);
    assert_eq!(revised.items[0].decision, Decision::Pending);
    assert_eq!(revised.items[1].decision, Decision::Accepted);
    assert_eq!(approved.items[0].payload, json!({"alias":"A"}));
    assert_eq!(
        revised
            .review(
                &ReviewInput {
                    expected_revision: 2,
                    accept_all: true,
                    decisions: vec![]
                },
                &ReviewPolicy::default(),
                "reviewer",
                time()
            )
            .unwrap_err()
            .code,
        "PROPOSAL_REVISION_MISMATCH"
    );
    let rebased = revised
        .revise(
            &ProposalRevision {
                expected_revision: 3,
                base_generation: 2,
                schema_hash: revised.schema_hash.clone(),
                summary: revised.summary.clone(),
                items: revised.items.clone(),
            },
            "editor",
            time(),
        )
        .unwrap();
    assert!(
        rebased
            .items
            .iter()
            .all(|item| item.decision == Decision::Pending)
    );
}

#[test]
fn changing_approved_payload_without_revision_invalidates_its_review_hash() {
    let original = proposal(vec![item("A")]);
    let mut approved = original
        .review(
            &ReviewInput {
                expected_revision: 1,
                accept_all: true,
                decisions: vec![],
            },
            &ReviewPolicy::default(),
            "reviewer",
            time(),
        )
        .unwrap();
    approved.items[0].payload = json!({"alias":"Hidden replacement"});
    assert_eq!(
        approved.validate().unwrap_err().code,
        "PROPOSAL_REVIEW_STALE"
    );
}

#[test]
fn no_op_reviews_keep_revision_and_terminal_proposals_cannot_be_edited() {
    let original = proposal(vec![item("A")]);
    let approved = original
        .review(
            &ReviewInput {
                expected_revision: 1,
                accept_all: true,
                decisions: vec![],
            },
            &ReviewPolicy::default(),
            "reviewer",
            time(),
        )
        .unwrap();
    let unchanged = approved
        .review(
            &ReviewInput {
                expected_revision: 2,
                accept_all: true,
                decisions: vec![],
            },
            &ReviewPolicy::default(),
            "reviewer",
            time(),
        )
        .unwrap();
    assert_eq!(unchanged.revision, 2);
    let applied = approved.mark_applied(2, 5, time()).unwrap();
    assert_eq!(applied.state, ProposalState::Applied);
    assert_eq!(applied.applied_generation, Some(5));
    assert_eq!(
        applied
            .mark_applied(2, 99, time())
            .unwrap()
            .applied_generation,
        Some(5)
    );
    assert_eq!(
        applied
            .review(
                &ReviewInput {
                    expected_revision: applied.revision,
                    accept_all: true,
                    decisions: vec![]
                },
                &ReviewPolicy::default(),
                "reviewer",
                time()
            )
            .unwrap_err()
            .code,
        "PROPOSAL_FINALIZED"
    );
}
