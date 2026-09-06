#[path = "../../../tests/support/mod.rs"]
mod support;

use std::fs;

use knowmesh_core::{
    application::{
        proposal::{
            ProposalRecord,
            apply::{self, ApplyInput},
            prepare,
        },
        sync,
    },
    canonical::{node::NodeDocument, snapshot::CanonicalSnapshot, workspace::Workspace},
    domain::{
        Timestamp,
        proposal::{
            PatchOp, ProposalInput, ProposalItem, ProposalKind, ProposalState, ReviewInput,
            ReviewPolicy,
        },
    },
    ports::{ProjectionStore, ProposalStore},
};
use knowmesh_sqlite::SqliteStore;
use rusqlite::Connection;
use serde_json::json;

fn now() -> Timestamp {
    "2026-09-06T02:00:00Z".parse().unwrap()
}

fn fixture() -> (
    tempfile::TempDir,
    Workspace,
    SqliteStore,
    ProposalRecord,
    ApplyInput,
) {
    let (temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let mut store = SqliteStore::open(&workspace.index_path().unwrap()).unwrap();
    store
        .bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    let generation = store.reconcile(&snapshot).unwrap().generation;
    let prepared = prepare(
        &workspace,
        &ProposalInput {
            kind: ProposalKind::Manual,
            base_generation: generation,
            schema_hash: snapshot.schema_hash,
            source_revision_id: None,
            compiler_run_id: None,
            summary: "Add two reviewed aliases.".into(),
            items: snapshot
                .nodes
                .iter()
                .map(|node| {
                    ProposalItem::new(
                        PatchOp::AddAlias,
                        node.metadata.id.to_string(),
                        json!({"alias":format!("Reviewed {}",node.metadata.name)}),
                    )
                    .unwrap()
                })
                .collect(),
        },
        "author",
        now(),
    )
    .unwrap();
    let mut record = ProposalRecord {
        proposal: prepared.proposal,
        base_snapshot_sha256: prepared.base_snapshot_sha256,
    };
    store.proposal_create(&record).unwrap();
    record.proposal = record
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
    store.proposal_save(1, &record).unwrap();
    let input = ApplyInput {
        proposal_id: record.proposal.id.clone(),
        expected_revision: 2,
        dry_run: false,
        yes: true,
    };
    (temp, workspace, store, record, input)
}

#[test]
fn apply_commits_canonical_index_review_history_and_a_repeatable_receipt() {
    let (_temp, workspace, mut store, approved, input) = fixture();
    let report = apply::execute(&workspace, &mut store, &input, "author", now()).unwrap();
    assert!(!report.dry_run);
    assert_eq!(report.changed_paths.len(), 2);
    assert!(report.transaction_id.is_some());
    assert_eq!(report.projection.as_ref().unwrap().generation, 2);
    assert_eq!(report.applied_revision, Some(3));
    let current = store.proposal_get(&input.proposal_id, None).unwrap();
    assert_eq!(current.proposal.state, ProposalState::Applied);
    assert_eq!(current.proposal.applied_generation, Some(2));
    assert_eq!(
        store.proposal_get(&input.proposal_id, Some(2)).unwrap(),
        approved
    );
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    assert!(snapshot.nodes.iter().all(|node| {
        node.metadata
            .aliases
            .contains(&format!("Reviewed {}", node.metadata.name))
    }));
    assert!(!sync::recovery_status(&workspace).unwrap().recovery_required);
    let repeated = apply::execute(&workspace, &mut store, &input, "retry", now()).unwrap();
    assert_eq!(
        serde_json::to_value(&repeated).unwrap(),
        serde_json::to_value(&report).unwrap()
    );
    let db = Connection::open(workspace.index_path().unwrap()).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM proposal_revisions", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM proposal_applications", [], |row| row
            .get::<_, u64>(
            0
        ))
        .unwrap(),
        1
    );
}

