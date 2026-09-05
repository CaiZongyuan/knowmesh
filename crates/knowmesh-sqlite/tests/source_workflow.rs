#[path = "../../../tests/support/mod.rs"]
mod support;

use knowmesh_core::{
    application::{source, sync},
    canonical::{
        snapshot::CanonicalSnapshot,
        source::{ImportInput, SourceLibrary},
    },
    domain::StorageMode,
    error::{AppError, AppResult, ErrorType},
    ports::{ProjectionStore, ReconcileReport},
};
use knowmesh_sqlite::SqliteStore;

struct InterruptedStore<'a> {
    store: &'a mut SqliteStore,
    baseline: String,
    after_commit: bool,
}

impl ProjectionStore for InterruptedStore<'_> {
    fn reconcile(&mut self, snapshot: &CanonicalSnapshot) -> AppResult<ReconcileReport> {
        if snapshot.content_sha256 == self.baseline {
            return self.store.reconcile(snapshot);
        }
        if self.after_commit {
            self.store.reconcile(snapshot)?;
        }
        Err(AppError::new(
            ErrorType::Io,
            "INJECTED_DATABASE_FAILURE",
            "Injected failure",
        ))
    }
}

#[test]
fn source_import_preview_commit_and_recovery_share_the_same_transaction_path() {
    let (temp, workspace) = support::fixture();
    let path = temp.path().join(".knowmesh/index.sqlite3");
    let mut store = SqliteStore::open(&path).unwrap();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    store
        .bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    sync::synchronize(&workspace, &mut store).unwrap();
    let document = temp.path().join("second.md");
    std::fs::write(&document, "# Second source\n\nSynthetic research notes.\n").unwrap();
    let mut input = ImportInput {
        path: document,
        source_id: None,
        storage: Some(StorageMode::Managed),
        title: Some("Second source".into()),
        kind: "note".into(),
        tags: vec![],
        dry_run: true,
    };
    let preview = source::add(&workspace, &mut store, &input, None).unwrap();
    assert!(preview.import.dry_run);
    assert!(preview.projection.is_none());
    assert_eq!(SourceLibrary::new(&workspace).list(true).unwrap().len(), 1);
    assert_eq!(store.generation().unwrap(), 1);
    input.dry_run = false;
    let mut interrupted = InterruptedStore { store: &mut store, baseline: snapshot.content_sha256, after_commit: false };
    assert_eq!(
        source::add(&workspace, &mut interrupted, &input, None)
            .unwrap_err()
            .code,
        "INJECTED_DATABASE_FAILURE"
    );
    assert_eq!(store.generation().unwrap(), 1);
    let pending = sync::recovery_status(&workspace).unwrap();
    assert!(pending.recovery_required);
    assert_eq!(pending.transactions.len(), 1);
    assert_eq!(pending.transactions[0].state, "canonical_committed");
    assert_eq!(
        sync::synchronize(&workspace, &mut store).unwrap_err().code,
        "TRANSACTION_RECOVERY_REQUIRED"
    );
    let recovered = sync::recover(&workspace, &mut store).unwrap();
    assert!(!recovered.recovery_required);
    assert_eq!(recovered.recovered_transaction_ids.len(), 1);
    assert_eq!(store.generation().unwrap(), 2);
    assert_eq!(SourceLibrary::new(&workspace).list(true).unwrap().len(), 2);
    sync::recover(&workspace, &mut store).unwrap();
    assert_eq!(store.generation().unwrap(), 2);
    let source_id = SourceLibrary::new(&workspace)
        .list(true)
        .unwrap()
        .into_iter()
        .find(|file| file.manifest.title == "Second source")
        .unwrap()
        .manifest
        .id;
    input.source_id = Some(source_id.clone());
    let repeated = source::add(&workspace, &mut store, &input, None).unwrap();
    assert!(repeated.import.deduplicated);
    assert_eq!(store.generation().unwrap(), 2);
    source::remove(
        &workspace,
        &mut store,
        &source::RemoveInput {
            source_id: source_id.clone(),
            dry_run: false,
        },
    )
    .unwrap();
    let removed = SourceLibrary::new(&workspace).get(&source_id).unwrap();
    assert!(removed.manifest.removed_at.is_some());
    assert!(
        SourceLibrary::new(&workspace)
            .content(&removed.manifest, &removed.manifest.current_revision_id)
            .is_ok()
    );
    assert_eq!(store.generation().unwrap(), 3);
}

#[test]
fn an_external_edit_after_interruption_preserves_the_journal_and_old_index() {
    let (temp, workspace) = support::fixture();
    let mut store = SqliteStore::open(&temp.path().join(".knowmesh/index.sqlite3")).unwrap();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    store
        .bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    sync::synchronize(&workspace, &mut store).unwrap();
    let id = snapshot.sources[0].manifest.id.clone();
    let mut interrupted = InterruptedStore { store: &mut store, baseline: snapshot.content_sha256, after_commit: false };
    source::remove(
        &workspace,
        &mut interrupted,
        &source::RemoveInput {
            source_id: id,
            dry_run: false,
        },
    )
    .unwrap_err();
    let path = temp.path().join(&snapshot.sources[0].manifest_path);
    let content = std::fs::read_to_string(&path).unwrap() + "\n# External edit\n";
    std::fs::write(&path, &content).unwrap();
    assert_eq!(
        sync::recover(&workspace, &mut store).unwrap_err().code,
        "TRANSACTION_RECOVERY_CONFLICT"
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), content);
    assert!(sync::recovery_status(&workspace).unwrap().recovery_required);
    assert_eq!(store.generation().unwrap(), 1);
}

#[test]
fn failure_after_database_commit_finishes_recovery_without_advancing_generation() {
    let (temp, workspace) = support::fixture();
    let mut store = SqliteStore::open(&temp.path().join(".knowmesh/index.sqlite3")).unwrap();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    store.bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash).unwrap();
    sync::synchronize(&workspace, &mut store).unwrap();
    let mut interrupted = InterruptedStore { store: &mut store, baseline: snapshot.content_sha256, after_commit: true };
    source::remove(&workspace, &mut interrupted, &source::RemoveInput { source_id: snapshot.sources[0].manifest.id.clone(), dry_run: false }).unwrap_err();
    assert_eq!(store.generation().unwrap(), 2);
    assert!(sync::recovery_status(&workspace).unwrap().recovery_required);
    let report = sync::recover(&workspace, &mut store).unwrap();
    assert!(!report.recovery_required);
    assert!(!report.projection.unwrap().changed);
    assert_eq!(store.generation().unwrap(), 2);
}

#[test]
fn an_incompatible_store_is_rejected_before_canonical_files_are_changed() {
    let (temp, workspace) = support::fixture();
    let mut store = SqliteStore::open(&temp.path().join(".knowmesh/index.sqlite3")).unwrap();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    store.bind_workspace(&knowmesh_core::domain::WorkspaceId::new(), &snapshot.schema_hash).unwrap();
    let input = source::RemoveInput { source_id: snapshot.sources[0].manifest.id.clone(), dry_run: false };
    assert_eq!(source::remove(&workspace, &mut store, &input).unwrap_err().code, "WORKSPACE_ID_MISMATCH");
    assert!(!sync::recovery_status(&workspace).unwrap().recovery_required);
    assert!(SourceLibrary::new(&workspace).get(&input.source_id).unwrap().manifest.removed_at.is_none());
}
