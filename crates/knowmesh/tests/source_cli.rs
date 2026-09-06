use std::{fs, path::Path};

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;

fn command(root: &Path, args: &[&str]) -> Value {
    let output = cargo_bin_cmd!("knowmesh")
        .arg("--workspace")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn source_and_sync_commands_preserve_dry_run_and_confirmation_contracts() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    cargo_bin_cmd!("knowmesh")
        .arg("init")
        .arg(&root)
        .assert()
        .success();
    let file = temp.path().join("notes.md");
    fs::write(&file, "# Notes\n\nSynthetic evidence.\n").unwrap();
    let file = file.to_str().unwrap();
    let preview = command(&root, &["source", "add", file, "--dry-run"]);
    assert_eq!(preview["data"]["dry_run"], true);
    assert!(!root.join(".knowmesh/index.sqlite3").exists());
    assert_eq!(fs::read_dir(root.join("sources")).unwrap().count(), 0);
    let sync_preview = command(&root, &["sync", "--dry-run"]);
    assert_eq!(sync_preview["data"]["dry_run"], true);
    assert!(!root.join(".knowmesh/index.sqlite3").exists());
    let added = command(&root, &["source", "add", file, "--title", "Research notes"]);
    assert_eq!(added["data"]["source"]["title"], "Research notes");
    assert_eq!(added["meta"]["command"], "source.add");
    let source_id = added["data"]["source"]["id"].as_str().unwrap();
    let generation = added["data"]["projection"]["generation"].as_u64().unwrap();
    let synced = command(&root, &["sync"]);
    assert_eq!(synced["data"]["projection"]["generation"], generation);
    assert_eq!(synced["data"]["projection"]["changed"], false);
    let duplicate = command(&root, &["source", "add", file, "--source-id", source_id]);
    assert_eq!(duplicate["data"]["deduplicated"], true);
    assert_eq!(duplicate["data"]["projection"]["generation"], generation);
    let unconfirmed = cargo_bin_cmd!("knowmesh")
        .arg("--workspace")
        .arg(&root)
        .args(["source", "remove", source_id])
        .output()
        .unwrap();
    assert!(!unconfirmed.status.success());
    assert!(unconfirmed.stdout.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&unconfirmed.stderr).unwrap()["error"]["code"],
        "CONFIRMATION_REQUIRED"
    );
    let removal = command(&root, &["source", "remove", source_id, "--dry-run"]);
    assert_eq!(removal["data"]["dry_run"], true);
    assert_eq!(removal["data"]["impact"]["preview"], true);
    assert_eq!(removal["data"]["impact"]["generation"], generation);
    assert_eq!(removal["data"]["impact"]["counts"]["claims"], 0);
    let removed = command(&root, &["source", "remove", source_id, "--yes"]);
    assert!(removed["data"]["source"]["removed_at"].is_string());
    assert_eq!(removed["data"]["projection"]["generation"], generation + 1);
    let repeated = command(&root, &["source", "remove", source_id, "--yes"]);
    assert_eq!(repeated["data"]["projection"]["generation"], generation + 1);
}

#[test]
fn source_and_sync_are_discoverable_with_application_owned_effects() {
    let temp = tempfile::tempdir().unwrap();
    for (operation, effect) in [
        ("source.add", "canonical-write"),
        ("source.remove", "canonical-write"),
        ("sync", "derived-write"),
    ] {
        let output = command(temp.path(), &["schema", "command", operation]);
        assert_eq!(output["data"]["effect"], effect);
        assert_eq!(output["data"]["supports_dry_run"], true);
    }
}