#[test]
fn apply_preview_and_missing_confirmation_do_not_mutate_runtime_or_canonical() {
    let (_temp, workspace, mut store, approved, mut input) = fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    input.yes = false;
    assert_eq!(
        apply::execute(&workspace, &mut store, &input, "author", now())
            .unwrap_err()
            .code,
        "CONFIRMATION_REQUIRED"
    );
    input.dry_run = true;
    let report = apply::execute(&workspace, &mut store, &input, "author", now()).unwrap();
    assert!(report.dry_run);
    assert_eq!(report.changed_paths.len(), 2);
    assert!(report.transaction_id.is_none());
    assert!(report.applied_revision.is_none());
    assert_eq!(
        store.proposal_get(&input.proposal_id, None).unwrap(),
        approved
    );
    assert_eq!(
        CanonicalSnapshot::scan(&workspace).unwrap().content_sha256,
        before.content_sha256
    );
    assert!(
        store
            .proposal_application(&input.proposal_id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn stale_content_and_wrong_revisions_fail_before_any_file_journal() {
    let (_temp, workspace, mut store, _approved, mut input) = fixture();
    input.expected_revision = 1;
    assert_eq!(
        apply::execute(&workspace, &mut store, &input, "author", now())
            .unwrap_err()
            .code,
        "PROPOSAL_REVISION_MISMATCH"
    );
    input.expected_revision = 2;
    let path = workspace.root.join("knowledge/nodes/model-a.md");
    let mut doc = NodeDocument::parse(&fs::read_to_string(&path).unwrap()).unwrap();
    doc.metadata.aliases.push("External edit".into());
    fs::write(path, doc.render().unwrap()).unwrap();
    assert_eq!(
        apply::execute(&workspace, &mut store, &input, "author", now())
            .unwrap_err()
            .code,
        "STALE_PROPOSAL"
    );
    assert!(!sync::recovery_status(&workspace).unwrap().recovery_required);
    assert!(
        store
            .proposal_application(&input.proposal_id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn failure_after_file_changes_rolls_back_sql_and_recovers_the_reviewed_revision() {
    let (_temp, workspace, mut store, approved, input) = fixture();
    let db = Connection::open(workspace.index_path().unwrap()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_apply_history BEFORE INSERT ON proposal_revisions WHEN NEW.revision=3 BEGIN SELECT RAISE(ABORT,'injected apply failure'); END;").unwrap();
    assert!(apply::execute(&workspace, &mut store, &input, "author", now()).is_err());
    assert!(sync::recovery_status(&workspace).unwrap().recovery_required);
    assert_eq!(
        store.proposal_get(&input.proposal_id, None).unwrap(),
        approved
    );
    assert_eq!(
        db.query_row(
            "SELECT indexed_generation FROM workspace_state",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert!(
        store
            .proposal_application(&input.proposal_id)
            .unwrap()
            .is_none()
    );
    for filename in ["model-a.md", "dataset-b.md"] {
        assert!(
            fs::read_to_string(workspace.root.join("knowledge/nodes").join(filename))
                .unwrap()
                .contains("Reviewed ")
        );
    }
    db.execute_batch("DROP TRIGGER fail_apply_history;")
        .unwrap();
    let pending = sync::recovery_status(&workspace).unwrap();
    let snapshot = CanonicalSnapshot::scan_committed(&workspace, &pending.transactions[0].id).unwrap();
    assert_eq!(store.reconcile(&snapshot).unwrap_err().code, "PROPOSAL_APPLY_COORDINATOR_REQUIRED");
    let recovered = sync::recover(&workspace, &mut store).unwrap();
    assert_eq!(recovered.recovered_transaction_ids.len(), 1);
    assert_eq!(recovered.projection.unwrap().generation, 2);
    assert_eq!(
        store
            .proposal_get(&input.proposal_id, None)
            .unwrap()
            .proposal
            .state,
        ProposalState::Applied
    );
    let repeated = apply::execute(&workspace, &mut store, &input, "retry", now()).unwrap();
    assert_eq!(repeated.projection.unwrap().generation, 2);
}

#[test]
fn recovery_refuses_to_apply_a_journal_after_its_reviewed_revision_changes() {
    let (_temp, workspace, mut store, approved, input) = fixture();
    let db = Connection::open(workspace.index_path().unwrap()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_apply_history BEFORE INSERT ON proposal_revisions WHEN NEW.revision=3 BEGIN SELECT RAISE(ABORT,'injected apply failure'); END;").unwrap();
    assert!(apply::execute(&workspace, &mut store, &input, "author", now()).is_err());
    db.execute_batch("DROP TRIGGER fail_apply_history;")
        .unwrap();
    let rejected = ProposalRecord {
        proposal: approved
            .proposal
            .reject(2, "Review changed during interruption", "reviewer", now())
            .unwrap(),
        ..approved
    };
    store.proposal_save(2, &rejected).unwrap();
    assert_eq!(
        sync::recover(&workspace, &mut store).unwrap_err().code,
        "PROPOSAL_REVISION_MISMATCH"
    );
    assert!(sync::recovery_status(&workspace).unwrap().recovery_required);
    assert_eq!(
        store.proposal_get(&input.proposal_id, None).unwrap(),
        rejected
    );
    assert_eq!(
        db.query_row(
            "SELECT indexed_generation FROM workspace_state",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
}
