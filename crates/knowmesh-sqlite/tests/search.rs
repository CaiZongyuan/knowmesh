#[path = "../../../tests/support/mod.rs"]
mod support;

use std::{collections::BTreeSet, fs};

use knowmesh_core::{
    application::{
        lexical::RecordType,
        search::{self, SearchInput, SearchReport},
    },
    canonical::{snapshot::CanonicalSnapshot, workspace::Workspace},
    domain::{ChunkId, SourceManifest, Timestamp, freshness::Freshness, sha256},
    ports::ProjectionStore,
};
use knowmesh_sqlite::SqliteStore;

fn fixture() -> (tempfile::TempDir, Workspace, SqliteStore, CanonicalSnapshot) {
    let (temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let mut store = SqliteStore::open(&workspace.index_path().unwrap()).unwrap();
    store
        .bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    store.reconcile(&snapshot).unwrap();
    (temp, workspace, store, snapshot)
}

fn input(query: &str) -> SearchInput {
    SearchInput {
        query: query.into(),
        explain: true,
        ..Default::default()
    }
}

fn ids(report: &SearchReport) -> Vec<String> {
    report
        .groups
        .knowledge
        .iter()
        .chain(&report.groups.claims)
        .chain(&report.groups.sources)
        .chain(&report.groups.syntheses)
        .chain(&report.groups.chunks)
        .map(|hit| hit.unit_id.clone())
        .collect()
}

#[test]
fn search_reads_real_candidates_exact_ids_and_freshness_through_one_core_use_case() {
    let (_temp, workspace, mut store, snapshot) = fixture();
    let report = search::execute(&workspace, &mut store, &input("Model")).unwrap();
    assert!(report.index_complete);
    assert!(report.capabilities.word_fts);
    assert!(!report.capabilities.vector);
    assert_eq!(report.groups.claims.len(), 1);
    let claim = &report.groups.claims[0];
    assert!(
        serde_json::to_value(claim).unwrap()["preview"]
            .as_str()
            .unwrap()
            .contains("Model A")
    );
    assert_eq!(
        claim.freshness.as_ref().unwrap().freshness,
        Freshness::Current
    );
    assert_eq!(claim.freshness.as_ref().unwrap().evidence_ids.len(), 1);
    assert!(claim.explain.as_ref().unwrap().normalization_bound > 0.0);
    assert!(!serde_json::to_string(&report).unwrap().contains("rowid"));
    let model = snapshot
        .nodes
        .iter()
        .find(|node| node.metadata.node_type == "Model")
        .unwrap();
    let mut exact = input(model.metadata.id.as_str());
    let report = search::execute(&workspace, &mut store, &exact).unwrap();
    assert_eq!(report.groups.knowledge.len(), 1);
    assert!(report.groups.knowledge[0].exact_id_tier);
    assert_eq!(report.resolved_entities[0].node_id, model.metadata.id);
    exact.node_types = vec!["Dataset".into()];
    assert!(ids(&search::execute(&workspace, &mut store, &exact).unwrap()).is_empty());
    exact.node_types.clear();
    exact.tags = vec!["absent-tag".into()];
    assert!(ids(&search::execute(&workspace, &mut store, &exact).unwrap()).is_empty());
}

#[test]
fn full_filters_apply_to_every_channel_and_source_dependencies_include_evidence_links() {
    let (_temp, workspace, mut store, snapshot) = fixture();
    let mut filtered = input("fixture");
    filtered.source_ids = vec![snapshot.sources[0].manifest.id.clone()];
    filtered.tags = vec!["fixture".into()];
    let report = search::execute(&workspace, &mut store, &filtered).unwrap();
    assert_eq!(report.groups.knowledge.len(), 2);
    assert_eq!(report.groups.sources.len(), 1);
    assert_eq!(report.groups.syntheses.len(), 1);
    filtered.record_types = vec![RecordType::Node];
    filtered.node_types = vec!["Model".into()];
    assert_eq!(
        ids(&search::execute(&workspace, &mut store, &filtered).unwrap()).len(),
        1
    );
    filtered.tags = vec!["fixture-other".into()];
    assert!(ids(&search::execute(&workspace, &mut store, &filtered).unwrap()).is_empty());
}

#[test]
fn a_node_type_filter_is_applied_before_a_one_candidate_channel_limit() {
    let (temp, workspace, mut store, snapshot) = fixture();
    let mut request = input("fixture");
    request.record_types = vec![RecordType::Node];
    let baseline = search::execute(&workspace, &mut store, &request).unwrap();
    let target = &baseline.groups.knowledge[1].record_id;
    request.node_types = vec![
        snapshot
            .nodes
            .iter()
            .find(|node| node.metadata.id.as_str() == target)
            .unwrap()
            .metadata
            .node_type
            .clone(),
    ];
    let mut config = serde_json::to_value(&workspace.config).unwrap();
    config["search"]["candidate_limit"] = 1.into();
    fs::write(
        temp.path().join("knowmesh.yaml"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let workspace = Workspace::load(temp.path()).unwrap();
    let filtered = search::execute(&workspace, &mut store, &request).unwrap();
    assert_eq!(filtered.groups.knowledge.len(), 1);
    assert_eq!(filtered.groups.knowledge[0].record_id, *target);
}

#[test]
fn chunk_filters_follow_their_source_revision_node_or_synthesis_owner() {
    let (_temp, workspace, mut store, snapshot) = fixture();
    let db = rusqlite::Connection::open(store.path()).unwrap();
    let model = snapshot
        .nodes
        .iter()
        .find(|node| node.metadata.node_type == "Model")
        .unwrap();
    for (kind, owner) in [
        (
            "source_revision",
            snapshot.sources[0].manifest.current_revision_id.as_str(),
        ),
        ("node", model.metadata.id.as_str()),
        ("synthesis", snapshot.syntheses[0].metadata.id.as_str()),
    ] {
        let id = ChunkId::new();
        db.execute("INSERT INTO chunks(id,owner_kind,owner_id,ordinal,text,content_sha256) VALUES(?1,?2,?3,0,'fixture chunk',?4)",
            rusqlite::params![id.as_str(), kind, owner, sha256(b"fixture chunk")]).unwrap();
        db.execute("INSERT INTO search_units(unit_id,record_type,record_id,body,content_sha256,updated_at) VALUES(?1,'chunk',?2,'fixture chunk',?3,'2026-09-06T00:00:00Z')",
            rusqlite::params![format!("chunk:{id}"), id.as_str(), sha256(b"fixture chunk")]).unwrap();
    }
    let mut request = input("fixture");
    request.record_types = vec![RecordType::Chunk];
    request.node_types = vec!["Model".into()];
    request.source_ids = vec![snapshot.sources[0].manifest.id.clone()];
    request.tags = vec!["fixture".into()];
    let result = search::execute(&workspace, &mut store, &request).unwrap();
    assert_eq!(result.groups.chunks.len(), 3);
    request.tags = vec!["unrelated".into()];
    assert!(
        search::execute(&workspace, &mut store, &request)
            .unwrap()
            .groups
            .chunks
            .is_empty()
    );
}

#[test]
fn synthesis_filters_follow_recorded_assertions_without_direct_source_or_node_links() {
    let (temp, workspace, mut store, snapshot) = fixture();
    let path = temp.path().join("knowledge/syntheses/comparison.md");
    let mut document = knowmesh_core::canonical::synthesis::SynthesisDocument::parse(
        &fs::read_to_string(&path).unwrap(),
    )
    .unwrap();
    document.metadata.related_nodes.clear();
    document.metadata.evidence_ids.clear();
    document
        .metadata
        .dependency_snapshot
        .as_mut()
        .unwrap()
        .source_heads
        .clear();
    document.body = "# Fixture comparison\n\nRecorded assertion dependencies.\n".into();
    fs::write(path, document.render().unwrap()).unwrap();
    let mut request = input("fixture");
    request.record_types = vec![RecordType::Synthesis];
    request.node_types = vec!["Model".into()];
    request.source_ids = vec![snapshot.sources[0].manifest.id.clone()];
    request.tags = vec!["fixture".into()];
    let result = search::execute(&workspace, &mut store, &request).unwrap();
    assert_eq!(result.groups.syntheses.len(), 1);
    assert_eq!(
        result.groups.syntheses[0]
            .freshness
            .as_ref()
            .unwrap()
            .evidence_ids
            .len(),
        1
    );
}

#[test]
fn real_search_pages_are_stable_and_expire_when_the_index_or_rank_settings_change() {
    let (temp, workspace, mut store, _) = fixture();
    let mut request = input("fixture");
    request.limit = Some(1);
    let first = search::execute(&workspace, &mut store, &request).unwrap();
    let cursor = first.next_cursor.clone().unwrap();
    let mut all = ids(&first);
    request.cursor = first.next_cursor;
    while request.cursor.is_some() {
        let page = search::execute(&workspace, &mut store, &request).unwrap();
        all.extend(ids(&page));
        request.cursor = page.next_cursor;
    }
    assert_eq!(all.len(), 4);
    assert_eq!(all.iter().collect::<BTreeSet<_>>().len(), 4);
    request.cursor = Some(cursor.clone());
    request.tags = vec!["other".into()];
    assert_eq!(
        search::execute(&workspace, &mut store, &request)
            .unwrap_err()
            .code,
        "CURSOR_QUERY_MISMATCH"
    );
    request.tags.clear();
    let path = temp.path().join("knowmesh.yaml");
    let mut config = serde_json::to_value(&workspace.config).unwrap();
    config["search"]["word_weight"] = 1.5.into();
    fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
    let workspace = Workspace::load(temp.path()).unwrap();
    assert_eq!(
        search::execute(&workspace, &mut store, &request)
            .unwrap_err()
            .code,
        "CURSOR_STALE"
    );
    request.cursor = None;
    let current = search::execute(&workspace, &mut store, &request).unwrap();
    request.cursor = current.next_cursor;
    let path = temp.path().join("knowledge/nodes/model-a.md");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str("\nExternal note.\n");
    fs::write(path, text).unwrap();
    assert_eq!(
        search::execute(&workspace, &mut store, &request)
            .unwrap_err()
            .code,
        "CURSOR_STALE"
    );
}

#[test]
fn a_failed_trigram_channel_is_disclosed_renormalized_and_expires_existing_cursors() {
    let (_temp, workspace, mut store, _) = fixture();
    let mut request = input("fixture");
    request.record_types = vec![RecordType::Node];
    request.limit = Some(1);
    let before = search::execute(&workspace, &mut store, &request).unwrap();
    let db = rusqlite::Connection::open(store.path()).unwrap();
    db.execute_batch("DROP TABLE search_fts_tri").unwrap();
    let after = search::execute(&workspace, &mut store, &request).unwrap();
    assert!(after.capabilities.word_fts);
    assert!(!after.capabilities.trigram_fts);
    let explain = after.groups.knowledge[0].explain.as_ref().unwrap();
    assert!((explain.normalization_bound - 1.0 / 61.0).abs() < 1e-12);
    assert!((explain.normalized_score - 1.0).abs() < 1e-12);
    assert!(
        after
            .channels
            .iter()
            .any(|channel| channel.unavailable_reason.as_deref()
                == Some("DATABASE_OPERATION_FAILED"))
    );
    request.cursor = before.next_cursor;
    assert_eq!(
        search::execute(&workspace, &mut store, &request)
            .unwrap_err()
            .code,
        "CURSOR_STALE"
    );
}

#[test]
fn search_preserves_historical_evidence_and_discloses_incomplete_or_removed_dependencies() {
    let (temp, workspace, mut store, _) = fixture();
    let path = temp.path().join("sources/fixture/source.yaml");
    let mut source: SourceManifest = serde_yaml::from_slice(&fs::read(&path).unwrap()).unwrap();
    source.removed_at = Some(Timestamp::now());
    fs::write(path, serde_yaml::to_string(&source).unwrap()).unwrap();
    let mut query = input("Model");
    query.no_sync = true;
    let stale = search::execute(&workspace, &mut store, &query).unwrap();
    assert!(!stale.index_complete);
    let freshness = stale.groups.claims[0].freshness.as_ref().unwrap();
    assert_eq!(freshness.freshness, Freshness::Unknown);
    assert_eq!(freshness.evidence_ids.len(), 1);
    assert!(freshness.current_evidence_ids.is_empty());
    query.no_sync = false;
    let synced = search::execute(&workspace, &mut store, &query).unwrap();
    let freshness = synced.groups.claims[0].freshness.as_ref().unwrap();
    assert_eq!(freshness.freshness, Freshness::NeedsReview);
    assert_eq!(freshness.evidence_ids.len(), 1);
    assert!(freshness.current_evidence_ids.is_empty());
    assert!(
        synced
            .groups
            .knowledge
            .iter()
            .any(|hit| hit.freshness.as_ref().unwrap().freshness == Freshness::NeedsReview)
    );
    assert_eq!(
        synced.groups.syntheses[0]
            .freshness
            .as_ref()
            .unwrap()
            .freshness,
        Freshness::NeedsReview
    );
}
