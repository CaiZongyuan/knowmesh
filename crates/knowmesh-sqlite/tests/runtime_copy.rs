#[path = "../../../tests/support/mod.rs"]
mod support;
#[path = "support/runtime.rs"]
mod runtime_support;

use knowmesh_core::{canonical::snapshot::CanonicalSnapshot, ports::ProjectionStore};
use knowmesh_sqlite::SqliteStore;
use rusqlite::Connection;
use runtime_support::runtime_fixture;

#[test]
fn runtime_copy_preserves_rows_self_references_and_the_audit_sequence() {
    let (temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let path = temp.path().join(".knowmesh/index.sqlite3");
    let mut source = SqliteStore::open(&path).unwrap();
    source
        .bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    source.reconcile(&snapshot).unwrap();
    runtime_fixture(
        &path,
        snapshot.sources[0].manifest.current_revision_id.as_str(),
    );
    let next_path = temp.path().join(".knowmesh/index.next.sqlite3");
    let mut next = SqliteStore::open(&next_path).unwrap();
    next.bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    next.reconcile(&snapshot).unwrap();
    let report = next.copy_runtime_from(&source).unwrap();
    assert_eq!(report.table_counts["operation_runs"], 2);
    for table in [
        "proposals",
        "proposal_items",
        "idempotency_keys",
        "audit_events",
    ] {
        assert_eq!(report.table_counts[table], 1);
    }
    assert_eq!(next.diagnostics().unwrap().foreign_key_violations, 0);
    assert_eq!(
        next.logical_snapshot().unwrap(),
        source.logical_snapshot().unwrap()
    );
    let db = Connection::open(&next_path).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT parent_run_id FROM operation_runs WHERE id='child'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "parent"
    );
    assert_eq!(
        db.query_row("SELECT source_revision_id FROM proposals", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        snapshot.sources[0].manifest.current_revision_id.to_string()
    );
    assert_eq!(
        db.query_row("SELECT response_json FROM idempotency_keys", [], |r| r
            .get::<_, String>(
            0
        ))
        .unwrap(),
        "{\"output\":\"proposal\"}"
    );
    db.execute("INSERT INTO audit_events(event_id,event_type,actor,created_at) VALUES('new-event','fixture','fixture','2026-09-05T00:00:00Z')", []).unwrap();
    assert!(db.last_insert_rowid() > 90);
    // Refreshing the candidate uses the current runtime snapshot, without duplication.
    let db = Connection::open(&path).unwrap();
    db.execute(
        "UPDATE operation_runs SET status='cancelled' WHERE id='child'",
        [],
    )
    .unwrap();
    next.copy_runtime_from(&source).unwrap();
    let db = Connection::open(next_path).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT status FROM operation_runs WHERE id='child'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "cancelled"
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM operation_runs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn missing_runtime_references_stop_copy_without_changing_either_database() {
    let (temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let path = temp.path().join(".knowmesh/index.sqlite3");
    let mut source = SqliteStore::open(&path).unwrap();
    source
        .bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    source.reconcile(&snapshot).unwrap();
    runtime_fixture(
        &path,
        snapshot.sources[0].manifest.current_revision_id.as_str(),
    );
    for file in &snapshot.files {
        if ["source", "source_blob", "node", "synthesis"].contains(&file.kind.as_str()) {
            std::fs::remove_file(temp.path().join(&file.path)).unwrap();
        }
    }
    let empty = CanonicalSnapshot::scan(&workspace).unwrap();
    let next_path = temp.path().join(".knowmesh/index.next.sqlite3");
    let mut next = SqliteStore::open(&next_path).unwrap();
    next.bind_workspace(&workspace.config.workspace.id, &empty.schema_hash)
        .unwrap();
    next.reconcile(&empty).unwrap();
    assert_eq!(
        next.copy_runtime_from(&source).unwrap_err().code,
        "RUNTIME_REFERENCE_MISSING"
    );
    assert_eq!(
        Connection::open(next_path)
            .unwrap()
            .query_row("SELECT count(*) FROM operation_runs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        Connection::open(path)
            .unwrap()
            .query_row("SELECT count(*) FROM operation_runs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
}
