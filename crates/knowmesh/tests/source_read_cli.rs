use std::{fs, path::Path, process::Output};

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;

fn output(root: &Path, args: &[&str]) -> Output {
    cargo_bin_cmd!("knowmesh")
        .arg("--workspace")
        .arg(root)
        .args(args)
        .output()
        .unwrap()
}

fn json(root: &Path, args: &[&str]) -> Value {
    let output = output(root, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn source_reads_are_discoverable_and_raw_content_is_byte_exact_with_typed_errors() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    cargo_bin_cmd!("knowmesh")
        .arg("init")
        .arg(&root)
        .assert()
        .success();
    let path = temp.path().join("notes.txt");
    let bytes = "Synthetic notes\r\n".as_bytes();
    fs::write(&path, bytes).unwrap();
    let added = json(&root, &["source", "add", path.to_str().unwrap()]);
    let id = added["data"]["source"]["id"].as_str().unwrap();
    let revision = added["data"]["revision"]["id"].as_str().unwrap();
    let list = json(&root, &["source", "list", "--limit", "1"]);
    assert_eq!(list["data"]["items"][0]["id"], id);
    assert_eq!(list["meta"]["command"], "source.list");
    let got = json(&root, &["source", "get", id]);
    assert_eq!(got["data"]["source"]["revisions"][0]["id"], revision);
    let content = json(&root, &["source", "content", revision]);
    assert_eq!(content["data"]["encoding"], "utf-8");
    assert_eq!(content["data"]["content"], "Synthetic notes\r\n");
    for target in [id, revision] {
        let raw = output(&root, &["source", "content", target, "--raw"]);
        assert!(
            raw.status.success(),
            "{}",
            String::from_utf8_lossy(&raw.stderr)
        );
        assert!(raw.stderr.is_empty());
        assert_eq!(raw.stdout, bytes);
    }
    for args in [
        vec!["source", "content", id, "--raw", "--format", "json"],
        vec!["--format=pretty", "source", "content", id, "--raw"],
        vec!["source", "content", "../invalid", "--raw"],
        vec!["source", "list", "--limit", "0"],
    ] {
        let error = output(&root, &args);
        assert_eq!(error.status.code(), Some(2));
        assert!(error.stdout.is_empty());
        assert_eq!(
            serde_json::from_slice::<Value>(&error.stderr).unwrap()["ok"],
            false
        );
    }
    for op in ["source.list", "source.get", "source.content"] {
        let descriptor = json(&root, &["schema", "command", op]);
        assert_eq!(descriptor["data"]["effect"], "read");
        assert_eq!(descriptor["data"]["supports_dry_run"], false);
    }
    json(&root, &["source", "remove", id, "--yes"]);
    assert_eq!(json(&root, &["source", "list"])["data"]["total"], 0);
    assert_eq!(
        json(&root, &["source", "list", "--include-removed"])["data"]["total"],
        1
    );
    assert_eq!(
        output(&root, &["source", "content", revision, "--raw"]).stdout,
        bytes
    );
}

#[test]
fn binary_sources_have_explicit_json_encoding_and_lossless_raw_output() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    cargo_bin_cmd!("knowmesh")
        .arg("init")
        .arg(&root)
        .assert()
        .success();
    let path = temp.path().join("sample.pdf");
    let bytes = b"%PDF-1.7\n\x00\xff\xfe";
    fs::write(&path, bytes).unwrap();
    let added = json(&root, &["source", "add", path.to_str().unwrap()]);
    let id = added["data"]["source"]["id"].as_str().unwrap();
    let content = json(&root, &["source", "content", id]);
    assert_eq!(content["data"]["encoding"], "base64");
    assert_eq!(content["data"]["content"], "JVBERi0xLjcKAP/+");
    assert_eq!(content["data"]["revision"]["byte_size"], bytes.len());
    let raw = output(&root, &["source", "content", id, "--raw"]);
    assert!(raw.status.success());
    assert!(raw.stderr.is_empty());
    assert_eq!(raw.stdout, bytes);
}

#[test]
fn source_list_cursor_is_available_in_envelope_metadata_and_continues_the_same_query() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    cargo_bin_cmd!("knowmesh")
        .arg("init")
        .arg(&root)
        .assert()
        .success();
    let path = temp.path().join("notes.txt");
    fs::write(&path, "Pagination fixture").unwrap();
    for _ in 0..2 {
        json(&root, &["source", "add", path.to_str().unwrap()]);
    }
    let first = json(&root, &["source", "list", "--limit", "1"]);
    assert!(first["data"]["next_cursor"].is_string());
    assert_eq!(first["meta"]["next_cursor"], first["data"]["next_cursor"]);
    let cursor = first["meta"]["next_cursor"].as_str().unwrap();
    let second = json(
        &root,
        &["source", "list", "--limit", "1", "--cursor", cursor],
    );
    assert_ne!(
        first["data"]["items"][0]["id"],
        second["data"]["items"][0]["id"]
    );
    assert_eq!(second["data"]["total"], 2);
    assert!(second["data"]["next_cursor"].is_null());
    assert!(second["meta"]["next_cursor"].is_null());
}
