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
