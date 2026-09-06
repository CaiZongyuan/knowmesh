#[path = "../../../tests/support/mod.rs"]
mod support;

use std::fs;

use knowmesh_core::{
    application::{
        impact::{self, ImpactInput, ImpactKind},
        rebuild::{self, RebuildInput},
        source,
    },
    canonical::{
        node::NodeDocument, snapshot::CanonicalSnapshot, source::ImportInput,
        synthesis::SynthesisDocument, workspace::Workspace,
    },
    domain::{
        EvidenceId, SourceRevisionId,
        freshness::{Freshness, FreshnessReasonCode},
    },
    ports::ProjectionStore,
};
use knowmesh_sqlite::{SqliteImpactPreview, SqliteRebuilder, SqliteStore};

fn indexed(workspace: &Workspace) -> SqliteStore {
    let snapshot = CanonicalSnapshot::scan(workspace).unwrap();
    let mut store = SqliteStore::open(&workspace.index_path().unwrap()).unwrap();
    store
        .bind_workspace(&snapshot.workspace_id, &snapshot.schema_hash)
        .unwrap();
    store.reconcile(&snapshot).unwrap();
    store
}

#[test]
fn impact_keeps_independent_evidence_and_reports_missing_synthesis_snapshots() {
    let (temp, workspace) = support::fixture();
    let mut store = indexed(&workspace);
    let mut query = input(&workspace);
    let path = temp.path().join("independent.md");
    fs::write(
        &path,
        "# Independent source\n\nModel A was evaluated on Dataset B.\n",
    )
    .unwrap();
    let added = source::add(
        &workspace,
        &mut store,
        &ImportInput {
            path,
            source_id: None,
            storage: None,
            title: Some("Independent source".into()),
            kind: "paper".into(),
            tags: vec![],
            dry_run: false,
        },
        None,
    )
    .unwrap();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let independent = snapshot
        .sources
        .iter()
        .find(|source| source.manifest.id == added.import.source.id)
        .unwrap();
    let model_path = temp.path().join("knowledge/nodes/model-a.md");
    let mut model = NodeDocument::parse(&fs::read_to_string(&model_path).unwrap()).unwrap();
    let mut evidence = model.claims[0].evidence[0].clone();
    evidence.id = EvidenceId::new();
    evidence.source_revision_id = independent.manifest.current_revision_id.clone();
    model.claims[0].evidence.push(evidence.clone());
    fs::write(model_path, model.render().unwrap()).unwrap();
    source::remove(
        &workspace,
        &mut store,
        &source::RemoveInput {
            source_id: query.source_id.clone(),
            dry_run: false,
            yes: true,
        },
    )
    .unwrap();
    query.kind = Some(ImpactKind::Claim);
    let report = impact::execute(&workspace, &mut store, &query).unwrap();
    assert_eq!(report.items.len(), 1);
    assert_eq!(report.items[0].freshness.evidence_ids.len(), 2);
    assert_eq!(
        report.items[0].freshness.current_evidence_ids,
        vec![evidence.id]
    );
    assert_eq!(report.items[0].freshness.freshness, Freshness::NeedsReview);
    query.revision = Some(independent.manifest.current_revision_id.clone());
    assert_eq!(
        impact::execute(&workspace, &mut store, &query)
            .unwrap_err()
            .code,
        "SOURCE_REVISION_MISMATCH"
    );
    query.revision = None;
    let path = temp.path().join("knowledge/syntheses/comparison.md");
    let mut synthesis = SynthesisDocument::parse(&fs::read_to_string(&path).unwrap()).unwrap();
    synthesis.metadata.dependency_snapshot = None;
    fs::write(path, synthesis.render().unwrap()).unwrap();
    query.kind = Some(ImpactKind::Synthesis);
    let report = impact::execute(&workspace, &mut store, &query).unwrap();
    assert_eq!(report.items[0].freshness.freshness, Freshness::Unknown);
    assert!(
        report.items[0]
            .freshness
            .freshness_reasons
            .iter()
            .any(|reason| reason.code == FreshnessReasonCode::SnapshotMissing)
    );
}

#[test]
fn impact_traverses_snapshot_assertions_and_source_heads_without_direct_citations() {
    for source_head_only in [false, true] {
        let (temp, workspace) = support::fixture();
        let mut store = indexed(&workspace);
        let mut query = input(&workspace);
        query.kind = Some(ImpactKind::Synthesis);
        let path = temp.path().join("knowledge/syntheses/comparison.md");
        let mut metadata = SynthesisDocument::parse(&fs::read_to_string(&path).unwrap())
            .unwrap()
            .metadata;
        metadata.evidence_ids.clear();
        let snapshot = metadata.dependency_snapshot.as_mut().unwrap();
        if source_head_only {
            snapshot.assertions.clear();
        } else {
            snapshot.source_heads.clear();
        }
        let synthesis = SynthesisDocument::create(metadata, "# Snapshot dependency\n").unwrap();
        fs::write(path, synthesis.render().unwrap()).unwrap();
        let report = impact::execute(&workspace, &mut store, &query).unwrap();
        assert_eq!(report.counts.syntheses, 1);
        assert_eq!(report.items.len(), 1);
        assert_eq!(report.items[0].dependency_ids.len(), 1);
        assert_eq!(report.items[0].freshness.freshness, Freshness::Current);
        let reasons = serde_json::to_value(&report.items[0].reasons).unwrap();
        assert_eq!(
            reasons[0],
            if source_head_only {
                "source_head"
            } else {
                "assertion_dependency"
            }
        );
    }
}

