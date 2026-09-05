#[path = "support/runtime.rs"]
mod runtime_support;
#[path = "../../../tests/support/mod.rs"]
mod support;

use std::fs;

use knowmesh_core::{
    application::rebuild::{self, RebuildInput},
    canonical::snapshot::CanonicalSnapshot,
    domain::WorkspaceId,
    ports::{ProjectionStore, RebuildBackend},
};
use knowmesh_sqlite::{SqliteRebuilder, SqliteStore};

#[test]
fn rebuild_preserves_canonical_and_runtime_state_and_backs_up_the_previous_database() {
    let (temp, workspace) = support::fixture();
    let mut snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let path = workspace.index_path().unwrap();
    let mut store = SqliteStore::open(&path).unwrap();
    store
        .bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    store.reconcile(&snapshot).unwrap();
    let model_path = temp.path().join("knowledge/nodes/model-a.md");
    let content = fs::read_to_string(&model_path).unwrap();
    fs::write(
        model_path,
        content.replace("A fictional model.", "A revised fictional model."),
    )
    .unwrap();
    snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    store.reconcile(&snapshot).unwrap();
    let expected = store.logical_snapshot().unwrap();
    runtime_support::runtime_fixture(
        &path,
        snapshot.sources[0].manifest.current_revision_id.as_str(),
    );
    drop(store);
    let backend = SqliteRebuilder::new(&workspace).unwrap();
    let report = rebuild::execute(
        &workspace,
        &backend,
        &RebuildInput {
            yes: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(report.projection.generation, 2);
    assert_eq!(report.runtime_table_counts["operation_runs"], 2);
    assert!(!report.backup_paths.is_empty());
    assert!(!temp.path().join(".knowmesh/index.next.sqlite3").exists());
    let current = SqliteStore::open_read_only(&path).unwrap();
    assert_eq!(current.logical_snapshot().unwrap(), expected);
    assert_eq!(current.diagnostics().unwrap().foreign_key_violations, 0);
    let backup = SqliteStore::open_read_only(&report.backup_paths[0]).unwrap();
    assert_eq!(backup.logical_snapshot().unwrap(), expected);
    let db = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        db.query_row("SELECT response_json FROM idempotency_keys", [], |r| r
            .get::<_, String>(
            0
        ))
        .unwrap(),
        "{\"output\":\"proposal\"}"
    );
    assert_eq!(
        db.query_row(
            "SELECT seq FROM sqlite_sequence WHERE name='audit_events'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        90
    );
}

#[test]
fn rebuild_preview_and_missing_confirmation_do_not_create_database_files() {
    let (temp, workspace) = support::fixture();
    let backend = SqliteRebuilder::new(&workspace).unwrap();
    let report = rebuild::execute(
        &workspace,
        &backend,
        &RebuildInput {
            dry_run: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(report.dry_run);
    assert_eq!(report.projection.node_count, 2);
    assert!(!workspace.index_path().unwrap().exists());
    assert!(!temp.path().join(".knowmesh/index.next.sqlite3").exists());
    assert!(!temp.path().join(".knowmesh/backups").exists());
    assert_eq!(
        rebuild::execute(&workspace, &backend, &RebuildInput::default())
            .unwrap_err()
            .code,
        "CONFIRMATION_REQUIRED"
    );
}

#[test]
fn unreadable_runtime_is_preserved_unless_discard_is_explicit_and_backed_up() {
    let (temp, workspace) = support::fixture();
    let path = workspace.index_path().unwrap();
    fs::write(&path, b"corrupt fixture").unwrap();
    let backend = SqliteRebuilder::new(&workspace).unwrap();
    assert_eq!(
        rebuild::execute(
            &workspace,
            &backend,
            &RebuildInput {
                yes: true,
                ..Default::default()
            }
        )
        .unwrap_err()
        .code,
        "RUNTIME_PRESERVATION_FAILED"
    );
    assert_eq!(fs::read(&path).unwrap(), b"corrupt fixture");
    assert!(temp.path().join(".knowmesh/index.next.sqlite3").exists());
    let report = rebuild::execute(
        &workspace,
        &backend,
        &RebuildInput {
            yes: true,
            discard_runtime: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(report.discarded_runtime_tables.len(), 5);
    assert_eq!(
        fs::read(&report.backup_paths[0]).unwrap(),
        b"corrupt fixture"
    );
    assert_eq!(
        SqliteStore::open_read_only(&path)
            .unwrap()
            .diagnostics()
            .unwrap()
            .integrity,
        "ok"
    );
}

#[test]
fn active_writers_prevent_replacement_and_leave_both_complete_databases_available() {
    let (temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let path = workspace.index_path().unwrap();
    let mut writer = SqliteStore::open(&path).unwrap();
    writer
        .bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    writer.reconcile(&snapshot).unwrap();
    let expected = writer.logical_snapshot().unwrap();
    let backend = SqliteRebuilder::new(&workspace).unwrap();
    let input = RebuildInput {
        yes: true,
        ..Default::default()
    };
    assert_eq!(
        rebuild::execute(&workspace, &backend, &input)
            .unwrap_err()
            .code,
        "DATABASE_IN_USE"
    );
    assert_eq!(writer.logical_snapshot().unwrap(), expected);
    assert!(temp.path().join(".knowmesh/index.next.sqlite3").exists());
    drop(writer);
    rebuild::execute(&workspace, &backend, &input).unwrap();
    assert_eq!(
        SqliteStore::open_read_only(&path)
            .unwrap()
            .logical_snapshot()
            .unwrap(),
        expected
    );
}

#[test]
fn rebuilding_recopies_runtime_committed_after_preparation() {
    let (_temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let path = workspace.index_path().unwrap();
    let mut store = SqliteStore::open(&path).unwrap();
    store
        .bind_workspace(&snapshot.workspace_id, &snapshot.schema_hash)
        .unwrap();
    store.reconcile(&snapshot).unwrap();
    let backend = SqliteRebuilder::new(&workspace).unwrap();
    let input = RebuildInput {
        yes: true,
        ..Default::default()
    };
    let candidate = backend.prepare(&snapshot, &input).unwrap();
    runtime_support::runtime_fixture(
        &path,
        snapshot.sources[0].manifest.current_revision_id.as_str(),
    );
    drop(store);
    let report = candidate.publish(&snapshot).unwrap();
    assert_eq!(report.runtime_table_counts["operation_runs"], 2);
    assert_eq!(report.runtime_table_counts["audit_events"], 1);
    assert_eq!(report.projection.generation, 1);
    let db = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        db.query_row("SELECT response_json FROM idempotency_keys", [], |r| r
            .get::<_, String>(
            0
        ))
        .unwrap(),
        "{\"output\":\"proposal\"}"
    );
}

#[test]
fn preview_checks_runtime_references_without_creating_a_candidate() {
    let (temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let path = workspace.index_path().unwrap();
    let mut store = SqliteStore::open(&path).unwrap();
    store
        .bind_workspace(&snapshot.workspace_id, &snapshot.schema_hash)
        .unwrap();
    store.reconcile(&snapshot).unwrap();
    runtime_support::runtime_fixture(
        &path,
        snapshot.sources[0].manifest.current_revision_id.as_str(),
    );
    drop(store);
    fs::remove_dir_all(temp.path().join("sources/fixture")).unwrap();
    for file in [
        "knowledge/nodes/model-a.md",
        "knowledge/nodes/dataset-b.md",
        "knowledge/syntheses/comparison.md",
    ] {
        fs::remove_file(temp.path().join(file)).unwrap();
    }
    let backend = SqliteRebuilder::new(&workspace).unwrap();
    let input = RebuildInput {
        dry_run: true,
        ..Default::default()
    };
    assert_eq!(
        rebuild::execute(&workspace, &backend, &input)
            .unwrap_err()
            .code,
        "RUNTIME_REFERENCE_MISSING"
    );
    assert!(!temp.path().join(".knowmesh/index.next.sqlite3").exists());
    let report = rebuild::execute(
        &workspace,
        &backend,
        &RebuildInput {
            discard_runtime: true,
            ..input
        },
    )
    .unwrap();
    assert!(report.runtime_table_counts.is_empty());
    assert!(!report.backup_paths.is_empty());
    assert!(report.backup_paths.iter().all(|path| !path.exists()));
}

#[test]
fn discard_does_not_override_workspace_identity_or_version_checks() {
    let (_temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let path = workspace.index_path().unwrap();
    let store = SqliteStore::open(&path).unwrap();
    store
        .bind_workspace(&WorkspaceId::new(), &snapshot.schema_hash)
        .unwrap();
    drop(store);
    let backend = SqliteRebuilder::new(&workspace).unwrap();
    let input = RebuildInput {
        yes: true,
        discard_runtime: true,
        ..Default::default()
    };
    assert_eq!(
        rebuild::execute(&workspace, &backend, &input)
            .unwrap_err()
            .code,
        "WORKSPACE_ID_MISMATCH"
    );
    let db = rusqlite::Connection::open(path).unwrap();
    db.pragma_update(None, "user_version", 999).unwrap();
    drop(db);
    assert_eq!(
        rebuild::execute(&workspace, &backend, &input)
            .unwrap_err()
            .code,
        "UNSUPPORTED_DATABASE_VERSION"
    );
}

#[test]
fn stale_preparation_and_failed_backup_preserve_the_current_database() {
    let (temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let path = workspace.index_path().unwrap();
    let mut store = SqliteStore::open(&path).unwrap();
    store
        .bind_workspace(&snapshot.workspace_id, &snapshot.schema_hash)
        .unwrap();
    store.reconcile(&snapshot).unwrap();
    let backend = SqliteRebuilder::new(&workspace).unwrap();
    let input = RebuildInput {
        yes: true,
        ..Default::default()
    };
    let candidate = backend.prepare(&snapshot, &input).unwrap();
    let model = temp.path().join("knowledge/nodes/model-a.md");
    let content = fs::read_to_string(&model).unwrap();
    fs::write(
        model,
        content.replace("A fictional model.", "An updated model."),
    )
    .unwrap();
    let changed = CanonicalSnapshot::scan(&workspace).unwrap();
    store.reconcile(&changed).unwrap();
    let expected = store.logical_snapshot().unwrap();
    drop(store);
    assert_eq!(
        candidate.publish(&snapshot).unwrap_err().code,
        "REBUILD_GENERATION_CHANGED"
    );
    fs::write(
        temp.path().join(".knowmesh/backups"),
        "blocked backup directory",
    )
    .unwrap();
    assert_eq!(
        rebuild::execute(&workspace, &backend, &input)
            .unwrap_err()
            .code,
        "REBUILD_IO_FAILED"
    );
    assert_eq!(
        SqliteStore::open_read_only(&path)
            .unwrap()
            .logical_snapshot()
            .unwrap(),
        expected
    );
    assert!(temp.path().join(".knowmesh/index.next.sqlite3").exists());
    fs::remove_file(temp.path().join(".knowmesh/backups")).unwrap();
    let report = rebuild::execute(&workspace, &backend, &input).unwrap();
    assert!(!report.retained_candidate_paths.is_empty());
    assert_eq!(report.projection.generation, 2);
}

#[test]
fn backup_retention_uses_creation_time_and_preserves_unrecognized_materials() {
    let (temp, workspace) = support::fixture();
    let backend = SqliteRebuilder::new(&workspace).unwrap();
    let input = RebuildInput {
        yes: true,
        keep_backups: 1,
        ..Default::default()
    };
    rebuild::execute(&workspace, &backend, &input).unwrap();
    let previous = rebuild::execute(&workspace, &backend, &input).unwrap();
    let backups = temp.path().join(".knowmesh/backups");
    let renamed = backups.join("rebuild-zz-previous");
    fs::rename(previous.backup_paths[0].parent().unwrap(), &renamed).unwrap();
    let unrelated = backups.join("rebuild-unrecognized");
    fs::create_dir(&unrelated).unwrap();
    fs::write(unrelated.join("notes.txt"), "Keep this recovery note.").unwrap();
    let report = rebuild::execute(&workspace, &backend, &input).unwrap();
    assert!(report.backup_paths[0].exists());
    assert!(!renamed.exists());
    assert!(unrelated.join("notes.txt").exists());
}
