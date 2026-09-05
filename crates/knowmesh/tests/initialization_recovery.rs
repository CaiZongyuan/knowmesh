use std::{fs, path::{Path, PathBuf}, process::Output};

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;

fn invoke(root: &Path, args: &[&str]) -> Output {
    cargo_bin_cmd!("knowmesh").arg("--workspace").arg(root).args(args).output().unwrap()
}

fn success(output: Output) -> Value {
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["data"].clone()
}

fn error(output: Output) -> String {
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"].as_str().unwrap().into()
}

fn interrupted(root: &Path, applied: usize) -> (PathBuf, Vec<(PathBuf, Vec<u8>)>) {
    success(cargo_bin_cmd!("knowmesh").arg("init").arg(root).output().unwrap());
    let directory = fs::read_dir(root.join(".knowmesh/transactions")).unwrap().next().unwrap().unwrap().path();
    let journal = directory.join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&journal).unwrap()).unwrap();
    let staging = root.join(".knowmesh/staging").join(manifest["id"].as_str().unwrap());
    fs::create_dir_all(&staging).unwrap();
    let mut files = Vec::new();
    for (index, change) in manifest["changes"].as_array().unwrap().iter().enumerate() {
        let path = root.join(change["path"].as_str().unwrap());
        let bytes = fs::read(&path).unwrap();
        fs::write(staging.join(format!("{index}.blob")), &bytes).unwrap();
        if index >= applied { fs::remove_file(&path).unwrap(); }
        files.push((path, bytes));
    }
    manifest["state"] = "prepared".into();
    fs::write(&journal, serde_json::to_vec(&manifest).unwrap()).unwrap();
    (staging, files)
}

#[test]
fn doctor_recovers_initialization_before_configuration_exists_at_every_file_boundary() {
    for applied in 0..=5 {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        let (staging, files) = interrupted(&root, applied);
        assert_eq!(error(invoke(&root, &["doctor", "--repair"])), "CONFIRMATION_REQUIRED");
        let diagnosis = success(invoke(&root, &["doctor"]));
        assert_eq!(diagnosis["recovery"]["recovery_required"], true);
        let preview = success(invoke(&root, &["doctor", "--repair", "--dry-run"]));
        assert_eq!(preview["dry_run"], true);
        assert_eq!(preview["recovery"]["transactions"][0]["paths"].as_array().unwrap().len(), files.len());
        assert!(!root.join(".knowmesh/index.sqlite3").exists());
        for (index, (path, _)) in files.iter().enumerate() { assert_eq!(path.exists(), index < applied); }
        let report = success(invoke(&root, &["doctor", "--repair", "--yes"]));
        assert!(report["workspace_id"].is_string());
        assert_eq!(report["recovery"]["recovery_required"], false);
        assert_eq!(report["recovery"]["recovered_transaction_ids"].as_array().unwrap().len(), 1);
        assert_eq!(report["generation"], 1);
        for (path, bytes) in files { assert_eq!(fs::read(path).unwrap(), bytes); }
        assert!(!staging.exists());
        assert_eq!(success(invoke(&root, &["doctor", "--repair", "--yes"]))["generation"], 1);
    }
}

#[test]
fn recovery_preview_and_execution_preserve_external_edits_and_bad_staging() {
    for corrupt_staging in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        let (staging, files) = interrupted(&root, 1);
        let target = if corrupt_staging { staging.join("1.blob") } else { files[0].0.clone() };
        fs::write(&target, "External changes.").unwrap();
        let code = if corrupt_staging { "TRANSACTION_STAGING_CORRUPT" } else { "TRANSACTION_RECOVERY_CONFLICT" };
        let preview = success(invoke(&root, &["doctor", "--repair", "--dry-run"]));
        assert!(preview["issues"].as_array().unwrap().iter().any(|issue| issue["code"] == code));
        assert_eq!(error(invoke(&root, &["doctor", "--repair", "--yes"])), code);
        assert_eq!(fs::read_to_string(target).unwrap(), "External changes.");
        assert!(!root.join("knowmesh.yaml").exists());
        assert!(!root.join(".knowmesh/index.sqlite3").exists());
        assert!(staging.exists());
    }
}

#[test]
fn configuration_recovery_rejects_another_workspaces_database_before_writing() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("interrupted");
    let (staging, _) = interrupted(&root, 1);
    let other = temp.path().join("other");
    success(cargo_bin_cmd!("knowmesh").arg("init").arg(&other).output().unwrap());
    success(invoke(&other, &["sync"]));
    fs::copy(other.join(".knowmesh/index.sqlite3"), root.join(".knowmesh/index.sqlite3")).unwrap();
    assert_eq!(error(invoke(&root, &["doctor", "--repair", "--yes"])), "WORKSPACE_ID_MISMATCH");
    assert!(!root.join("knowmesh.yaml").exists());
    assert!(staging.exists());
}
