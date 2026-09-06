use std::{fs, path::Path};

use assert_cmd::cargo::cargo_bin_cmd;
use knowmesh_core::{domain::sha256, ingest::TextParser, ports::SourceParser};
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

fn add(root: &Path, path: &Path, extra: &[&str]) -> std::process::Output {
    cargo_bin_cmd!("knowmesh")
        .arg("--workspace")
        .arg(root)
        .args(["source", "add"])
        .arg(path)
        .args(extra)
        .output()
        .unwrap()
}

#[test]
fn explicit_encoding_preserves_raw_snapshots_and_historical_read_and_parse_contracts() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    cargo_bin_cmd!("knowmesh")
        .arg("init")
        .arg(&root)
        .assert()
        .success();
    let path = temp.path().join("legacy.txt");
    let raw = b"caf\xe9\n";
    fs::write(&path, raw).unwrap();
    let rejected = add(&root, &path, &[]);
    assert!(!rejected.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&rejected.stderr).unwrap()["error"]["code"],
        "UNSUPPORTED_ENCODING"
    );
    let preview = success(add(
        &root,
        &path,
        &["--encoding", "windows-1252", "--dry-run"],
    ));
    assert_eq!(preview["data"]["revision"]["encoding"], "windows-1252");
    assert_eq!(fs::read_dir(root.join("sources")).unwrap().count(), 0);
    let first = success(add(&root, &path, &["--encoding", "latin1"]));
    let source_id = first["data"]["source"]["id"].as_str().unwrap();
    let revision_id = first["data"]["revision"]["id"].as_str().unwrap();
    assert_eq!(first["data"]["revision"]["sha256"], sha256(raw));
    assert_eq!(first["data"]["revision"]["byte_size"], raw.len());
    assert_eq!(first["data"]["revision"]["encoding"], "windows-1252");
    let duplicate = success(add(&root, &path, &["--source-id", source_id]));
    assert_eq!(duplicate["data"]["deduplicated"], true);
    assert_eq!(duplicate["data"]["revision"]["id"], revision_id);
    let conflict = add(
        &root,
        &path,
        &["--source-id", source_id, "--encoding", "windows-1251"],
    );
    assert_eq!(conflict.status.code(), Some(7));
    assert_eq!(
        serde_json::from_slice::<Value>(&conflict.stderr).unwrap()["error"]["code"],
        "SOURCE_ENCODING_MISMATCH"
    );
    fs::write(&path, "café updated\n").unwrap();
    let appended = success(add(&root, &path, &["--source-id", source_id]));
    assert_ne!(appended["data"]["revision"]["id"], revision_id);
    assert!(appended["data"]["revision"].get("encoding").is_none());
    let content = success(
        cargo_bin_cmd!("knowmesh")
            .arg("--workspace")
            .arg(&root)
            .args(["source", "content", revision_id])
            .output()
            .unwrap(),
    );
    assert_eq!(content["data"]["content"], "café\n");
    assert_eq!(content["data"]["encoding"], "utf-8");
    let bytes = cargo_bin_cmd!("knowmesh")
        .arg("--workspace")
        .arg(&root)
        .args(["source", "content", revision_id, "--raw"])
        .output()
        .unwrap();
    assert!(bytes.status.success());
    assert_eq!(bytes.stdout, raw);
    let revision = serde_json::from_value(content["data"]["revision"].clone()).unwrap();
    let parsed = TextParser::default().parse(&revision, raw).unwrap();
    assert_eq!(parsed.normalized_text, "café");
    assert!(
        parsed
            .blocks
            .iter()
            .all(|block| block.source_bytes.is_none())
    );
}

#[test]
fn referenced_legacy_bytes_are_verified_and_invalid_labels_or_pdf_encodings_fail() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    cargo_bin_cmd!("knowmesh")
        .arg("init")
        .arg(&root)
        .assert()
        .success();
    let path = temp.path().join("wide.txt");
    let raw = [0xff, 0xfe, b'A', 0, b'B', 0];
    fs::write(&path, raw).unwrap();
    let added = success(add(
        &root,
        &path,
        &["--storage", "referenced", "--encoding", "utf-16le"],
    ));
    let id = added["data"]["source"]["id"].as_str().unwrap();
    let content = success(
        cargo_bin_cmd!("knowmesh")
            .arg("--workspace")
            .arg(&root)
            .args(["source", "content", id])
            .output()
            .unwrap(),
    );
    assert_eq!(content["data"]["content"], "\u{feff}AB");
    let revision = serde_json::from_value(content["data"]["revision"].clone()).unwrap();
    assert_eq!(
        TextParser::default()
            .parse(&revision, &raw)
            .unwrap()
            .normalized_text,
        "AB"
    );
    fs::write(&path, b"modified").unwrap();
    let changed = cargo_bin_cmd!("knowmesh")
        .arg("--workspace")
        .arg(&root)
        .args(["source", "content", id, "--no-sync"])
        .output()
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&changed.stderr).unwrap()["error"]["code"],
        "SOURCE_REVISION_CHANGED"
    );
    assert!(
        !add(&root, &path, &["--encoding", "not-an-encoding"])
            .status
            .success()
    );
    let pdf = temp.path().join("paper.pdf");
    fs::write(&pdf, b"%PDF-1.7\nfixture").unwrap();
    let error = add(&root, &pdf, &["--encoding", "utf-8"]);
    assert!(!error.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&error.stderr).unwrap()["error"]["code"],
        "ENCODING_NOT_APPLICABLE"
    );
}

#[test]
fn synchronization_rejects_rewriting_historical_encoding_and_no_sync_uses_indexed_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    cargo_bin_cmd!("knowmesh")
        .arg("init")
        .arg(&root)
        .assert()
        .success();
    let path = temp.path().join("legacy.txt");
    fs::write(&path, b"caf\xe9").unwrap();
    let added = success(add(&root, &path, &["--encoding", "windows-1252"]));
    let id = added["data"]["source"]["id"].as_str().unwrap();
    let workspace = knowmesh_core::canonical::workspace::Workspace::load(&root).unwrap();
    let mut source = knowmesh_core::canonical::source::SourceLibrary::new(&workspace)
        .get(&id.parse().unwrap())
        .unwrap();
    source.manifest.revisions[0].encoding = Some("windows-1251".parse().unwrap());
    fs::write(
        root.join(source.path),
        serde_yaml::to_string(&source.manifest).unwrap(),
    )
    .unwrap();
    let sync = cargo_bin_cmd!("knowmesh")
        .arg("--workspace")
        .arg(&root)
        .arg("sync")
        .output()
        .unwrap();
    assert!(!sync.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&sync.stderr).unwrap()["error"]["code"],
        "IMMUTABLE_REVISION_CHANGED"
    );
    let content = success(
        cargo_bin_cmd!("knowmesh")
            .arg("--workspace")
            .arg(&root)
            .args(["source", "content", id, "--no-sync"])
            .output()
            .unwrap(),
    );
    assert_eq!(content["data"]["content"], "café");
    assert_eq!(content["data"]["revision"]["encoding"], "windows-1252");
}
