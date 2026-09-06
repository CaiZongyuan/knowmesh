#[path = "../../../tests/support/mod.rs"]
mod support;

use knowmesh_core::{
    application::proposal::{ProposalRecord, prepare},
    canonical::{snapshot::CanonicalSnapshot, workspace::Workspace},
    domain::{
        Timestamp,
        proposal::{PatchOp, ProposalInput, ProposalItem, ProposalKind, ReviewInput, ReviewPolicy},
    },
    ports::{ProjectionStore, ProposalStore},
};
use knowmesh_sqlite::SqliteStore;
use rusqlite::Connection;
use serde_json::json;

fn now() -> Timestamp {
    "2026-09-06T01:00:00Z".parse().unwrap()
}

fn fixture() -> (tempfile::TempDir, Workspace, SqliteStore, ProposalRecord) {
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
            summary: "Stored proposal.".into(),
            items: vec![
                ProposalItem::new(
                    PatchOp::AddAlias,
                    snapshot.nodes[0].metadata.id.to_string(),
                    json!({"alias":"Proposed alias"}),
                )
                .unwrap(),
            ],
        },
        "author",
        now(),
    )
    .unwrap();
    let record = ProposalRecord {
        proposal: prepared.proposal,
        base_snapshot_sha256: prepared.base_snapshot_sha256,
    };
    (temp, workspace, store, record)
}

fn reviewed(record: &ProposalRecord) -> ProposalRecord {
    ProposalRecord {
        proposal: record
            .proposal
            .review(
                &ReviewInput {
                    expected_revision: record.proposal.revision,
                    accept_all: true,
                    decisions: vec![],
                },
                &ReviewPolicy::default(),
                "reviewer",
                now(),
            )
            .unwrap(),
        base_snapshot_sha256: record.base_snapshot_sha256.clone(),
    }
}

#[test]
fn revision_history_keeps_complete_review_metadata_and_atomic_current_items() {
    let (_temp, workspace, mut store, original) = fixture();
    store.proposal_create(&original).unwrap();
    assert_eq!(
        store.proposal_get(&original.proposal.id, None).unwrap(),
        original
    );
    let next = reviewed(&original);
    store.proposal_save(1, &next).unwrap();
    assert_eq!(
        store.proposal_get(&original.proposal.id, Some(1)).unwrap(),
        original
    );
    assert_eq!(
        store.proposal_get(&original.proposal.id, None).unwrap(),
        next
    );
    store.proposal_save(2, &next).unwrap();
    let db = Connection::open(workspace.index_path().unwrap()).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM proposal_revisions", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM audit_events", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.query_row("SELECT decision FROM proposal_items", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "accepted"
    );
    assert_eq!(
        CanonicalSnapshot::scan(&workspace).unwrap().content_sha256,
        original.base_snapshot_sha256
    );
    drop(store);
    let reopened = SqliteStore::open_read_only(&workspace.index_path().unwrap()).unwrap();
    assert_eq!(
        reopened
            .proposal_get(&original.proposal.id, Some(2))
            .unwrap(),
        next
    );
}

#[test]
fn stale_writers_cannot_replace_a_newer_revision_or_skip_history() {
    let (_temp, workspace, mut store, original) = fixture();
    store.proposal_create(&original).unwrap();
    let mut second = SqliteStore::open(&workspace.index_path().unwrap()).unwrap();
    let stale = second.proposal_get(&original.proposal.id, None).unwrap();
    let next = reviewed(&original);
    store.proposal_save(1, &next).unwrap();
    let rejected = ProposalRecord {
        proposal: stale
            .proposal
            .reject(1, "Rejected from stale read", "other-reviewer", now())
            .unwrap(),
        ..stale
    };
    assert_eq!(
        second.proposal_save(1, &rejected).unwrap_err().code,
        "PROPOSAL_REVISION_MISMATCH"
    );
    let mut skipped = next.clone();
    skipped.proposal.revision = 4;
    assert_eq!(
        second.proposal_save(2, &skipped).unwrap_err().code,
        "PROPOSAL_REVISION_MISMATCH"
    );
    assert_eq!(
        second.proposal_get(&original.proposal.id, None).unwrap(),
        next
    );
    assert_eq!(
        store.proposal_create(&original).unwrap_err().code,
        "PROPOSAL_ALREADY_EXISTS"
    );
}

