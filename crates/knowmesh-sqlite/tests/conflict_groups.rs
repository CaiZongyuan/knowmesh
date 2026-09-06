#[path = "../../../tests/support/mod.rs"]
mod support;

use std::fs;

use knowmesh_core::{
    canonical::{node::NodeDocument, snapshot::CanonicalSnapshot, workspace::Workspace},
    domain::{ClaimId, ConflictGroup, ConflictGroupId, ConflictGroupStatus, EvidenceStatus},
    ports::ProjectionStore,
};
use knowmesh_sqlite::SqliteStore;

fn fixture() -> (tempfile::TempDir, Workspace, NodeDocument) {
    let (temp, workspace) = support::fixture();
    let path = temp.path().join("knowledge/nodes/model-a.md");
    let mut doc = NodeDocument::parse(&fs::read_to_string(&path).unwrap()).unwrap();
    let mut other = doc.claims[0].clone();
    other.id = ClaimId::new();
    other.statement = "Model A was not evaluated on Dataset B.".into();
    doc.claims.push(other);
    let mut claim_ids: Vec<_> = doc.claims.iter().map(|claim| claim.id.clone()).collect();
    claim_ids.sort();
    let group = ConflictGroup {
        id: ConflictGroupId::new(),
        claim_ids,
        reason: "The statements disagree about the same evaluation.".into(),
        status: ConflictGroupStatus::Open,
        created_at: "2026-09-06T00:00:00Z".parse().unwrap(),
        resolved_at: None,
    };
    for claim in &mut doc.claims {
        claim.evidence_status = EvidenceStatus::Conflicting;
        claim.conflict_groups = vec![group.clone()];
    }
    fs::write(path, doc.render().unwrap()).unwrap();
    (temp, workspace, doc)
}

fn store(workspace: &Workspace) -> SqliteStore {
    let snapshot = CanonicalSnapshot::scan(workspace).unwrap();
    let mut store = SqliteStore::open(&workspace.index_path().unwrap()).unwrap();
    store
        .bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    store.reconcile(&snapshot).unwrap();
    store
}

#[test]
fn canonical_conflict_groups_are_indexed_once_and_rebuilt_with_all_members() {
    let (temp, workspace, doc) = fixture();
    let store = store(&workspace);
    let connection = rusqlite::Connection::open(workspace.index_path().unwrap()).unwrap();
    let count: usize = connection
        .query_row("SELECT count(*) FROM conflict_groups", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
    let count: usize = connection
        .query_row("SELECT count(*) FROM conflict_group_claims", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 2);
    let expected = store.logical_snapshot().unwrap();
    assert_eq!(
        expected["conflict_groups"][0]["group"]["id"],
        doc.claims[0].conflict_groups[0].id.as_str()
    );
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let mut rebuilt = SqliteStore::open(&temp.path().join("rebuilt.sqlite3")).unwrap();
    rebuilt
        .bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    rebuilt.reconcile(&snapshot).unwrap();
    assert_eq!(rebuilt.logical_snapshot().unwrap(), expected);
    drop(connection);
    drop(store);
    let backend = knowmesh_sqlite::SqliteRebuilder::new(&workspace).unwrap();
    let report = knowmesh_core::application::rebuild::execute(
        &workspace,
        &backend,
        &knowmesh_core::application::rebuild::RebuildInput {
            yes: true,
            ..Default::default()
        },
    )
    .unwrap();
    let current = SqliteStore::open_read_only(&workspace.index_path().unwrap()).unwrap();
    assert_eq!(current.logical_snapshot().unwrap(), expected);
    let backup = SqliteStore::open_read_only(&report.backup_paths[0]).unwrap();
    assert_eq!(backup.logical_snapshot().unwrap(), expected);
}

#[test]
fn resolved_and_removed_conflicts_follow_canonical_changes() {
    let (temp, workspace, mut doc) = fixture();
    let mut store = store(&workspace);
    for claim in &mut doc.claims {
        claim.conflict_groups[0].status = ConflictGroupStatus::Resolved;
        claim.conflict_groups[0].resolved_at = Some("2026-09-06T01:00:00Z".parse().unwrap());
    }
    let path = temp.path().join("knowledge/nodes/model-a.md");
    fs::write(&path, doc.render().unwrap()).unwrap();
    store
        .reconcile(&CanonicalSnapshot::scan(&workspace).unwrap())
        .unwrap();
    assert_eq!(
        store.logical_snapshot().unwrap()["conflict_groups"][0]["group"]["status"],
        "resolved"
    );
    for claim in &mut doc.claims {
        claim.conflict_groups.clear();
    }
    fs::write(&path, doc.render().unwrap()).unwrap();
    store
        .reconcile(&CanonicalSnapshot::scan(&workspace).unwrap())
        .unwrap();
    assert_eq!(
        store.logical_snapshot().unwrap()["conflict_groups"],
        serde_json::json!([])
    );
}

#[test]
fn a_failed_conflict_projection_restores_the_previous_complete_index() {
    let (temp, workspace, mut doc) = fixture();
    let mut store = store(&workspace);
    let before = store.logical_snapshot().unwrap();
    let connection = rusqlite::Connection::open(workspace.index_path().unwrap()).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_conflict BEFORE INSERT ON conflict_groups BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
    for claim in &mut doc.claims {
        claim.conflict_groups[0].reason = "Updated conflict explanation.".into();
    }
    fs::write(
        temp.path().join("knowledge/nodes/model-a.md"),
        doc.render().unwrap(),
    )
    .unwrap();
    assert!(
        store
            .reconcile(&CanonicalSnapshot::scan(&workspace).unwrap())
            .is_err()
    );
    assert_eq!(store.logical_snapshot().unwrap(), before);
}
