use knowmesh_core::domain::{WorkspaceId, sha256};
use knowmesh_sqlite::SqliteStore;
use rusqlite::Connection;

#[test]
fn read_only_open_never_creates_or_upgrades_a_database() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing.sqlite3");
    assert!(SqliteStore::open_read_only(&missing).is_err());
    assert!(!missing.exists());
    let path = temp.path().join("legacy.sqlite3");
    let db = Connection::open(&path).unwrap();
    db.execute_batch(include_str!("../migrations/0001_initial.sql"))
        .unwrap();
    db.execute(
        "INSERT INTO schema_migrations VALUES (1,'initial','2026-09-05T00:00:00Z',?1)",
        [sha256(include_bytes!("../migrations/0001_initial.sql"))],
    )
    .unwrap();
    db.pragma_update(None, "user_version", 1).unwrap();
    drop(db);
    let before = std::fs::read(&path).unwrap();
    assert_eq!(
        SqliteStore::open_read_only(&path).unwrap_err().code,
        "DATABASE_UPGRADE_REQUIRED"
    );
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn database_bootstrap_enables_wal_constraints_and_reopens_idempotently() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("index.sqlite3");
    let workspace = WorkspaceId::new();
    let store = SqliteStore::open(&path).unwrap();
    let diagnostics = store.diagnostics().unwrap();
    assert_eq!(diagnostics.journal_mode, "wal");
    assert!(diagnostics.foreign_keys);
    assert_eq!(diagnostics.busy_timeout_ms, 5000);
    assert_eq!(diagnostics.schema_version, 5);
    assert_eq!(diagnostics.integrity, "ok");
    store
        .bind_workspace(&workspace, &sha256(b"schema"))
        .unwrap();
    drop(store);
    let store = SqliteStore::open(&path).unwrap();
    store
        .bind_workspace(&workspace, &sha256(b"schema"))
        .unwrap();
    assert_eq!(
        store
            .bind_workspace(&WorkspaceId::new(), &sha256(b"schema"))
            .unwrap_err()
            .code,
        "WORKSPACE_ID_MISMATCH"
    );
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        5
    );
}

#[test]
fn old_database_migrates_without_losing_existing_rows() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("index.sqlite3");
    {
        let db = Connection::open(&path).unwrap();
        db.execute_batch(include_str!("../migrations/0001_initial.sql"))
            .unwrap();
        db.execute(
            "INSERT INTO schema_migrations VALUES (1, 'initial', '2026-09-05T00:00:00Z', ?1)",
            [sha256(include_bytes!("../migrations/0001_initial.sql"))],
        )
        .unwrap();
        db.pragma_update(None, "user_version", 1).unwrap();
        db.execute("INSERT INTO sources (id,slug,kind,title,storage_mode,manifest_path,status,created_at,updated_at) VALUES ('fixture','fixture','paper','Preserved','managed','sources/fixture/source.yaml','registered','2026-09-05T00:00:00Z','2026-09-05T00:00:00Z')", []).unwrap();
    }
    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(store.diagnostics().unwrap().schema_version, 5);
    let db = Connection::open(path).unwrap();
    assert_eq!(
        db.query_row("SELECT title FROM sources WHERE id='fixture'", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap(),
        "Preserved"
    );
}

#[test]
fn unknown_versions_and_changed_migration_checksums_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("index.sqlite3");
    drop(SqliteStore::open(&path).unwrap());
    let db = Connection::open(&path).unwrap();
    db.pragma_update(None, "user_version", 999).unwrap();
    assert_eq!(
        SqliteStore::open(&path).unwrap_err().code,
        "UNSUPPORTED_DATABASE_VERSION"
    );
    db.pragma_update(None, "user_version", 5).unwrap();
    db.execute(
        "UPDATE schema_migrations SET checksum='changed' WHERE version=1",
        [],
    )
    .unwrap();
    assert_eq!(
        SqliteStore::open(&path).unwrap_err().code,
        "MIGRATION_CHECKSUM_MISMATCH"
    );
}

#[test]
fn both_fts_indexes_follow_insert_update_and_delete() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("index.sqlite3");
    drop(SqliteStore::open(&path).unwrap());
    let db = Connection::open(path).unwrap();
    db.execute("INSERT INTO search_units(unit_id,record_type,record_id,title,body,content_sha256,updated_at) VALUES ('unit','node','node','Perturbation','originaltoken',?1,'2026-09-05T00:00:00Z')", [sha256(b"original")]).unwrap();
    for table in ["search_fts_word", "search_fts_tri"] {
        assert_eq!(count_matches(&db, table, "originaltoken"), 1);
    }
    db.execute(
        "UPDATE search_units SET body='replacementtoken' WHERE unit_id='unit'",
        [],
    )
    .unwrap();
    for table in ["search_fts_word", "search_fts_tri"] {
        assert_eq!(count_matches(&db, table, "originaltoken"), 0);
        assert_eq!(count_matches(&db, table, "replacementtoken"), 1);
    }
    db.execute("DELETE FROM search_units", []).unwrap();
    for table in ["search_fts_word", "search_fts_tri"] {
        assert_eq!(count_matches(&db, table, "replacementtoken"), 0);
    }
}

#[test]
fn opening_an_up_to_date_store_reads_wal_without_waiting_for_a_writer() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("index.sqlite3");
    let id = WorkspaceId::new();
    let store = SqliteStore::open(&path).unwrap();
    store.bind_workspace(&id, &sha256(b"schema")).unwrap();
    let writer = Connection::open(&path).unwrap();
    writer
        .execute_batch(
            "BEGIN IMMEDIATE; UPDATE workspace_state SET indexed_generation=1 WHERE singleton=1;",
        )
        .unwrap();
    let started = std::time::Instant::now();
    let reader = SqliteStore::open(&path).unwrap();
    reader.bind_workspace(&id, &sha256(b"schema")).unwrap();
    assert_eq!(reader.generation().unwrap(), 0);
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    writer.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn a_non_knowmesh_database_is_preserved_and_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("other.sqlite3");
    let db = Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TABLE unrelated(value TEXT); INSERT INTO unrelated VALUES ('human data');",
    )
    .unwrap();
    assert_eq!(
        SqliteStore::open(&path).unwrap_err().code,
        "MIGRATION_HISTORY_INVALID"
    );
    assert_eq!(
        db.query_row("SELECT value FROM unrelated", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "human data"
    );
}

fn count_matches(db: &Connection, table: &str, query: &str) -> i64 {
    db.query_row(
        &format!("SELECT count(*) FROM {table} WHERE {table} MATCH ?1"),
        [query],
        |r| r.get(0),
    )
    .unwrap()
}