fn input(workspace: &Workspace) -> ImpactInput {
    ImpactInput {
        source_id: CanonicalSnapshot::scan(workspace).unwrap().sources[0]
            .manifest
            .id
            .clone(),
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
        if query.cursor.is_none() {
            break;
        }
    }
    assert_eq!(objects.len(), 4);
    assert!(objects.windows(2).all(|pair| pair[0] < pair[1]));
    query.cursor = Some(cursor.clone());
    query.kind = Some(ImpactKind::Synthesis);
    assert_eq!(
        impact::execute(&workspace, &mut store, &query)
            .unwrap_err()
            .code,
        "CURSOR_QUERY_MISMATCH"
    );
    query.kind = None;
    query.cursor = None;
    query.revision = Some(SourceRevisionId::new());
    assert_eq!(
        impact::execute(&workspace, &mut store, &query)
            .unwrap_err()
            .code,
        "SOURCE_REVISION_NOT_FOUND"
    );
    query.revision = None;
    source::remove(
        &workspace,
        &mut store,
        &source::RemoveInput {
            source_id: query.source_id.clone(),
            dry_run: false,
            yes: true,
        },
    )
    .unwrap();
    query.cursor = Some(cursor);
    assert_eq!(
        impact::execute(&workspace, &mut store, &query)
            .unwrap_err()
            .code,
        "CURSOR_STALE"
    );
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
    source::add(
        &workspace,
        &mut store,
        &ImportInput {
            path,
            source_id: Some(query.source_id.clone()),
            storage: None,
            title: None,
            kind: "paper".into(),
            tags: vec![],
            dry_run: false,
        },
        None,
    )
    .unwrap();
    query.revision = Some(revision);
    let revised = impact::execute(&workspace, &mut store, &query).unwrap();
    assert_eq!(revised.items.len(), 4);
    assert!(
        revised
            .items
            .iter()
            .all(|item| item.freshness.freshness == Freshness::NeedsReview)
    );
    source::remove(
        &workspace,
        &mut store,
        &source::RemoveInput {
            source_id: query.source_id.clone(),
            dry_run: false,
            yes: true,
        },
    )
    .unwrap();
    let removed = impact::execute(&workspace, &mut store, &query).unwrap();
    assert!(removed.items.iter().all(|item| {
        item.freshness
            .freshness_reasons
            .iter()
            .any(|reason| reason.code == FreshnessReasonCode::SourceRemoved)
    }));
    assert_eq!(fs::read(&synthesis_path).unwrap(), synthesis);
    assert_eq!(
        CanonicalSnapshot::scan(&workspace).unwrap().evidence.len(),
        before.evidence.len()
    );
    drop(store);
    rebuild::execute(
        &workspace,
        &SqliteRebuilder::new(&workspace).unwrap(),
        &RebuildInput {
            yes: true,
            ..Default::default()
        },
    )
    .unwrap();
    let mut store = SqliteStore::open(&workspace.index_path().unwrap()).unwrap();
    let rebuilt = impact::execute(&workspace, &mut store, &query).unwrap();
    assert_eq!(
        serde_json::to_value(&rebuilt.items).unwrap(),
        serde_json::to_value(&removed.items).unwrap()
    );
    assert_eq!(rebuilt.generation, removed.generation);
    query.no_sync = true;
    let unsynchronized = impact::execute(&workspace, &mut store, &query).unwrap();
    assert!(
        unsynchronized
            .items
            .iter()
            .all(|item| item.freshness.freshness == Freshness::Unknown
                && item.freshness.current_evidence_ids.is_empty())
    );
}

#[test]
fn removal_preview_uses_canonical_impact_without_writing_the_index_and_its_cursor_can_continue() {
    for existing_index in [false, true] {
        let (temp, workspace) = support::fixture();
        let mut query = input(&workspace);
        if existing_index { drop(indexed(&workspace)); }
        let path = temp.path().join("knowledge/nodes/model-a.md");
        let mut model = NodeDocument::parse(&fs::read_to_string(&path).unwrap()).unwrap();
        for index in 0..24 {
            let mut claim = model.claims[0].clone();
            claim.id = knowmesh_core::domain::ClaimId::new();
            claim.statement = format!("Synthetic assertion {index}.");
            model.claims.push(claim);
        }
        fs::write(path, model.render().unwrap()).unwrap();
        let index_path = workspace.index_path().unwrap();
        let before = fs::read(&index_path).ok();
        let preview = source::preview_remove_with_impact(&workspace, &source::RemoveInput { source_id: query.source_id.clone(), dry_run: true, yes: false }, &SqliteImpactPreview::new(&workspace).unwrap()).unwrap();
        let impact = preview.impact.unwrap();
        assert!(impact.preview);
        assert_eq!(impact.items.len(), 20);
        assert_eq!(impact.counts.claims, 25);
        assert_eq!(impact.generation, if existing_index { 2 } else { 1 });
        assert_eq!(fs::read(&index_path).ok(), before);
        assert!(CanonicalSnapshot::scan(&workspace).unwrap().sources[0].manifest.removed_at.is_none());
        query.cursor = impact.next_cursor;
        assert!(query.cursor.is_some());
        let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
        let mut store = SqliteStore::open(&index_path).unwrap();
        store.bind_workspace(&snapshot.workspace_id, &snapshot.schema_hash).unwrap();
        let continued = impact::execute(&workspace, &mut store, &query).unwrap();
        assert!(!continued.preview);
        assert_eq!(continued.items.len(), 8);
        assert!(continued.next_cursor.is_none());
    }
}
