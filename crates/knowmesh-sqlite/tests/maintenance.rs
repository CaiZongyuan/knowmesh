use knowmesh_sqlite::SqliteStore;

#[test]
fn a_controlled_maintenance_connection_retains_the_exclusive_guard_until_it_closes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("index.sqlite3");
    drop(SqliteStore::open(&path).unwrap());
    let access = SqliteStore::exclusive_access(&path).unwrap();
    let store = access.open_store().unwrap();
    assert_eq!(store.diagnostics().unwrap().integrity, "ok");
    assert_eq!(access.ensure_quiescent().unwrap_err().code, "DATABASE_IN_USE");
    drop(access);
    assert_eq!(SqliteStore::open(&path).unwrap_err().code, "DATABASE_IN_USE");
    drop(store);
    let access = SqliteStore::exclusive_access(&path).unwrap();
    access.ensure_quiescent().unwrap();
}

#[test]
fn replacement_excludes_writable_connections_until_the_access_guard_is_dropped() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("index.sqlite3");
    let first = SqliteStore::open(&path).unwrap();
    let second = SqliteStore::open(&path).unwrap();
    assert_eq!(
        SqliteStore::exclusive_access(&path).unwrap_err().code,
        "DATABASE_IN_USE"
    );
    drop(first);
    assert_eq!(
        SqliteStore::exclusive_access(&path).unwrap_err().code,
        "DATABASE_IN_USE"
    );
    drop(second);
    let access = SqliteStore::exclusive_access(&path).unwrap();
    assert_eq!(
        SqliteStore::open(&path).unwrap_err().code,
        "DATABASE_IN_USE"
    );
    let reader = SqliteStore::open_read_only(&path).unwrap();
    assert_eq!(reader.diagnostics().unwrap().integrity, "ok");
    drop(reader);
    drop(access);
    assert!(SqliteStore::open(&path).is_ok());
}

#[test]
fn equivalent_paths_share_one_connection_guard_and_guarding_does_not_create_a_database() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("index.sqlite3");
    std::fs::create_dir(temp.path().join("nested")).unwrap();
    let access = SqliteStore::exclusive_access(&path).unwrap();
    assert!(!path.exists());
    let alias = temp.path().join("nested/../index.sqlite3");
    assert_eq!(
        SqliteStore::open(&alias).unwrap_err().code,
        "DATABASE_IN_USE"
    );
    drop(access);
    let store = SqliteStore::open(&alias).unwrap();
    assert_eq!(
        SqliteStore::exclusive_access(&path).unwrap_err().code,
        "DATABASE_IN_USE"
    );
    drop(store);
}
