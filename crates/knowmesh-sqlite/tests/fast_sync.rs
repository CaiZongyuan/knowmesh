#[path = "../../../tests/support/mod.rs"]
mod support;

use std::{
    fs,
    time::{Duration, SystemTime},
};

use knowmesh_core::{
    application::sync,
    canonical::{node::NodeDocument, snapshot::CanonicalSnapshot},
    domain::NodeId,
};
use knowmesh_sqlite::SqliteStore;

#[test]
fn a_migrated_index_populates_missing_warning_state_before_using_the_fast_path() {
    let (temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let path = temp.path().join(".knowmesh/index.sqlite3");
    let mut store = SqliteStore::open(&path).unwrap();
    store
        .bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    sync::synchronize(&workspace, &mut store).unwrap();
    rusqlite::Connection::open(path)
        .unwrap()
        .execute("UPDATE workspace_state SET snapshot_warnings_json=NULL", [])
        .unwrap();
    let report = sync::fast_synchronize(&workspace, &mut store).unwrap();
    assert!(!report.fast_path);
    assert_eq!(report.warnings.len(), 2);
    assert_eq!(report.projection.unwrap().generation, 1);
    assert!(
        sync::fast_synchronize(&workspace, &mut store)
            .unwrap()
            .fast_path
    );
}

#[test]
fn fast_sync_detects_edits_additions_and_deletions_and_keeps_link_warnings() {
    let (temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let mut store = SqliteStore::open(&temp.path().join(".knowmesh/index.sqlite3")).unwrap();
    store
        .bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    let initial = sync::fast_synchronize(&workspace, &mut store).unwrap();
    assert!(!initial.fast_path);
    let unchanged = sync::fast_synchronize(&workspace, &mut store).unwrap();
    assert!(unchanged.fast_path);
    assert_eq!(
        serde_json::to_value(&initial.warnings).unwrap(),
        serde_json::to_value(&unchanged.warnings).unwrap()
    );
    assert_eq!(unchanged.projection.unwrap().generation, 1);

    let path = temp.path().join("knowledge/nodes/model-a.md");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        text.replace("A fictional model.", "A modified fictional model."),
    )
    .unwrap();
    let modified = sync::fast_synchronize(&workspace, &mut store).unwrap();
    assert!(!modified.fast_path);
    assert_eq!(modified.projection.unwrap().generation, 2);

    let template = fs::read_to_string(temp.path().join("knowledge/nodes/dataset-b.md")).unwrap();
    let mut node = NodeDocument::parse(&template).unwrap();
    node.metadata.id = NodeId::new();
    node.metadata.name = "Additional dataset".into();
    let node = NodeDocument::create(
        node.metadata,
        "# Additional dataset\n\n## Summary\n\nSynthetic dataset.",
    )
    .unwrap();
    let new_path = temp.path().join("knowledge/nodes/additional.md");
    fs::write(&new_path, node.render().unwrap()).unwrap();
    let added = sync::fast_synchronize(&workspace, &mut store).unwrap();
    assert_eq!(added.projection.unwrap().node_count, 3);
    fs::remove_file(new_path).unwrap();
    let deleted = sync::fast_synchronize(&workspace, &mut store).unwrap();
    assert_eq!(deleted.projection.unwrap().node_count, 2);
    assert_eq!(store.generation().unwrap(), 4);
}

#[test]
fn metadata_only_changes_refresh_scan_hints_without_a_new_generation() {
    let (temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let mut store = SqliteStore::open(&temp.path().join(".knowmesh/index.sqlite3")).unwrap();
    store
        .bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    sync::synchronize(&workspace, &mut store).unwrap();
    let file = fs::File::options()
        .write(true)
        .open(temp.path().join("knowledge/nodes/model-a.md"))
        .unwrap();
    file.set_modified(SystemTime::now() + Duration::from_secs(60))
        .unwrap();
    let touched = sync::fast_synchronize(&workspace, &mut store).unwrap();
    assert!(!touched.fast_path);
    assert_eq!(touched.projection.unwrap().generation, 1);
    let again = sync::fast_synchronize(&workspace, &mut store).unwrap();
    assert!(again.fast_path);
    assert_eq!(again.projection.unwrap().generation, 1);
}

#[test]
fn v3_claim_keys_are_recomputed_before_metadata_can_take_the_fast_path() {
    use knowmesh_core::domain::{normalize_name, sha256};

    let (temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let path = workspace.index_path().unwrap();
    let mut store = SqliteStore::open(&path).unwrap();
    store
        .bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    sync::synchronize(&workspace, &mut store).unwrap();
    drop(store);
    let assertion = &snapshot.claims[0].claim.assertion;
    let legacy_key = sha256(
        &serde_json::to_vec(&(normalize_name(&assertion.statement), &assertion.qualifiers))
            .unwrap(),
    );
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("DELETE FROM schema_migrations WHERE version=4", [])
        .unwrap();
    db.pragma_update(None, "user_version", 3).unwrap();
    db.execute("UPDATE claims SET normalized_hash=?1,canonical_json=json_set(canonical_json,'$.normalized_hash',?1)", [&legacy_key]).unwrap();
    db.execute(
        "UPDATE workspace_state SET snapshot_sha256=?1",
        [sha256(b"legacy projection hash")],
    )
    .unwrap();
    drop(db);
    let node_path = temp.path().join("knowledge/nodes/model-a.md");
    let before = fs::read(&node_path).unwrap();
    let mut store = SqliteStore::open(&path).unwrap();
    let report = sync::fast_synchronize(&workspace, &mut store).unwrap();
    assert!(!report.fast_path);
    assert_eq!(report.projection.unwrap().generation, 2);
    let db = rusqlite::Connection::open(&path).unwrap();
    let current: String = db
        .query_row("SELECT normalized_hash FROM claims", [], |row| row.get(0))
        .unwrap();
    assert_ne!(current, legacy_key);
    assert_eq!(current, assertion.normalized_hash().unwrap());
    assert_eq!(fs::read(node_path).unwrap(), before);
    assert!(
        sync::fast_synchronize(&workspace, &mut store)
            .unwrap()
            .fast_path
    );
}
