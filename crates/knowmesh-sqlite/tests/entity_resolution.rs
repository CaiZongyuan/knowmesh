#[path = "../../../tests/support/mod.rs"]
mod support;

use knowmesh_core::{
    application::entity_resolution::{
        EntityInput, ResolutionDecision, ResolutionOptions, resolve_batch,
    },
    canonical::{snapshot::CanonicalSnapshot, workspace::Workspace},
    ports::ProjectionStore,
};
use knowmesh_sqlite::SqliteStore;

fn fixture() -> (tempfile::TempDir, Workspace, SqliteStore) {
    let (temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let mut store = SqliteStore::open(&workspace.index_path().unwrap()).unwrap();
    store
        .bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    store.reconcile(&snapshot).unwrap();
    (temp, workspace, store)
}

fn entity(name: &str) -> EntityInput {
    EntityInput {
        name: name.into(),
        node_type: "Model".into(),
        aliases: vec![],
        properties: Default::default(),
    }
}

#[test]
fn real_entity_retrieval_uses_title_alias_candidates_and_one_batch_snapshot() {
    let (_temp, workspace, mut store) = fixture();
    let report = resolve_batch(
        &workspace,
        &mut store,
        &[entity("Model"), entity("fictional")],
        &Default::default(),
    )
    .unwrap();
    assert_eq!(report.workspace_id, workspace.config.workspace.id);
    assert!(report.generation > 0);
    assert_eq!(report.snapshot_sha256.len(), 64);
    let matching = &report.results[0];
    assert_eq!(matching.decision, ResolutionDecision::Existing);
    assert!(!matching.automatic);
    assert_eq!(matching.candidates[0].name, "Model A");
    assert!(matching.candidates[0].retrieval_score.is_some());
    assert!(
        matching.candidates[0]
            .matched_by
            .iter()
            .any(|reason| reason == "fts")
    );
    assert!(matching.retrieval_available);
    assert!(
        matching
            .warnings
            .iter()
            .any(|warning| warning == "VECTOR_DISABLED")
    );
    assert_eq!(report.results[1].decision, ResolutionDecision::New);
    assert!(
        report.results[1].candidates.is_empty(),
        "body-only matches must not become entity candidates"
    );
    assert_eq!(matching.catalog_sha256, report.results[1].catalog_sha256);
}

#[test]
fn lexical_filters_and_display_limits_cannot_hide_deterministic_ambiguity() {
    let (_temp, workspace, mut store) = fixture();
    let report = resolve_batch(
        &workspace,
        &mut store,
        &[entity("Shared alias")],
        &ResolutionOptions {
            candidate_limit: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(report.results[0].decision, ResolutionDecision::Ambiguous);
    assert_eq!(report.results[0].selected_node_id, None);
    assert!(!report.results[0].automatic);
    assert_eq!(report.results[0].total_candidates, 2);
    assert!(report.results[0].candidates_truncated);
}

#[test]
fn incomplete_catalogs_and_empty_batches_fail_instead_of_claiming_uniqueness() {
    let (_temp, workspace, mut store) = fixture();
    assert_eq!(
        resolve_batch(
            &workspace,
            &mut store,
            &[entity("Model")],
            &ResolutionOptions {
                max_catalog_nodes: 1,
                ..Default::default()
            },
        )
        .unwrap_err()
        .code,
        "ENTITY_CATALOG_LIMIT"
    );
    assert_eq!(
        resolve_batch(&workspace, &mut store, &[], &Default::default())
            .unwrap_err()
            .code,
        "INVALID_ENTITY_BATCH"
    );
}

#[test]
fn near_ranked_candidates_remain_ambiguous_and_source_updates_refresh_the_catalog() {
    use knowmesh_core::{canonical::node::NodeDocument, domain::NodeId};
    use std::fs;

    let (temp, workspace, mut store) = fixture();
    let before = resolve_batch(
        &workspace,
        &mut store,
        &[entity("Model")],
        &Default::default(),
    )
    .unwrap();
    let path = temp.path().join("knowledge/nodes/model-a.md");
    let original = NodeDocument::parse(&fs::read_to_string(path).unwrap()).unwrap();
    let mut metadata = original.metadata.clone();
    metadata.id = NodeId::new();
    metadata.name = "Model C".into();
    metadata.aliases = vec![];
    let new_node = NodeDocument::create(metadata, "## Summary\n\nAnother model.").unwrap();
    fs::write(
        temp.path().join("knowledge/nodes/model-c.md"),
        new_node.render().unwrap(),
    )
    .unwrap();
    let after = resolve_batch(
        &workspace,
        &mut store,
        &[entity("Model")],
        &Default::default(),
    )
    .unwrap();
    assert!(after.generation > before.generation);
    assert_ne!(after.snapshot_sha256, before.snapshot_sha256);
    assert_ne!(
        after.results[0].catalog_sha256,
        before.results[0].catalog_sha256
    );
    assert_eq!(after.results[0].decision, ResolutionDecision::Ambiguous);
    assert_eq!(after.results[0].selected_node_id, None);
    assert_eq!(after.results[0].total_candidates, 2);
    assert!(!after.results[0].automatic);
}

#[test]
fn failed_fts_channels_are_disclosed_and_change_retrieval_identity() {
    let (_temp, workspace, mut store) = fixture();
    let before = resolve_batch(
        &workspace,
        &mut store,
        &[entity("Model")],
        &Default::default(),
    )
    .unwrap();
    let connection = rusqlite::Connection::open(workspace.index_path().unwrap()).unwrap();
    connection
        .execute_batch("DROP TABLE search_fts_tri")
        .unwrap();
    let after = resolve_batch(
        &workspace,
        &mut store,
        &[entity("Model")],
        &Default::default(),
    )
    .unwrap();
    assert_eq!(after.generation, before.generation);
    assert_eq!(
        after.results[0].catalog_sha256,
        before.results[0].catalog_sha256
    );
    assert_ne!(
        after.results[0].retrieval_sha256,
        before.results[0].retrieval_sha256
    );
    assert!(
        after.results[0]
            .warnings
            .iter()
            .any(|warning| warning == "ENTITY_TRIGRAM_FTS_UNAVAILABLE")
    );
    assert!(after.results[0].retrieval_available);
    assert!(!after.results[0].automatic);
    connection
        .execute_batch("DROP TABLE search_fts_word")
        .unwrap();
    let unavailable = resolve_batch(
        &workspace,
        &mut store,
        &[entity("Model")],
        &Default::default(),
    )
    .unwrap();
    assert!(!unavailable.results[0].retrieval_available);
    assert!(!unavailable.results[0].automatic);
}

#[test]
fn short_unicode_alias_retrieval_and_literal_operators_do_not_search_node_bodies() {
    use knowmesh_core::canonical::node::NodeDocument;
    use std::fs;

    let (temp, workspace, mut store) = fixture();
    let path = temp.path().join("knowledge/nodes/model-a.md");
    let mut doc = NodeDocument::parse(&fs::read_to_string(&path).unwrap()).unwrap();
    doc.metadata
        .aliases
        .extend(["小模型".into(), "Literal OR Token".into()]);
    fs::write(path, doc.render().unwrap()).unwrap();
    let report = resolve_batch(
        &workspace,
        &mut store,
        &[entity("小模"), entity("OR"), entity("missing OR Model")],
        &Default::default(),
    )
    .unwrap();
    for result in &report.results[..2] {
        assert_eq!(result.decision, ResolutionDecision::Existing);
        assert!(!result.automatic);
        assert!(
            result.candidates[0]
                .matched_by
                .iter()
                .any(|reason| reason == "fts")
        );
    }
    assert!(report.results[2].candidates.is_empty());
}

#[test]
fn mixed_snapshot_and_changed_candidate_metadata_fail_at_the_core_boundary() {
    use knowmesh_core::{
        application::entity_resolution::{EntityBatchData, EntityBatchQuery},
        error::AppResult,
        ports::{
            DatabaseDiagnostics, EntityResolutionStore, IndexStore, ProjectionState,
            ReconcileReport,
        },
    };

    struct ChangedStore {
        store: SqliteStore,
        mode: u8,
    }
    impl ProjectionStore for ChangedStore {
        fn reconcile(&mut self, snapshot: &CanonicalSnapshot) -> AppResult<ReconcileReport> {
            self.store.reconcile(snapshot)
        }
    }
    impl IndexStore for ChangedStore {
        fn projection_state(&self) -> AppResult<ProjectionState> {
            self.store.projection_state()
        }
        fn diagnostics(&self) -> AppResult<DatabaseDiagnostics> {
            self.store.diagnostics()
        }
    }
    impl EntityResolutionStore for ChangedStore {
        fn entity_resolution_data(&self, query: &EntityBatchQuery) -> AppResult<EntityBatchData> {
            let mut data = self.store.entity_resolution_data(query)?;
            match self.mode {
                0 => data.lexical[0].generation += 1,
                1 => data.generation += 1,
                2 => {
                    for channel in &mut data.lexical[0].channels {
                        channel.hits[0].title = "Different title".into();
                    }
                }
                _ => data.schema_hash = "different".into(),
            }
            Ok(data)
        }
    }
    for mode in 0..4 {
        let (_temp, workspace, store) = fixture();
        let mut store = ChangedStore { store, mode };
        let result = resolve_batch(
            &workspace,
            &mut store,
            &[entity("Model")],
            &Default::default(),
        )
        .unwrap_err();
        assert_eq!(
            result.code,
            if mode == 1 {
                "ENTITY_INDEX_INCOMPLETE"
            } else {
                "ENTITY_CONTEXT_MISMATCH"
            }
        );
    }
}
