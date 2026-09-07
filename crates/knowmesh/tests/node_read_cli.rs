use std::fs;

use assert_cmd::cargo::cargo_bin_cmd;
use knowmesh_core::{
    canonical::node::NodeDocument,
    domain::{LifecycleStatus, NodeId, NodeKind, NodeMetadata, Timestamp},
};
use serde_json::Value;
use std::collections::BTreeMap;

fn node_metadata(id: NodeId, node_type: &str, name: &str, aliases: &[&str]) -> NodeMetadata {
    let now: Timestamp = "2026-09-05T00:00:00Z".parse().unwrap();
    NodeMetadata {
        version: 1,
        id,
        kind: NodeKind::Node,
        schema: "research@1".into(),
        node_type: node_type.into(),
        name: name.into(),
        aliases: aliases.iter().map(|alias| alias.to_string()).collect(),
        tags: vec!["fixture".into()],
        lifecycle_status: LifecycleStatus::Active,
        created_at: now,
        updated_at: now,
        properties: BTreeMap::new(),
        extra: BTreeMap::new(),
    }
}

fn write_node(root: &std::path::Path, file: &str, metadata: NodeMetadata, summary: &str) {
    let document = NodeDocument::create(
        metadata,
        &format!("# {summary}\n\n## Summary\n\n{summary}.\n"),
    )
    .unwrap();
    fs::write(
        root.join("knowledge/nodes").join(file),
        document.render().unwrap(),
    )
    .unwrap();
}

fn output(root: &std::path::Path, args: &[&str]) -> std::process::Output {
    cargo_bin_cmd!("knowmesh")
        .arg("--workspace")
        .arg(root)
        .args(args)
        .output()
        .unwrap()
}

fn json(root: &std::path::Path, args: &[&str]) -> Value {
    let output = output(root, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}

fn error(root: &std::path::Path, args: &[&str]) -> (Option<i32>, Value) {
    let output = output(root, args);
    assert!(output.stdout.is_empty());
    (
        output.status.code(),
        serde_json::from_slice(&output.stderr).unwrap(),
    )
}

fn fixture_workspace() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    cargo_bin_cmd!("knowmesh")
        .arg("init")
        .arg(&root)
        .assert()
        .success();
    write_node(
        &root,
        "model-a.md",
        node_metadata(NodeId::new(), "Model", "Model A", &["Shared alias"]),
        "A fictional model",
    );
    write_node(
        &root,
        "dataset-b.md",
        node_metadata(NodeId::new(), "Dataset", "Dataset B", &["Shared alias"]),
        "A fictional dataset",
    );
    (temp, root)
}

