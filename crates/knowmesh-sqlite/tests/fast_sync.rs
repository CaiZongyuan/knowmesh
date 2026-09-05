#[path = "../../../tests/support/mod.rs"]
mod support;

use std::{fs, time::{Duration, SystemTime}};

use knowmesh_core::{application::sync, canonical::{node::NodeDocument, snapshot::CanonicalSnapshot}, domain::NodeId};
use knowmesh_sqlite::SqliteStore;

#[test]
fn fast_sync_detects_edits_additions_and_deletions_and_keeps_link_warnings() {
    let (temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let mut store = SqliteStore::open(&temp.path().join(".knowmesh/index.sqlite3")).unwrap();
    store.bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash).unwrap();
    let initial = sync::fast_synchronize(&workspace, &mut store).unwrap();
    assert!(!initial.fast_path);
    let unchanged = sync::fast_synchronize(&workspace, &mut store).unwrap();
    assert!(unchanged.fast_path);
    assert_eq!(serde_json::to_value(&initial.warnings).unwrap(), serde_json::to_value(&unchanged.warnings).unwrap());
    assert_eq!(unchanged.projection.unwrap().generation, 1);

    let path = temp.path().join("knowledge/nodes/model-a.md");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace("A fictional model.", "A modified fictional model.")).unwrap();
    let modified = sync::fast_synchronize(&workspace, &mut store).unwrap();
    assert!(!modified.fast_path);
    assert_eq!(modified.projection.unwrap().generation, 2);

    let template = fs::read_to_string(temp.path().join("knowledge/nodes/dataset-b.md")).unwrap();
    let mut node = NodeDocument::parse(&template).unwrap();
    node.metadata.id = NodeId::new();
    node.metadata.name = "Additional dataset".into();
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
    store.bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash).unwrap();
    sync::synchronize(&workspace, &mut store).unwrap();
    let file = fs::File::options().write(true).open(temp.path().join("knowledge/nodes/model-a.md")).unwrap();
    file.set_modified(SystemTime::now() + Duration::from_secs(60)).unwrap();
    let touched = sync::fast_synchronize(&workspace, &mut store).unwrap();
    assert!(!touched.fast_path);
    assert_eq!(touched.projection.unwrap().generation, 1);
    let again = sync::fast_synchronize(&workspace, &mut store).unwrap();
    assert!(again.fast_path);
    assert_eq!(again.projection.unwrap().generation, 1);
}
