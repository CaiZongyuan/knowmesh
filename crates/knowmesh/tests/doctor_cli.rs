use std::{fs, path::Path, process::Command};

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;

fn doctor(root: &Path) -> Value {
    let output = cargo_bin_cmd!("knowmesh")
        .arg("--workspace")
        .arg(root)
        .arg("doctor")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["data"].clone()
}

#[test]
fn doctor_reports_missing_stale_and_invalid_content_without_changing_the_index() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    cargo_bin_cmd!("knowmesh")
        .arg("init")
        .arg(&root)
        .assert()
        .success();
    let initial = doctor(&root);
    assert!(
        initial["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["code"] == "INDEX_MISSING")
    );
    assert!(!root.join(".knowmesh/index.sqlite3").exists());
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["init", "-q"])
            .status()
            .unwrap()
            .success()
    );
    cargo_bin_cmd!("knowmesh")
        .arg("--workspace")
        .arg(&root)
        .arg("sync")
        .assert()
        .success();
    let healthy = doctor(&root);
    assert_eq!(healthy["healthy"], true);
    assert_eq!(healthy["database"]["integrity"], "ok");
    assert_eq!(healthy["sync_required"], false);
    assert_eq!(healthy["git"]["runtime_ignored"], true);
    let ignore_path = root.join(".gitignore");
    let original_ignore = fs::read_to_string(&ignore_path).unwrap();
    fs::write(&ignore_path, ".knowmesh/index.sqlite3\n").unwrap();
    assert_eq!(doctor(&root)["git"]["runtime_ignored"], false);
    fs::write(ignore_path, original_ignore).unwrap();
    let purpose = root.join("purpose.md");
    let text = fs::read_to_string(&purpose).unwrap();
    fs::write(purpose, text + "\nAdditional context.\n").unwrap();
    assert_eq!(doctor(&root)["sync_required"], true);
    cargo_bin_cmd!("knowmesh")
        .arg("--workspace")
        .arg(&root)
        .args(["doctor", "--repair", "--yes"])
        .assert()
        .success();
    assert_eq!(doctor(&root)["sync_required"], false);
    fs::write(root.join("knowledge/nodes/invalid.md"), "No frontmatter.").unwrap();
    let invalid = doctor(&root);
    assert_eq!(invalid["healthy"], false);
    assert!(
        invalid["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["severity"] == "error")
    );
}

#[test]
fn doctor_preserves_a_corrupt_database_and_requires_confirmation_for_repair() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    cargo_bin_cmd!("knowmesh")
        .arg("init")
        .arg(&root)
        .assert()
        .success();
    let unconfirmed = cargo_bin_cmd!("knowmesh")
        .arg("--workspace")
        .arg(&root)
        .args(["doctor", "--repair"])
        .output()
        .unwrap();
    assert!(!unconfirmed.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&unconfirmed.stderr).unwrap()["error"]["code"],
        "CONFIRMATION_REQUIRED"
    );
    assert!(!root.join(".knowmesh/index.sqlite3").exists());
    fs::write(root.join(".knowmesh/index.sqlite3"), b"corrupt fixture").unwrap();
    let report = doctor(&root);
    assert_eq!(report["healthy"], false);
    assert!(
        report["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["code"] == "DATABASE_CORRUPT")
    );
    assert_eq!(
        fs::read(root.join(".knowmesh/index.sqlite3")).unwrap(),
        b"corrupt fixture"
    );
}
