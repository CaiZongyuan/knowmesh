#[path = "../../../tests/support/mod.rs"]
mod support;

use std::fs;

use knowmesh_core::{
    application::{impact::{self, ImpactInput, ImpactKind}, rebuild::{self, RebuildInput}, source},
    canonical::{snapshot::CanonicalSnapshot, source::ImportInput, workspace::Workspace},
    domain::{SourceRevisionId, freshness::{Freshness, FreshnessReasonCode}},
    ports::ProjectionStore,
};
use knowmesh_sqlite::{SqliteRebuilder, SqliteStore};

fn indexed(workspace: &Workspace) -> SqliteStore {
    let snapshot = CanonicalSnapshot::scan(workspace).unwrap();
    let mut store = SqliteStore::open(&workspace.index_path().unwrap()).unwrap();
    store.bind_workspace(&snapshot.workspace_id, &snapshot.schema_hash).unwrap();
    store.reconcile(&snapshot).unwrap();
    store
}

fn input(workspace: &Workspace) -> ImpactInput {
    ImpactInput {
        source_id: CanonicalSnapshot::scan(workspace).unwrap().sources[0].manifest.id.clone(),
        revision: None,
        kind: None,
        limit: 20,
        cursor: None,
        no_sync: false,
    }
}

#[test]
fn impact_pages_all_dependency_kinds_and_binds_cursors_to_the_query_and_generation() {
    let (_temp, workspace) = support::fixture();
    let mut store = indexed(&workspace);
    let mut query = input(&workspace);
    query.limit = 1;
    let first = impact::execute(&workspace, &mut store, &query).unwrap();
    assert_eq!(first.counts.evidence, 1);
    assert_eq!(first.counts.claims, 1);
    assert_eq!(first.counts.relations, 1);
    assert_eq!(first.counts.syntheses, 1);
    assert_eq!(first.generation, 1);
    let cursor = first.next_cursor.clone().unwrap();
    let mut objects = vec![];
    loop {
        let page = impact::execute(&workspace, &mut store, &query).unwrap();
        assert!(page.items.len() <= 1);
        assert_eq!(page.items[0].freshness.freshness, Freshness::Current);
        assert!(!page.items[0].dependency_ids.is_empty());
        objects.push(page.items[0].object.clone());
        query.cursor = page.next_cursor;
        if query.cursor.is_none() { break; }
    }
    assert_eq!(objects.len(), 4);
    assert!(objects.windows(2).all(|pair| pair[0] < pair[1]));
    query.cursor = Some(cursor.clone());
    query.kind = Some(ImpactKind::Synthesis);
    assert_eq!(impact::execute(&workspace, &mut store, &query).unwrap_err().code, "CURSOR_QUERY_MISMATCH");
    query.kind = None;
    query.cursor = None;
    query.revision = Some(SourceRevisionId::new());
    assert_eq!(impact::execute(&workspace, &mut store, &query).unwrap_err().code, "SOURCE_REVISION_NOT_FOUND");
    query.revision = None;
    source::remove(&workspace, &mut store, &source::RemoveInput { source_id: query.source_id.clone(), dry_run: false, yes: true }).unwrap();
    query.cursor = Some(cursor);
    assert_eq!(impact::execute(&workspace, &mut store, &query).unwrap_err().code, "CURSOR_STALE");
}

#[test]
fn source_changes_preserve_historical_evidence_and_impact_survives_database_rebuild() {
    let (temp, workspace) = support::fixture();
    let mut store = indexed(&workspace);
    let mut query = input(&workspace);
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let revision = before.sources[0].manifest.current_revision_id.clone();
    let synthesis_path = temp.path().join("knowledge/syntheses/comparison.md");
    let synthesis = fs::read(&synthesis_path).unwrap();
    let path = temp.path().join("revision.md");
    fs::write(&path, "# Updated source\n\nA new synthetic revision.\n").unwrap();
    source::add(&workspace, &mut store, &ImportInput { path, source_id: Some(query.source_id.clone()), storage: None, title: None, kind: "paper".into(), tags: vec![], dry_run: false }, None).unwrap();
    query.revision = Some(revision);
    let revised = impact::execute(&workspace, &mut store, &query).unwrap();
    assert_eq!(revised.items.len(), 4);
    assert!(revised.items.iter().all(|item| item.freshness.freshness == Freshness::NeedsReview));
    source::remove(&workspace, &mut store, &source::RemoveInput { source_id: query.source_id.clone(), dry_run: false, yes: true }).unwrap();
    let removed = impact::execute(&workspace, &mut store, &query).unwrap();
    assert!(removed.items.iter().all(|item| item.freshness.freshness_reasons.iter().any(|reason| reason.code == FreshnessReasonCode::SourceRemoved)));
    assert_eq!(fs::read(&synthesis_path).unwrap(), synthesis);
    assert_eq!(CanonicalSnapshot::scan(&workspace).unwrap().evidence.len(), before.evidence.len());
    drop(store);
    rebuild::execute(&workspace, &SqliteRebuilder::new(&workspace).unwrap(), &RebuildInput { yes: true, ..Default::default() }).unwrap();
    let mut store = SqliteStore::open(&workspace.index_path().unwrap()).unwrap();
    let rebuilt = impact::execute(&workspace, &mut store, &query).unwrap();
    assert_eq!(serde_json::to_value(&rebuilt.items).unwrap(), serde_json::to_value(&removed.items).unwrap());
    assert_eq!(rebuilt.generation, removed.generation);
    query.no_sync = true;
    let unsynchronized = impact::execute(&workspace, &mut store, &query).unwrap();
    assert!(unsynchronized.items.iter().all(|item| item.freshness.freshness == Freshness::Unknown && item.freshness.current_evidence_ids.is_empty()));
}
