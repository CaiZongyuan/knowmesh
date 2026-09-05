use std::{fs, path::Path};

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;

fn status(root: &Path, no_sync: bool) -> Value {
    let mut command = cargo_bin_cmd!("knowmesh");
    command.arg("--workspace").arg(root).arg("status");
    if no_sync { command.arg("--no-sync"); }
    let output = command.output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["data"].clone()
}

#[test]
fn status_fast_syncs_external_edits_and_no_sync_keeps_the_previous_generation() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    cargo_bin_cmd!("knowmesh").arg("init").arg(&root).assert().success();
    let first = status(&root, false);
    assert_eq!(first["projection"]["generation"], 1);
    assert_eq!(first["fast_path"], false);
    assert_eq!(status(&root, false)["fast_path"], true);
    let purpose = root.join("purpose.md");
    let text = fs::read_to_string(&purpose).unwrap();
    fs::write(purpose, text + "\nA new comparison dimension.\n").unwrap();
    let skipped = status(&root, true);
    assert_eq!(skipped["projection"]["generation"], 1);
    assert_eq!(skipped["sync_skipped"], "requested");
    let updated = status(&root, false);
    assert_eq!(updated["projection"]["generation"], 2);
    fs::write(root.join("schemas/research.yaml"), "invalid: schema\n").unwrap();
    let output = cargo_bin_cmd!("knowmesh").arg("--workspace").arg(&root).args(["status", "--no-sync"]).output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["type"], "validation");
}
