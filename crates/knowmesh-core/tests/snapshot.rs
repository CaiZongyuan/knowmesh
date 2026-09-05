#[path = "../../../tests/support/mod.rs"]
mod support;

use std::fs;

use knowmesh_core::canonical::snapshot::CanonicalSnapshot;

#[test]
fn snapshot_rejects_a_stale_workspace_configuration_or_purpose() {
    let (temp, workspace) = support::fixture();
    let path = temp.path().join("knowmesh.yaml");
    let original = fs::read_to_string(&path).unwrap();
    fs::write(&path, original.replace("Knowledge Space", "Renamed Space")).unwrap();
    assert_eq!(
        CanonicalSnapshot::scan(&workspace).unwrap_err().code,
        "CANONICAL_FILE_CONFLICT"
    );
    fs::write(path, original).unwrap();
    let path = temp
        .path()
        .join(workspace.config.workspace.purpose.as_ref().unwrap());
    let original = fs::read_to_string(&path).unwrap();
    fs::write(
        path,
        original.replace("Virtual cell models", "Other models"),
    )
    .unwrap();
    assert_eq!(
        CanonicalSnapshot::scan(&workspace).unwrap_err().code,
        "CANONICAL_FILE_CONFLICT"
    );
}

#[test]
fn modified_projection_payload_cannot_reuse_the_original_snapshot_digest() {
    let (_temp, workspace) = support::fixture();
    let mut snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    snapshot.nodes[0].summary = "Mutated after parsing".into();
    assert_eq!(
        snapshot.validate().unwrap_err().code,
        "SNAPSHOT_DIGEST_MISMATCH"
    );
}

#[test]
fn complete_snapshot_preserves_identity_evidence_and_dependency_graph() {
    let (_temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    assert_eq!(snapshot.sources.len(), 1);
    assert_eq!(snapshot.nodes.len(), 2);
    assert_eq!(snapshot.claims.len(), 1);
    assert_eq!(snapshot.relations.len(), 1);
    assert_eq!(snapshot.evidence.len(), 1);
    assert_eq!(snapshot.syntheses.len(), 1);
    assert_eq!(snapshot.mentions.len(), 1);
    assert!(
        snapshot
            .warnings
            .iter()
            .any(|warning| warning.code == "AMBIGUOUS_NODE_LINK")
    );
    assert!(
        snapshot
            .warnings
            .iter()
            .any(|warning| warning.code == "UNRESOLVED_NODE_LINK")
    );
    assert_eq!(
        snapshot.content_sha256,
        CanonicalSnapshot::scan(&workspace).unwrap().content_sha256
    );
}

#[test]
fn missing_sources_and_invalid_relation_types_stop_projection() {
    let (temp, workspace) = support::fixture();
    let path = temp.path().join("knowledge/nodes/dataset-b.md");
    let original = fs::read_to_string(&path).unwrap();
    fs::write(&path, original.replace("type: Dataset", "type: Gene")).unwrap();
    assert_eq!(
        CanonicalSnapshot::scan(&workspace).unwrap_err().code,
        "RELATION_TYPE_MISMATCH"
    );
    fs::write(path, original).unwrap();
    fs::remove_file(temp.path().join("sources/fixture/source.yaml")).unwrap();
    assert_eq!(
        CanonicalSnapshot::scan(&workspace).unwrap_err().code,
        "SOURCE_REVISION_NOT_FOUND"
    );
}

#[test]
fn duplicate_node_identity_and_changed_managed_blobs_are_rejected() {
    let (temp, workspace) = support::fixture();
    let copy = temp.path().join("knowledge/nodes/duplicate.md");
    fs::copy(temp.path().join("knowledge/nodes/model-a.md"), &copy).unwrap();
    assert_eq!(
        CanonicalSnapshot::scan(&workspace).unwrap_err().code,
        "DUPLICATE_NODE_ID"
    );
    fs::remove_file(copy).unwrap();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let source = &snapshot.sources[0];
    let path = temp
        .path()
        .join(source.manifest_path.parent().unwrap())
        .join(&source.manifest.revisions[0].path);
    fs::write(path, "Altered historical evidence.").unwrap();
    assert_eq!(
        CanonicalSnapshot::scan(&workspace).unwrap_err().code,
        "SOURCE_REVISION_CHANGED"
    );
}
