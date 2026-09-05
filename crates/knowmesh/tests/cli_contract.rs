use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;

#[test]
fn schema_commands_expose_the_same_application_catalog() {
    let output = cargo_bin_cmd!("knowmesh")
        .args(["schema", "command", "version"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["meta"]["command"], "schema.command");
    assert_eq!(value["data"]["name"], "version");
    assert_eq!(value["data"]["effect"], "read");
    assert_eq!(
        value["data"]["output_schema"]["properties"]["version"]["type"],
        "string"
    );
    let list = cargo_bin_cmd!("knowmesh")
        .args(["schema", "list"])
        .output()
        .unwrap();
    assert!(list.status.success());
    let list: Value = serde_json::from_slice(&list.stdout).unwrap();
    assert!(
        list["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|op| op["name"] == "schema.command")
    );
}

#[test]
fn version_is_one_json_value_without_a_workspace_or_web() {
    let temp = tempfile::tempdir().unwrap();
    let output = cargo_bin_cmd!("knowmesh")
        .current_dir(temp.path())
        .arg("version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["data"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(value["meta"]["schema_version"], "1");
    assert_eq!(value["meta"]["command"], "version");
    assert_eq!(value["data"]["api_contract_version"], "1.0.0");
    assert!(!temp.path().join("knowmesh.yaml").exists());
}

#[test]
fn unknown_commands_use_typed_stderr_and_leave_stdout_empty() {
    let output = cargo_bin_cmd!("knowmesh")
        .arg("not-a-command")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let value: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["type"], "validation");
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
    assert!(value["error"]["hint"].is_string());
}

#[test]
fn unsupported_output_formats_fail_instead_of_silently_changing_the_contract() {
    let output = cargo_bin_cmd!("knowmesh")
        .args(["version", "--format", "csv"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let value: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(value["error"]["code"], "UNSUPPORTED_FORMAT");
}

#[test]
fn init_is_discoverable_dry_runnable_and_repeatable() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let planned = cargo_bin_cmd!("knowmesh")
        .arg("init")
        .arg(&root)
        .args(["--name", "Research", "--dry-run"])
        .output()
        .unwrap();
    assert!(
        planned.status.success(),
        "{}",
        String::from_utf8_lossy(&planned.stderr)
    );
    let planned: Value = serde_json::from_slice(&planned.stdout).unwrap();
    assert_eq!(planned["data"]["dry_run"], true);
    assert!(!root.exists());
    let first = cargo_bin_cmd!("knowmesh")
        .arg("init")
        .arg(&root)
        .args(["--name", "Research"])
        .output()
        .unwrap();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(first["meta"]["workspace_id"], first["data"]["workspace_id"]);
    let again = cargo_bin_cmd!("knowmesh")
        .arg("init")
        .arg(&root)
        .args(["--name", "Research"])
        .output()
        .unwrap();
    assert!(again.status.success());
    let again: Value = serde_json::from_slice(&again.stdout).unwrap();
    assert_eq!(first["data"]["workspace_id"], again["data"]["workspace_id"]);
    assert_eq!(again["data"]["created_paths"], serde_json::json!([]));
    let schema = cargo_bin_cmd!("knowmesh")
        .args(["schema", "command", "init"])
        .output()
        .unwrap();
    assert!(schema.status.success());
    let schema: Value = serde_json::from_slice(&schema.stdout).unwrap();
    assert_eq!(schema["data"]["effect"], "canonical-write");
    assert_eq!(schema["data"]["supports_dry_run"], true);
    assert_eq!(schema["data"]["supports_idempotency"], true);
}

#[test]
fn schema_pack_uses_workspace_resolution_and_reports_missing_packs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    cargo_bin_cmd!("knowmesh")
        .arg("init")
        .arg(&root)
        .assert()
        .success();
    let output = cargo_bin_cmd!("knowmesh")
        .current_dir(root.join("knowledge"))
        .env_remove("KNOWMESH_WORKSPACE")
        .args(["schema", "pack", "research"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["data"]["id"], "research");
    assert_eq!(
        result["data"]["predicates"]["evaluated_on"]["inverse"],
        "evaluates"
    );
    assert!(result["meta"]["workspace_id"].is_string());
    let missing = cargo_bin_cmd!("knowmesh")
        .arg("--workspace")
        .arg(&root)
        .args(["schema", "pack", "unknown"])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(missing.stdout.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&missing.stderr).unwrap()["error"]["code"],
        "SCHEMA_PACK_NOT_FOUND"
    );
}
