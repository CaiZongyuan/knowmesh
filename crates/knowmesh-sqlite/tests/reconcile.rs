#[path = "../../../tests/support/mod.rs"]
mod support;

use knowmesh_core::{canonical::snapshot::CanonicalSnapshot, ports::ProjectionStore};
use knowmesh_sqlite::SqliteStore;
use rusqlite::Connection;

#[test]
fn reconcile_is_atomic_idempotent_and_rebuilds_the_same_logical_projection() {
    let (temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let path = temp.path().join(".knowmesh/index.sqlite3");
    let mut store = SqliteStore::open(&path).unwrap();
    store
        .bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    let first = store.reconcile(&snapshot).unwrap();
    assert_eq!(first.generation, 1);
    assert_eq!(store.reconcile(&snapshot).unwrap().generation, 1);
    let expected = store.logical_snapshot().unwrap();
    let mut rebuilt = SqliteStore::open(&temp.path().join(".knowmesh/rebuilt.sqlite3")).unwrap();
    rebuilt
        .bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    rebuilt.reconcile(&snapshot).unwrap();
    assert_eq!(rebuilt.logical_snapshot().unwrap(), expected);
    assert_eq!(store.diagnostics().unwrap().foreign_key_violations, 0);
    let db = Connection::open(path).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM claim_evidence", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM relation_evidence", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM source_revisions", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn reconciliation_keeps_runtime_rows_and_old_projection_on_validation_failure() {
    let (temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let path = temp.path().join(".knowmesh/index.sqlite3");
    let mut store = SqliteStore::open(&path).unwrap();
    store
        .bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    store.reconcile(&snapshot).unwrap();
    let db = Connection::open(&path).unwrap();
    db.execute("INSERT INTO proposals(id,kind,state,base_generation,source_revision_id,schema_hash,summary_json,created_by,created_at,updated_at) VALUES('runtime-proposal','compile','draft',1,?1,'schema','{}','fixture','2026-09-05T00:00:00Z','2026-09-05T00:00:00Z')", [snapshot.sources[0].manifest.current_revision_id.as_str()]).unwrap();
    let original = store.logical_snapshot().unwrap();
    let model_path = temp.path().join("knowledge/nodes/model-a.md");
    let content = std::fs::read_to_string(&model_path).unwrap();
    std::fs::write(
        &model_path,
        content.replace("A fictional model.", "A revised fictional model."),
    )
    .unwrap();
    let next = CanonicalSnapshot::scan(&workspace).unwrap();
    assert_eq!(store.reconcile(&next).unwrap().generation, 2);
    assert_ne!(original, store.logical_snapshot().unwrap());
    assert_eq!(
        db.query_row(
            "SELECT source_revision_id FROM proposals WHERE id='runtime-proposal'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        snapshot.sources[0].manifest.current_revision_id.to_string()
    );
    let before = store.logical_snapshot().unwrap();
    let mut invalid = next;
    invalid.relations[0].relation.assertion.target_node_id = knowmesh_core::domain::NodeId::new();
    assert!(store.reconcile(&invalid).is_err());
    assert_eq!(store.logical_snapshot().unwrap(), before);
    assert_eq!(store.generation().unwrap(), 2);
}
