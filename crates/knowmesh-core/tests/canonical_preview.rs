#[path = "../../../tests/support/mod.rs"]
mod support;

use std::{collections::BTreeMap, fs, path::PathBuf};

use knowmesh_core::{
    canonical::{node::NodeDocument, snapshot::CanonicalSnapshot},
    domain::{ClaimId, NodeId},
};

#[test]
fn preview_is_read_only_and_matches_the_full_scan_after_applying_its_documents() {
    let (temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let mut metadata = before.nodes[0].metadata.clone();
    metadata.id = NodeId::new();
    metadata.name = "Missing node".into();
    metadata.aliases.clear();
    let created = NodeDocument::create(
        metadata,
        "# Missing node\n\n## Summary\n\nNewly resolved knowledge.",
    )
    .unwrap();
    let path = PathBuf::from("knowledge/nodes/missing-node.md");
    let changes = BTreeMap::from([(path.clone(), created.render().unwrap().into_bytes())]);
    let preview = before.preview_documents(&workspace, &changes).unwrap();
    preview.validate().unwrap();
    assert_eq!(preview.nodes().len(), before.nodes.len() + 1);
    assert_eq!(preview.mentions().len(), before.mentions.len() + 1);
    assert!(!temp.path().join(&path).exists());
    assert!(!workspace.index_path().unwrap().exists());
    assert_eq!(before.nodes.len(), 2);
    let mut forged = before.clone();
    forged.nodes = preview.nodes().to_vec();
    forged.claims = preview.claims().to_vec();
    forged.relations = preview.relations().to_vec();
    forged.evidence = preview.evidence().to_vec();
    forged.sources = preview.sources().to_vec();
    forged.syntheses = preview.syntheses().to_vec();
    forged.mentions = preview.mentions().to_vec();
    forged.files = preview.files().to_vec();
    forged.content_sha256 = preview.content_sha256().into();
    assert_eq!(
        forged.validate().unwrap_err().code,
        "SNAPSHOT_DIGEST_MISMATCH"
    );
    fs::write(temp.path().join(path), changes.values().next().unwrap()).unwrap();
    let actual = CanonicalSnapshot::scan(&workspace).unwrap();
    assert_eq!(preview.content_sha256(), actual.content_sha256);
}

#[test]
fn preview_reuses_schema_and_cross_reference_validation() {
    let (temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let path = PathBuf::from("knowledge/nodes/model-a.md");
    let original = fs::read_to_string(temp.path().join(&path)).unwrap();
    let mut doc = NodeDocument::parse(&original).unwrap();
    let mut added = doc.claims[0].clone();
    added.id = ClaimId::new();
    added.statement = "The fixture recorded a model evaluation.".into();
    doc.claims.push(added);
    let changes = BTreeMap::from([(path.clone(), doc.render().unwrap().into_bytes())]);
    assert_eq!(
        before
            .preview_documents(&workspace, &changes)
            .unwrap()
            .claims()
            .len(),
        2
    );
    assert_eq!(
        fs::read_to_string(temp.path().join(&path)).unwrap(),
        original
    );
    doc.relations[0].target_node_id = NodeId::new();
    let changes = BTreeMap::from([(path, doc.render().unwrap().into_bytes())]);
    assert_eq!(
        before
            .preview_documents(&workspace, &changes)
            .unwrap_err()
            .code,
        "NODE_NOT_FOUND"
    );
}

#[test]
fn preview_rejects_configuration_blob_and_identity_replacement() {
    let (_temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    for path in [
        PathBuf::from("knowmesh.yaml"),
        before
            .files
            .iter()
            .find(|file| file.kind == "source_blob")
            .unwrap()
            .path
            .clone(),
    ] {
        let changes = BTreeMap::from([(path, b"replacement".to_vec())]);
        assert_eq!(
            before
                .preview_documents(&workspace, &changes)
                .unwrap_err()
                .code,
            "CANONICAL_PREVIEW_PATH_FORBIDDEN"
        );
    }
    let mut metadata = before
        .nodes
        .iter()
        .find(|node| node.canonical_path == std::path::Path::new("knowledge/nodes/model-a.md"))
        .unwrap()
        .metadata
        .clone();
    metadata.id = NodeId::new();
    let doc = NodeDocument::create(metadata, "Replacement identity.").unwrap();
    let changes = BTreeMap::from([(
        PathBuf::from("knowledge/nodes/model-a.md"),
        doc.render().unwrap().into_bytes(),
    )]);
    assert_eq!(
        before
            .preview_documents(&workspace, &changes)
            .unwrap_err()
            .code,
        "NODE_IDENTITY_CHANGED"
    );
}

#[test]
fn preview_rejects_a_base_snapshot_whose_files_changed() {
    let (temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let path = temp.path().join("knowledge/nodes/model-a.md");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(
        path,
        text.replace("A fictional model.", "An external edit."),
    )
    .unwrap();
    assert_eq!(
        before
            .preview_documents(&workspace, &BTreeMap::new())
            .unwrap_err()
            .code,
        "CANONICAL_FILE_CONFLICT"
    );
}

#[test]
fn source_metadata_and_synthesis_previews_match_scanning_without_rewriting_revisions() {
    use knowmesh_core::canonical::{source::SourceFile, synthesis::SynthesisDocument};

    let (temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let source_path = before.sources[0].manifest_path.clone();
    let mut source = SourceFile::parse(
        source_path.clone(),
        &fs::read(temp.path().join(&source_path)).unwrap(),
    )
    .unwrap();
    source.manifest.title = "A clarified source title".into();
    let synthesis_path = before.syntheses[0].canonical_path.clone();
    let mut synthesis =
        SynthesisDocument::parse(&fs::read_to_string(temp.path().join(&synthesis_path)).unwrap())
            .unwrap();
    synthesis.metadata.title = "Revised comparison".into();
    let changes = BTreeMap::from([
        (source_path.clone(), source.render().unwrap().into_bytes()),
        (synthesis_path, synthesis.render().unwrap().into_bytes()),
    ]);
    let preview = before.preview_documents(&workspace, &changes).unwrap();
    assert_eq!(
        preview.sources()[0].manifest.title,
        "A clarified source title"
    );
    assert_eq!(preview.syntheses()[0].metadata.title, "Revised comparison");
    let mut altered = source.manifest.clone();
    altered.revisions[0].sha256 = knowmesh_core::domain::sha256(b"changed source");
    assert!(
        before
            .preview_documents(
                &workspace,
                &BTreeMap::from([(
                    source_path,
                    serde_yaml::to_string(&altered).unwrap().into_bytes()
                )])
            )
            .is_err()
    );
    for (path, bytes) in changes {
        fs::write(temp.path().join(path), bytes).unwrap();
    }
    assert_eq!(
        preview.content_sha256(),
        CanonicalSnapshot::scan(&workspace).unwrap().content_sha256
    );
}

#[test]
fn newly_added_canonical_files_invalidate_a_previous_base_snapshot() {
    let (temp, workspace) = support::fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let mut metadata = before.nodes[0].metadata.clone();
    metadata.id = NodeId::new();
    let added = NodeDocument::create(metadata, "Another canonical file.").unwrap();
    fs::write(
        temp.path().join("knowledge/nodes/added.md"),
        added.render().unwrap(),
    )
    .unwrap();
    assert_eq!(
        before
            .preview_documents(&workspace, &BTreeMap::new())
            .unwrap_err()
            .code,
        "CANONICAL_FILE_CONFLICT"
    );
}
