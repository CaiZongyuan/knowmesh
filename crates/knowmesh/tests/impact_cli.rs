use std::fs;

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;

#[test]
fn source_impact_cli_uses_the_registered_read_contract() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    cargo_bin_cmd!("knowmesh").arg("init").arg(&root).assert().success();
    let path = temp.path().join("source.md");
    fs::write(&path, "# A synthetic source\n").unwrap();
    let added = cargo_bin_cmd!("knowmesh").arg("--workspace").arg(&root).args(["source", "add"]).arg(path).output().unwrap();
    assert!(added.status.success());
    let added: Value = serde_json::from_slice(&added.stdout).unwrap();
    let id = added["data"]["import"]["source_id"].as_str().unwrap();
    let result = cargo_bin_cmd!("knowmesh").arg("--workspace").arg(&root).args(["source", "impact", id, "--limit", "1"]).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["meta"]["command"], "source.impact");
    assert_eq!(report["data"]["counts"]["evidence"], 0);
    assert_eq!(report["data"]["items"].as_array().unwrap().len(), 0);
    let schema = cargo_bin_cmd!("knowmesh").args(["schema", "command", "source.impact"]).output().unwrap();
    assert!(schema.status.success());
    assert_eq!(serde_json::from_slice::<Value>(&schema.stdout).unwrap()["data"]["effect"], "read");
}
