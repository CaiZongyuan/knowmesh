#[path = "../../../tests/support/mod.rs"]
mod support;

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;

fn success(output: std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn search_cli_exposes_the_core_contract_and_continues_filtered_pages() {
    let (temp, _) = support::fixture();
    let schema = success(
        cargo_bin_cmd!("knowmesh")
            .args(["schema", "command", "knowledge.search"])
            .output()
            .unwrap(),
    );
    assert_eq!(schema["data"]["effect"], "read");
    assert!(schema["data"]["input_schema"]["properties"]["query"].is_object());
    let first = success(
        cargo_bin_cmd!("knowmesh")
            .arg("--workspace")
            .arg(temp.path())
            .args([
                "search",
                "fixture",
                "--record-type",
                "node",
                "--limit",
                "1",
                "--explain",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(first["meta"]["command"], "knowledge.search");
    assert!(first["meta"]["workspace_id"].is_string());
    assert_eq!(
        first["data"]["groups"]["knowledge"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(first["meta"]["next_cursor"], first["data"]["next_cursor"]);
    assert!(first["data"]["groups"]["knowledge"][0]["explain"]["normalized_score"].is_number());
    let cursor = first["meta"]["next_cursor"].as_str().unwrap();
    let second = success(
        cargo_bin_cmd!("knowmesh")
            .arg("--workspace")
            .arg(temp.path())
            .args([
                "search",
                "fixture",
                "--record-type",
                "node",
                "--limit",
                "1",
                "--cursor",
                cursor,
            ])
            .output()
            .unwrap(),
    );
    assert_ne!(
        first["data"]["groups"]["knowledge"][0]["record_id"],
        second["data"]["groups"]["knowledge"][0]["record_id"]
    );
    assert!(second["meta"].get("next_cursor").is_none());
    assert!(
        second["data"]["groups"]["knowledge"][0]
            .get("explain")
            .is_none()
    );
    let changed = cargo_bin_cmd!("knowmesh")
        .arg("--workspace")
        .arg(temp.path())
        .args(["search", "fixture", "--tag", "changed", "--cursor", cursor])
        .output()
        .unwrap();
    assert_eq!(changed.status.code(), Some(2));
    assert!(changed.stdout.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&changed.stderr).unwrap()["error"]["code"],
        "CURSOR_QUERY_MISMATCH"
    );
}

#[test]
fn search_cli_keeps_advanced_syntax_explicit_and_returns_typed_query_errors() {
    let (temp, _) = support::fixture();
    let result = success(
        cargo_bin_cmd!("knowmesh")
            .arg("--workspace")
            .arg(temp.path())
            .args(["search", "Model OR Dataset", "--query-syntax", "advanced"])
            .output()
            .unwrap(),
    );
    assert!(
        !result["data"]["groups"]["knowledge"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let invalid = cargo_bin_cmd!("knowmesh")
        .arg("--workspace")
        .arg(temp.path())
        .args(["search", "title:(", "--query-syntax", "advanced"])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    assert!(invalid.stdout.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&invalid.stderr).unwrap()["error"]["code"],
        "INVALID_SEARCH_SYNTAX"
    );
}