#[test]
fn node_reads_are_discoverable_filtered_paginated_and_ambiguity_aware_via_cli() {
    let (_temp, root) = fixture_workspace();
    let list = json(&root, &["node", "list"]);
    assert_eq!(list["meta"]["command"], "node.list");
    assert_eq!(list["data"]["total"], 2);
    assert_eq!(list["data"]["items"].as_array().unwrap().len(), 2);
    assert_eq!(list["data"]["index_complete"], true);
    let model_id = list["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["node_type"] == "Model")
        .unwrap()["id"]
        .clone();
    let filtered = json(&root, &["node", "list", "--type", "Model"]);
    assert_eq!(filtered["data"]["total"], 1);
    assert_eq!(filtered["data"]["items"][0]["id"], model_id);
    let tagged = json(&root, &["node", "list", "--tag", "fixture"]);
    assert_eq!(tagged["data"]["total"], 2);
    let paged = json(&root, &["node", "list", "--limit", "1"]);
    assert!(paged["data"]["next_cursor"].is_string());
    assert_eq!(paged["meta"]["next_cursor"], paged["data"]["next_cursor"]);
    let cursor = paged["meta"]["next_cursor"].as_str().unwrap();
    let second = json(&root, &["node", "list", "--limit", "1", "--cursor", cursor]);
    assert_ne!(
        second["data"]["items"][0]["id"],
        paged["data"]["items"][0]["id"]
    );
    assert_eq!(second["data"]["total"], 2);
    let got = json(&root, &["node", "get", model_id.as_str().unwrap()]);
    assert_eq!(got["meta"]["command"], "node.get");
    assert_eq!(got["data"]["node"]["name"], "Model A");
    assert_eq!(got["data"]["node"]["summary"], "A fictional model.");
    assert_eq!(got["data"]["node"]["type"], "Model");
    let by_name = json(&root, &["node", "get", "  model   a "]);
    assert_eq!(by_name["data"]["node"]["id"], model_id);
    let (code, ambiguous) = error(&root, &["node", "get", "Shared alias"]);
    assert_eq!(code, Some(2));
    assert_eq!(ambiguous["error"]["code"], "AMBIGUOUS_NODE_NAME");
    assert_eq!(
        ambiguous["error"]["details"]["candidates"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let (code, missing) = error(&root, &["node", "get", "Absent Node"]);
    assert_eq!(code, Some(3));
    assert_eq!(missing["error"]["code"], "NODE_NOT_FOUND");
    let unknown_id = NodeId::new();
    let (code, unknown) = error(&root, &["node", "get", unknown_id.as_str()]);
    assert_eq!(code, Some(3));
    assert_eq!(unknown["error"]["code"], "NODE_NOT_FOUND");
    for reference in ["kn_short", "src_01ABCDEFGHJKMNPQRSTVWX2345"] {
        let (code, invalid) = error(&root, &["node", "get", reference]);
        assert_eq!(code, Some(2));
        assert_eq!(invalid["error"]["code"], "INVALID_NODE_ID");
    }
    let (code, status) = error(&root, &["node", "list", "--status", "obsolete"]);
    assert_eq!(code, Some(2));
    assert_eq!(status["error"]["code"], "INVALID_NODE_STATUS");
}

#[test]
fn node_list_reports_no_sync_staleness_and_external_edits_accurately() {
    let (_temp, root) = fixture_workspace();
    json(&root, &["node", "list"]);
    let list = json(&root, &["node", "list", "--no-sync"]);
    assert_eq!(list["data"]["index_complete"], false);
    let dataset = fs::read_to_string(root.join("knowledge/nodes/dataset-b.md")).unwrap();
    let mut document = NodeDocument::parse(&dataset).unwrap();
    document.metadata.name = "Dataset B Two".into();
    fs::write(
        root.join("knowledge/nodes/dataset-b.md"),
        document.render().unwrap(),
    )
    .unwrap();
    let stale = json(&root, &["node", "list", "--no-sync"]);
    assert_eq!(stale["data"]["index_complete"], false);
    assert_eq!(stale["data"]["total"], 2);
    let current = json(&root, &["node", "list"]);
    assert_eq!(current["data"]["index_complete"], true);
    let names: Vec<_> = current["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].clone())
        .collect();
    assert!(names.contains(&Value::String("Dataset B Two".into())));
    let old = error(&root, &["node", "get", "Dataset B"]);
    assert_eq!(old.1["error"]["code"], "NODE_NOT_FOUND");
    let renamed = json(&root, &["node", "get", "dataset b two"]);
    assert_eq!(renamed["data"]["node"]["id"], document.metadata.id.as_str());
}

#[test]
fn schema_entity_discovers_types_without_model_or_web_and_reports_schema_errors() {
    let (_temp, root) = fixture_workspace();
    let entity = json(&root, &["schema", "entity", "Model"]);
    assert_eq!(entity["meta"]["command"], "schema.entity");
    assert_eq!(entity["data"]["entity"]["label"], "Model");
    assert_eq!(entity["data"]["entity"]["icon"], "cpu");
    assert!(
        entity["data"]["entity"]["properties"]["developer"]
            .as_object()
            .is_some()
    );
    let predicates = entity["data"]["predicates"].as_array().unwrap();
    let evaluated = predicates
        .iter()
        .find(|p| p["name"] == "evaluated_on")
        .unwrap();
    assert_eq!(evaluated["source"], true);
    assert_eq!(evaluated["target"], false);
    assert_eq!(evaluated["evidence_required"], true);
    let (code, missing) = error(&root, &["schema", "entity", "Creature"]);
    assert_eq!(code, Some(3));
    assert_eq!(missing["error"]["code"], "SCHEMA_ENTITY_NOT_FOUND");
    let (code, empty) = error(&root, &["schema", "entity", ""]);
    assert_eq!(code, Some(2));
    assert_eq!(empty["error"]["code"], "INVALID_ARGUMENT");
}

#[test]
fn node_and_entity_operations_have_complete_read_descriptors() {
    let (_temp, root) = fixture_workspace();
    for operation in ["node.get", "node.list", "schema.entity"] {
        let descriptor = json(&root, &["schema", "command", operation]);
        assert_eq!(descriptor["data"]["effect"], "read");
        assert_eq!(descriptor["data"]["supports_dry_run"], false);
        assert_eq!(descriptor["data"]["supports_idempotency"], false);
        assert_eq!(descriptor["data"]["policy"], "public");
        assert!(descriptor["data"]["input_schema"].is_object());
        assert!(descriptor["data"]["output_schema"].is_object());
    }
    let list = json(&root, &["schema", "list"]);
    let names: Vec<_> = list["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|op| op["name"].as_str())
        .collect();
    for operation in ["node.get", "node.list", "schema.entity"] {
        assert!(names.contains(&operation), "{operation} missing");
    }
}
