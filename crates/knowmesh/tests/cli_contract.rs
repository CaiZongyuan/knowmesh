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
