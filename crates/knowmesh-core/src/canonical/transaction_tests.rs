use std::{fs, path::PathBuf};

use super::transaction::{FileChange, TransactionState, WorkspaceWriter};
use crate::{
    domain::sha256,
    error::{AppError, ErrorType},
};

fn change(path: &str, before: Option<&str>, after: Option<&str>) -> FileChange {
    FileChange {
        path: PathBuf::from(path),
        before_sha256: before.map(|s| sha256(s.as_bytes())),
        content: after.map(|s| s.as_bytes().to_vec()),
    }
}

#[test]
fn file_transaction_rejects_conflicts_and_parallel_writers_before_changes() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("purpose.md"), "human text").unwrap();
    let writer = WorkspaceWriter::acquire(temp.path()).unwrap();
    assert_eq!(
        WorkspaceWriter::acquire(temp.path()).unwrap_err().code,
        "WORKSPACE_LOCKED"
    );
    assert_eq!(
        writer
            .prepare(vec![
                change("knowmesh.yaml", None, Some("new")),
                change("purpose.md", Some("old"), Some("new"))
            ])
            .unwrap_err()
            .code,
        "CANONICAL_FILE_CONFLICT"
    );
    assert!(!temp.path().join("knowmesh.yaml").exists());
    assert!(writer.pending().unwrap().is_empty());
}

#[test]
fn every_replacement_crash_point_can_roll_forward_and_index_once() {
    for stop_after in 0..=3 {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("knowledge/nodes")).unwrap();
        fs::write(temp.path().join("knowledge/nodes/old.md"), "before").unwrap();
        fs::write(temp.path().join("purpose.md"), "remove me").unwrap();
        let id;
        {
            let writer = WorkspaceWriter::acquire(temp.path()).unwrap();
            id = writer
                .prepare(vec![
                    change("knowledge/nodes/old.md", Some("before"), Some("after")),
                    change("knowledge/nodes/new.md", None, Some("created")),
                    change("purpose.md", Some("remove me"), None),
                ])
                .unwrap();
            if stop_after > 0 {
                let error = writer
                    .apply_observed(&id, |count| {
                        if count == stop_after {
                            Err(AppError::new(
                                ErrorType::Internal,
                                "INJECTED_CRASH",
                                "Fixture interruption.",
                            ))
                        } else {
                            Ok(())
                        }
                    })
                    .unwrap_err();
                assert_eq!(error.code, "INJECTED_CRASH");
            }
        }
        let writer = WorkspaceWriter::acquire(temp.path()).unwrap();
        assert_eq!(writer.pending().unwrap().len(), 1);
        assert_eq!(
            writer
                .prepare(vec![change("other.md", None, Some("new"))])
                .unwrap_err()
                .code,
            "TRANSACTION_RECOVERY_REQUIRED"
        );
        let report = writer.apply(&id).unwrap();
        assert_eq!(report.state, TransactionState::CanonicalCommitted);
        assert_eq!(
            fs::read_to_string(temp.path().join("knowledge/nodes/old.md")).unwrap(),
            "after"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("knowledge/nodes/new.md")).unwrap(),
            "created"
        );
        assert!(!temp.path().join("purpose.md").exists());
        writer.mark_indexed(&id).unwrap();
        writer.mark_indexed(&id).unwrap();
        assert!(writer.pending().unwrap().is_empty());
        assert!(!temp.path().join(".knowmesh/staging").join(&id).exists());
    }
}

#[test]
fn recovery_preserves_external_edits_and_all_recovery_material() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("purpose.md"), "old").unwrap();
    let writer = WorkspaceWriter::acquire(temp.path()).unwrap();
    let id = writer
        .prepare(vec![
            change("new.md", None, Some("new")),
            change("purpose.md", Some("old"), Some("after")),
        ])
        .unwrap();
    fs::write(temp.path().join("purpose.md"), "external edit").unwrap();
    assert_eq!(
        writer.apply(&id).unwrap_err().code,
        "TRANSACTION_RECOVERY_CONFLICT"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("purpose.md")).unwrap(),
        "external edit"
    );
    assert!(!temp.path().join("new.md").exists());
    assert!(
        temp.path()
            .join(".knowmesh/transactions")
            .join(&id)
            .join("manifest.json")
            .exists()
    );
    assert!(temp.path().join(".knowmesh/staging").join(&id).exists());
}