#[test]
fn failed_runtime_write_rolls_back_header_items_history_and_audit_together() {
    let (_temp, workspace, mut store, original) = fixture();
    store.proposal_create(&original).unwrap();
    let db = Connection::open(workspace.index_path().unwrap()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_proposal_audit BEFORE INSERT ON audit_events BEGIN SELECT RAISE(ABORT,'injected audit failure'); END;").unwrap();
    assert!(store.proposal_save(1, &reviewed(&original)).is_err());
    assert_eq!(
        store.proposal_get(&original.proposal.id, None).unwrap(),
        original
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM proposal_revisions", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT decision FROM proposal_items", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "pending"
    );
}

#[test]
fn unindexed_baselines_and_direct_applied_metadata_cannot_be_saved() {
    let (_temp, _workspace, mut store, original) = fixture();
    let mut stale = original.clone();
    stale.base_snapshot_sha256 = "0".repeat(64);
    assert_eq!(
        store.proposal_create(&stale).unwrap_err().code,
        "STALE_PROPOSAL"
    );
    store.proposal_create(&original).unwrap();
    let next = reviewed(&original);
    store.proposal_save(1, &next).unwrap();
    let applied = ProposalRecord {
        proposal: next.proposal.mark_applied(2, 2, "author", now()).unwrap(),
        ..next.clone()
    };
    assert_eq!(
        store.proposal_save(2, &applied).unwrap_err().code,
        "PROPOSAL_APPLY_COORDINATOR_REQUIRED"
    );
    assert_eq!(
        store.proposal_get(&original.proposal.id, None).unwrap(),
        next
    );
}

#[test]
fn corrupt_snapshot_content_and_header_pointers_are_detected_on_read() {
    let (_temp, workspace, mut store, original) = fixture();
    store.proposal_create(&original).unwrap();
    let db = Connection::open(workspace.index_path().unwrap()).unwrap();
    db.execute("UPDATE proposal_revisions SET snapshot_json='{}'", [])
        .unwrap();
    assert_eq!(
        store
            .proposal_get(&original.proposal.id, None)
            .unwrap_err()
            .code,
        "PROPOSAL_HISTORY_INVALID"
    );
    db.execute("UPDATE proposals SET revision=99", []).unwrap();
    assert_eq!(
        store
            .proposal_get(&original.proposal.id, None)
            .unwrap_err()
            .code,
        "PROPOSAL_HISTORY_UNAVAILABLE"
    );
}

#[test]
fn runtime_copy_retains_every_proposal_revision_and_review_binding() {
    let (temp, workspace, mut store, original) = fixture();
    store.proposal_create(&original).unwrap();
    let next = reviewed(&original);
    store.proposal_save(1, &next).unwrap();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let mut candidate = SqliteStore::open(&temp.path().join(".knowmesh/next.sqlite3")).unwrap();
    candidate
        .bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    candidate.reconcile(&snapshot).unwrap();
    let report = candidate.copy_runtime_from(&store).unwrap();
    assert_eq!(report.table_counts["proposal_revisions"], 2);
    assert_eq!(
        candidate
            .proposal_get(&original.proposal.id, Some(1))
            .unwrap(),
        original
    );
    assert_eq!(
        candidate.proposal_get(&original.proposal.id, None).unwrap(),
        next
    );
    candidate.copy_runtime_from(&store).unwrap();
    assert_eq!(
        candidate.proposal_get(&original.proposal.id, None).unwrap(),
        next
    );
}

#[test]
fn stale_proposals_cannot_restore_the_old_approval_by_changing_only_state() {
    use knowmesh_core::domain::proposal::{ProposalRevision, ProposalState};
    let (_temp, _workspace, mut store, original) = fixture();
    store.proposal_create(&original).unwrap();
    let next = reviewed(&original);
    store.proposal_save(1, &next).unwrap();
    let stale = ProposalRecord {
        proposal: next
            .proposal
            .mark_stale(2, "Requires revalidation", "author", now())
            .unwrap(),
        ..next
    };
    store.proposal_save(2, &stale).unwrap();
    let mut restored = stale.clone();
    restored.proposal.state = ProposalState::Approved;
    restored.proposal.state_reason = None;
    restored.proposal.revision += 1;
    restored.validate().unwrap();
    assert_eq!(
        store.proposal_save(3, &restored).unwrap_err().code,
        "PROPOSAL_REVALIDATION_REQUIRED"
    );
    let revised = ProposalRecord {
        proposal: stale
            .proposal
            .revise(
                &ProposalRevision {
                    expected_revision: 3,
                    base_generation: stale.proposal.base_generation,
                    schema_hash: stale.proposal.schema_hash.clone(),
                    summary: stale.proposal.summary.clone(),
                    items: stale.proposal.items.clone(),
                },
                "author",
                now(),
            )
            .unwrap(),
        ..stale
    };
    store.proposal_save(3, &revised).unwrap();
    assert_eq!(
        store
            .proposal_get(&original.proposal.id, None)
            .unwrap()
            .proposal
            .state,
        ProposalState::Draft
    );
}

#[test]
fn migration_preserves_legacy_rows_without_inventing_missing_review_history() {
    let (_temp, workspace, mut store, original) = fixture();
    store.proposal_create(&original).unwrap();
    drop(store);
    let db = Connection::open(workspace.index_path().unwrap()).unwrap();
    db.execute_batch("DROP TABLE proposal_revisions; DELETE FROM schema_migrations WHERE version=6; PRAGMA user_version=5;").unwrap();
    drop(db);
    let store = SqliteStore::open(&workspace.index_path().unwrap()).unwrap();
    assert_eq!(store.diagnostics().unwrap().schema_version, 6);
    assert_eq!(
        store
            .proposal_get(&original.proposal.id, None)
            .unwrap_err()
            .code,
        "PROPOSAL_HISTORY_UNAVAILABLE"
    );
    let db = Connection::open(workspace.index_path().unwrap()).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM proposals", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM proposal_items", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM proposal_revisions", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn atomic_rebuild_and_its_backup_preserve_complete_revision_history() {
    use knowmesh_core::application::rebuild::{self, RebuildInput};
    let (_temp, workspace, mut store, original) = fixture();
    store.proposal_create(&original).unwrap();
    let next = reviewed(&original);
    store.proposal_save(1, &next).unwrap();
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
    assert_eq!(report.runtime_table_counts["proposal_revisions"], 2);
    for path in std::iter::once(workspace.index_path().unwrap()).chain(report.backup_paths) {
        let store = SqliteStore::open_read_only(&path).unwrap();
        assert_eq!(
            store.proposal_get(&original.proposal.id, Some(1)).unwrap(),
            original
        );
        assert_eq!(
            store.proposal_get(&original.proposal.id, None).unwrap(),
            next
        );
    }
}
