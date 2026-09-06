#[path = "support/architecture.rs"]
mod architecture;

use std::{fs, path::Path};

use serde_json::json;

#[test]
fn architecture_guard_rejects_compiler_writes_and_adapter_database_access() {
    for (path, source, code) in [
        (
            "crates/knowmesh-core/src/compiler/mod.rs",
            "use std::fs::write as publish; fn compile() { publish(\"knowledge/new.md\", \"accepted\"); }",
            "UNREGISTERED_FILE_WRITE",
        ),
        (
            "crates/knowmesh-core/src/compiler/mod.rs",
            "use crate::canonical::transaction::WorkspaceWriter as Writer; fn compile() { Writer::acquire(root); }",
            "CANONICAL_WRITER_ACCESS",
        ),
        (
            "crates/knowmesh-core/src/compiler/mod.rs",
            "use crate::ports::ProjectionStore; fn compile(store: &mut dyn ProjectionStore) { store.reconcile(snapshot); }",
            "PROJECTION_WRITE_CAPABILITY",
        ),
        (
            "crates/knowmesh/src/http/routes.rs",
            "use rusqlite::Connection as Database; fn route() { Database::open(path); }",
            "DATABASE_DRIVER_ACCESS",
        ),
        (
            "crates/knowmesh/src/cli.rs",
            "fn command() { knowmesh_sqlite::SqliteStore::open(path); }",
            "DATABASE_ADAPTER_ACCESS",
        ),
        (
            "crates/knowmesh-sqlite/src/extra.rs",
            "fn write(store: &Store) { store.connection.execute(\"DELETE FROM claims\", []); }",
            "UNREGISTERED_SQL_WRITE",
        ),
    ] {
        let violations = architecture::check_source(path, source);
        assert!(
            violations.iter().any(|violation| violation.code == code),
            "{path}: {violations:?}"
        );
    }
}

#[test]
fn architecture_guard_accepts_registered_writers_and_ignores_documentation_and_tests() {
    for (path, source) in [
        (
            "crates/knowmesh-sqlite/src/reconcile.rs",
            "fn reconcile(c: &Connection) { c.execute(\"INSERT INTO claims VALUES (?)\", [id]); }",
        ),
        (
            "crates/knowmesh-sqlite/src/migrations.rs",
            "fn migrate(c: &mut Connection) { c.execute_batch(sql); }",
        ),
        (
            "crates/knowmesh-core/src/canonical/transaction.rs",
            "fn commit() { std::fs::rename(staged, target); }",
        ),
        (
            "crates/knowmesh-core/src/compiler/mod.rs",
            "// std::fs::write(path, data)\nfn compile() { let note = \"rusqlite::Connection::open(path)\"; }\n#[cfg(test)] mod tests { fn fixture() { std::fs::write(path, bytes); } }",
        ),
    ] {
        assert!(
            architecture::check_source(path, source).is_empty(),
            "{path}"
        );
    }
}

#[test]
fn architecture_guard_rejects_public_connection_and_writer_exposure() {
    for (path, source) in [
        (
            "crates/knowmesh-sqlite/src/lib.rs",
            "pub struct SqliteStore { pub connection: rusqlite::Connection }",
        ),
        (
            "crates/knowmesh-core/src/canonical/transaction.rs",
            "pub struct WorkspaceWriter { root: PathBuf }",
        ),
    ] {
        assert!(
            architecture::check_source(path, source)
                .iter()
                .any(|violation| violation.code == "PUBLIC_MUTATOR_EXPOSURE")
        );
    }
}

#[test]
fn dependency_guard_uses_package_identity_even_for_renamed_dependencies() {
    let invalid = json!({"packages": [
        {"name":"knowmesh-core", "dependencies":[{"name":"rusqlite", "rename":"storage", "kind":null}]},
        {"name":"knowmesh-sqlite", "dependencies":[{"name":"knowmesh", "kind":null}]}
    ]});
    assert_eq!(architecture::check_dependencies(&invalid).len(), 2);
    let valid = json!({"packages": [
        {"name":"knowmesh", "dependencies":[{"name":"knowmesh-core", "kind":null}, {"name":"knowmesh-sqlite", "kind":null}]},
        {"name":"knowmesh-sqlite", "dependencies":[{"name":"knowmesh-core", "kind":null}, {"name":"rusqlite", "kind":null}]}
    ]});
    assert!(architecture::check_dependencies(&valid).is_empty());
}

#[test]
fn guard_checks_glob_imports_qualified_mutators_and_raw_connection_returns() {
    for (path, source, code) in [
        ("crates/knowmesh-sqlite/src/extra.rs", "fn write(c: &rusqlite::Connection) { rusqlite::Connection::execute(c, sql, []); }", "UNREGISTERED_SQL_WRITE"),
        ("crates/knowmesh-core/src/compiler/mod.rs", "use std::fs::*; fn compile() { write(path, bytes); }", "UNREGISTERED_FILE_WRITE"),
        ("crates/knowmesh-core/src/compiler/mod.rs", "use crate::canonical::transaction::*; fn compile() { WorkspaceWriter::acquire(root); }", "CANONICAL_WRITER_ACCESS"),
        ("crates/knowmesh-sqlite/src/lib.rs", "pub struct SqliteStore { pub raw: rusqlite::Connection }", "PUBLIC_MUTATOR_EXPOSURE"),
        ("crates/knowmesh-sqlite/src/lib.rs", "impl SqliteStore { pub fn raw(&self) -> &rusqlite::Connection { &self.connection } }", "PUBLIC_MUTATOR_EXPOSURE"),
    ] {
        assert!(architecture::check_source(path, source).iter().any(|violation| violation.code == code), "{path}: {source}");
    }
}

#[test]
fn production_module_discovery_follows_rust_modules_including_misleading_test_filenames() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("lib.rs"),
        "#[path=\"hidden_tests.rs\"] mod production; #[cfg(test)] mod absent_test_fixture;",
    )
    .unwrap();
    fs::write(
        temp.path().join("hidden_tests.rs"),
        "fn publish() { std::fs::write(path, content); }",
    )
    .unwrap();
    assert!(
        architecture::check_tree("knowmesh-core", temp.path())
            .iter()
            .any(|violation| violation.code == "UNREGISTERED_FILE_WRITE")
    );
}

#[test]
fn repository_respects_dependency_visibility_and_write_boundaries() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let violations = architecture::check_workspace(root);
    assert!(violations.is_empty(), "{violations:#?}");
}