#[test]
fn staged_corruption_and_manifest_traversal_cannot_replace_canonical_content() {
    let temp = tempfile::tempdir().unwrap();
    let writer = WorkspaceWriter::acquire(temp.path()).unwrap();
    let id = writer
        .prepare(vec![change("purpose.md", None, Some("after"))])
        .unwrap();
    fs::write(
        temp.path()
            .join(".knowmesh/staging")
            .join(&id)
            .join("0.blob"),
        "corrupt",
    )
    .unwrap();
    assert_eq!(
        writer.apply(&id).unwrap_err().code,
        "TRANSACTION_STAGING_CORRUPT"
    );
    assert!(!temp.path().join("purpose.md").exists());
    assert_eq!(
        writer.apply("../other").unwrap_err().code,
        "INVALID_TRANSACTION_ID"
    );
}

#[test]
fn staging_is_revalidated_at_each_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let writer = WorkspaceWriter::acquire(temp.path()).unwrap();
    let id = writer
        .prepare(vec![
            change("first.md", None, Some("first")),
            change("second.md", None, Some("second")),
        ])
        .unwrap();
    let result = writer.apply_observed(&id, |count| {
        if count == 1 {
            fs::write(
                temp.path()
                    .join(".knowmesh/staging")
                    .join(&id)
                    .join("1.blob"),
                "tampered",
            )
            .unwrap();
        }
        Ok(())
    });
    assert_eq!(result.unwrap_err().code, "TRANSACTION_STAGING_CORRUPT");
    assert!(!temp.path().join("second.md").exists());
}

#[test]
fn reserved_and_escaping_paths_and_repeated_targets_are_rejected() {
    for path in [
        "../outside",
        ".git/config",
        ".GIT/config",
        ".git./config",
        ".knowmesh/index.sqlite3",
        "file.md:stream",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let writer = WorkspaceWriter::acquire(temp.path()).unwrap();
        assert_eq!(
            writer
                .prepare(vec![change(path, None, Some("new"))])
                .unwrap_err()
                .code,
            "INVALID_CANONICAL_PATH"
        );
    }
    let temp = tempfile::tempdir().unwrap();
    let writer = WorkspaceWriter::acquire(temp.path()).unwrap();
    assert_eq!(
        writer
            .prepare(vec![
                change("purpose.md", None, Some("one")),
                change("purpose.md", None, Some("two"))
            ])
            .unwrap_err()
            .code,
        "DUPLICATE_TRANSACTION_PATH"
    );
}

#[cfg(unix)]
#[test]
fn transaction_paths_do_not_follow_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let outside = temp.path().join("outside");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("knowledge")).unwrap();
    let writer = WorkspaceWriter::acquire(&root).unwrap();
    assert_eq!(
        writer
            .prepare(vec![change("knowledge/new.md", None, Some("new"))])
            .unwrap_err()
            .code,
        "PATH_OUTSIDE_WORKSPACE"
    );
    assert!(!outside.join("new.md").exists());
}

#[test]
fn repeated_completion_cleans_staging_left_after_an_indexed_journal() {
    let temp = tempfile::tempdir().unwrap();
    let writer = WorkspaceWriter::acquire(temp.path()).unwrap();
    let id = writer
        .prepare(vec![change("purpose.md", None, Some("new"))])
        .unwrap();
    writer.apply(&id).unwrap();
    writer.mark_indexed(&id).unwrap();
    let staging = temp.path().join(".knowmesh/staging").join(&id);
    fs::create_dir(&staging).unwrap();
    fs::write(staging.join("0.blob"), "new").unwrap();
    writer.mark_indexed(&id).unwrap();
    assert!(!staging.exists());
}

#[test]
fn case_aliases_cannot_create_an_unrecoverable_transaction_on_windows() {
    let temp = tempfile::tempdir().unwrap();
    let writer = WorkspaceWriter::acquire(temp.path()).unwrap();
    assert_eq!(
        writer
            .prepare(vec![
                change("purpose.md", None, Some("one")),
                change("PURPOSE.md", None, Some("two"))
            ])
            .unwrap_err()
            .code,
        "DUPLICATE_TRANSACTION_PATH"
    );
}
