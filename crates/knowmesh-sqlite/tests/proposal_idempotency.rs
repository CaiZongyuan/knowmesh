#[path = "../../../tests/support/mod.rs"]
mod support;

use knowmesh_core::{
    application::proposal::workflow::{self, CreateInput, MutationRequest, ReviewRequest},
    canonical::{snapshot::CanonicalSnapshot, workspace::Workspace},
    domain::{
        Timestamp,
        proposal::{PatchOp, ProposalInput, ProposalItem, ProposalKind, ReviewInput},
    },
    ports::{ProjectionStore, ProposalStore},
};
use knowmesh_sqlite::SqliteStore;
use serde_json::json;

fn now() -> Timestamp {
    "2026-09-07T01:00:00Z".parse().unwrap()
}

fn fixture() -> (tempfile::TempDir, Workspace, SqliteStore, CreateInput) {
    let (temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let mut store = SqliteStore::open(&workspace.index_path().unwrap()).unwrap();
    store
        .bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    store.reconcile(&snapshot).unwrap();
    let input = CreateInput {
        dry_run: false,
        proposal: ProposalInput {
            kind: ProposalKind::Manual,
            base_generation: 1,
            schema_hash: snapshot.schema_hash,
            source_revision_id: None,
            compiler_run_id: None,
            summary: "Idempotent Proposal.".into(),
            items: vec![
                ProposalItem::new(
                    PatchOp::AddAlias,
                    snapshot.nodes[0].metadata.id.to_string(),
                    json!({"alias":"Idempotent alias"}),
                )
                .unwrap(),
            ],
        },
    };
    (temp, workspace, store, input)
}

#[test]
fn same_request_returns_its_original_revision_after_later_reviews() {
    let (_temp, workspace, mut store, input) = fixture();
    let first = workflow::execute_request(
        &workspace,
        &mut store,
        MutationRequest::Create(&input),
        Some("create-1"),
        "author",
        now(),
    )
    .unwrap();
    let review = ReviewRequest {
        proposal_id: first.record.proposal.id.clone(),
        dry_run: false,
        review: ReviewInput {
            expected_revision: 1,
            accept_all: true,
            decisions: vec![],
        },
    };
    let approved = workflow::execute_request(
        &workspace,
        &mut store,
        MutationRequest::Review(&review),
        Some("review-1"),
        "reviewer",
        now(),
    )
    .unwrap();
    let replay = workflow::execute_request(
        &workspace,
        &mut store,
        MutationRequest::Create(&input),
        Some("create-1"),
        "retry",
        now(),
    )
    .unwrap();
    assert_eq!(replay.record, first.record);
    assert_eq!(
        store.proposal_get(&first.record.proposal.id, None).unwrap(),
        approved.record
    );
    let replay = workflow::execute_request(
        &workspace,
        &mut store,
        MutationRequest::Review(&review),
        Some("review-1"),
        "retry",
        now(),
    )
    .unwrap();
    assert_eq!(replay.record, approved.record);
    let db = rusqlite::Connection::open(workspace.index_path().unwrap()).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM proposals", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM proposal_revisions", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn changed_input_cannot_reuse_a_key_and_preview_does_not_reserve_it() {
    let (_temp, workspace, mut store, mut input) = fixture();
    input.dry_run = true;
    workflow::execute_request(
        &workspace,
        &mut store,
        MutationRequest::Create(&input),
        Some("new-key"),
        "author",
        now(),
    )
    .unwrap();
    let db = rusqlite::Connection::open(workspace.index_path().unwrap()).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM idempotency_keys", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    input.dry_run = false;
    workflow::execute_request(
        &workspace,
        &mut store,
        MutationRequest::Create(&input),
        Some("new-key"),
        "author",
        now(),
    )
    .unwrap();
    input.proposal.summary = "A different request.".into();
    assert_eq!(
        workflow::execute_request(
            &workspace,
            &mut store,
            MutationRequest::Create(&input),
            Some("new-key"),
            "author",
            now()
        )
        .unwrap_err()
        .code,
        "IDEMPOTENCY_KEY_REUSED"
    );
}

#[test]
fn failing_key_persistence_rolls_back_the_proposal_and_audit() {
    let (_temp, workspace, mut store, input) = fixture();
    let db = rusqlite::Connection::open(workspace.index_path().unwrap()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_key BEFORE INSERT ON idempotency_keys BEGIN SELECT RAISE(ABORT,'injected key failure'); END;").unwrap();
    assert!(
        workflow::execute_request(
            &workspace,
            &mut store,
            MutationRequest::Create(&input),
            Some("atomic-key"),
            "author",
            now()
        )
        .is_err()
    );
    for table in [
        "proposals",
        "proposal_revisions",
        "proposal_items",
        "audit_events",
        "idempotency_keys",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row
                .get::<_, u64>(0))
                .unwrap(),
            0
        );
    }
    db.execute_batch("DROP TRIGGER fail_key;").unwrap();
    workflow::execute_request(
        &workspace,
        &mut store,
        MutationRequest::Create(&input),
        Some("atomic-key"),
        "author",
        now(),
    )
    .unwrap();
}

#[test]
fn stale_review_side_effect_and_error_are_replayed_together() {
    let (_temp, workspace, mut store, input) = fixture();
    let created = workflow::create(&workspace, &mut store, &input, "author", now())
        .unwrap()
        .record;
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let path = workspace.root.join(&snapshot.nodes[0].canonical_path);
    let mut doc = knowmesh_core::canonical::node::NodeDocument::parse(
        &std::fs::read_to_string(&path).unwrap(),
    )
    .unwrap();
    doc.metadata.aliases.push("External change".into());
    std::fs::write(path, doc.render().unwrap()).unwrap();
    let review = ReviewRequest {
        proposal_id: created.proposal.id.clone(),
        dry_run: false,
        review: ReviewInput {
            expected_revision: 1,
            accept_all: true,
            decisions: vec![],
        },
    };
    for _ in 0..2 {
        assert_eq!(
            workflow::execute_request(
                &workspace,
                &mut store,
                MutationRequest::Review(&review),
                Some("stale-review"),
                "reviewer",
                now()
            )
            .unwrap_err()
            .code,
            "STALE_PROPOSAL"
        );
    }
    assert_eq!(
        store
            .proposal_get(&created.proposal.id, None)
            .unwrap()
            .proposal
            .revision,
        2
    );
}

#[test]
fn keys_are_scoped_by_operation_and_survive_atomic_rebuild() {
    use knowmesh_core::application::rebuild::{self, RebuildInput};
    let (_temp, workspace, mut store, input) = fixture();
    let created = workflow::execute_request(
        &workspace,
        &mut store,
        MutationRequest::Create(&input),
        Some("shared-name"),
        "author",
        now(),
    )
    .unwrap();
    let review = ReviewRequest {
        proposal_id: created.record.proposal.id.clone(),
        dry_run: false,
        review: ReviewInput {
            expected_revision: 1,
            accept_all: true,
            decisions: vec![],
        },
    };
    let approved = workflow::execute_request(
        &workspace,
        &mut store,
        MutationRequest::Review(&review),
        Some("shared-name"),
        "reviewer",
        now(),
    )
    .unwrap();
    drop(store);
    let backend = knowmesh_sqlite::SqliteRebuilder::new(&workspace).unwrap();
    let report = rebuild::execute(
        &workspace,
        &backend,
        &RebuildInput {
            yes: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(report.runtime_table_counts["idempotency_keys"], 2);
    let mut store = SqliteStore::open(&workspace.index_path().unwrap()).unwrap();
    assert_eq!(
        workflow::execute_request(
            &workspace,
            &mut store,
            MutationRequest::Create(&input),
            Some("shared-name"),
            "retry",
            now()
        )
        .unwrap()
        .record,
        created.record
    );
    assert_eq!(
        workflow::execute_request(
            &workspace,
            &mut store,
            MutationRequest::Review(&review),
            Some("shared-name"),
            "retry",
            now()
        )
        .unwrap()
        .record,
        approved.record
    );
}

#[test]
fn malformed_keys_and_corrupt_references_are_rejected_without_repeating_mutations() {
    let (_temp, workspace, mut store, input) = fixture();
    for key in [
        "".to_owned(),
        " \t".to_owned(),
        "key\nvalue".to_owned(),
        "k".repeat(257),
    ] {
        assert_eq!(
            workflow::execute_request(
                &workspace,
                &mut store,
                MutationRequest::Create(&input),
                Some(&key),
                "author",
                now()
            )
            .unwrap_err()
            .code,
            "INVALID_IDEMPOTENCY_KEY"
        );
    }
    workflow::execute_request(
        &workspace,
        &mut store,
        MutationRequest::Create(&input),
        Some("corrupt-key"),
        "author",
        now(),
    )
    .unwrap();
    let db = rusqlite::Connection::open(workspace.index_path().unwrap()).unwrap();
    db.execute(
        "UPDATE idempotency_keys SET response_json='{}' WHERE key='corrupt-key'",
        [],
    )
    .unwrap();
    assert_eq!(
        workflow::execute_request(
            &workspace,
            &mut store,
            MutationRequest::Create(&input),
            Some("corrupt-key"),
            "author",
            now()
        )
        .unwrap_err()
        .code,
        "IDEMPOTENCY_RESULT_INVALID"
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM proposals", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn cached_results_do_not_hide_nonfinite_typed_input() {
    let (_temp, workspace, mut store, mut input) = fixture();
    workflow::execute_request(&workspace, &mut store, MutationRequest::Create(&input), Some("finite-input"), "author", now()).unwrap();
    input.proposal.items[0].compiler_confidence = Some(f64::NAN);
    assert_eq!(workflow::execute_request(&workspace, &mut store, MutationRequest::Create(&input), Some("finite-input"), "author", now()).unwrap_err().code, "INVALID_PROPOSAL");
}
