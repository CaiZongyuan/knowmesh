use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;

#[test]
fn rebuild_cli_exposes_confirmation_preview_and_backup_retention() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    cargo_bin_cmd!("knowmesh")
        .arg("init")
        .arg(&root)
        .assert()
        .success();
    let invoke = |args: &[&str]| {
        cargo_bin_cmd!("knowmesh")
            .arg("--workspace")
            .arg(&root)
            .args(args)
            .output()
            .unwrap()
    };
    let unconfirmed = invoke(&["rebuild"]);
    assert!(!unconfirmed.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&unconfirmed.stderr).unwrap()["error"]["code"],
        "CONFIRMATION_REQUIRED"
    );
    let preview = invoke(&["rebuild", "--dry-run"]);
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&preview.stdout).unwrap()["data"]["dry_run"],
        true
    );
    assert!(!root.join(".knowmesh/index.sqlite3").exists());
    for _ in 0..3 {
        let result = invoke(&["rebuild", "--yes", "--keep-backups", "1"]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(report["meta"]["command"], "rebuild");
        assert_eq!(report["data"]["projection"]["generation"], 1);
    }
    assert_eq!(
        std::fs::read_dir(root.join(".knowmesh/backups"))
            .unwrap()
            .count(),
        1
    );
    let schema = invoke(&["schema", "command", "rebuild"]);
    assert!(schema.status.success());
    let descriptor: Value = serde_json::from_slice(&schema.stdout).unwrap();
    assert_eq!(descriptor["data"]["effect"], "destructive-derived");
    assert_eq!(descriptor["data"]["supports_dry_run"], true);
    let invalid = invoke(&["rebuild", "--yes", "--keep-backups", "0"]);
    assert_eq!(
        serde_json::from_slice::<Value>(&invalid.stderr).unwrap()["error"]["code"],
        "INVALID_BACKUP_RETENTION"
    );
}
