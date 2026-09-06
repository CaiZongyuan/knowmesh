use std::{collections::BTreeMap, fs};

use knowmesh_core::{
    canonical::{
        node::NodeDocument,
        synthesis::SynthesisDocument,
        workspace::{InitOptions, Workspace, initialize},
    },
    domain::{
        AssertionDependency, ClaimId, ClaimRecord, DependencySnapshot, Evidence, EvidenceId,
        EvidenceStance, EvidenceStatus, ExtractionMethod, LifecycleStatus, Locator, NodeId,
        NodeKind, NodeMetadata, RelationId, RelationRecord, SourceHead, SourceId, SourceManifest,
        SourceRevision, SourceRevisionId, StorageMode, SynthesisId, SynthesisKind,
        SynthesisMetadata, SynthesisStatus, Timestamp, sha256,
    },
};

pub fn fixture() -> (tempfile::TempDir, Workspace) {
    let temp = tempfile::tempdir().unwrap();
    initialize(temp.path(), &InitOptions::default()).unwrap();
    let workspace = Workspace::load(temp.path()).unwrap();
    let source_id = SourceId::new();
    let revision_id = SourceRevisionId::new();
    let model_id = NodeId::new();
    let dataset_id = NodeId::new();
    let now: Timestamp = "2026-09-05T00:00:00Z".parse().unwrap();
    let quote = "Model A was evaluated on Dataset B.";
    let original = format!("# Fixture\n\n{quote}\n");
    let source_dir = temp.path().join("sources/fixture");
    let revision_path = format!("revisions/{revision_id}/original.md");
    fs::create_dir_all(source_dir.join("revisions").join(revision_id.as_str())).unwrap();
    fs::write(source_dir.join(&revision_path), &original).unwrap();
    let source = SourceManifest {
        version: 1,
        id: source_id.clone(),
        slug: "fixture".into(),
        kind: "paper".into(),
        title: "Fictional evaluation fixture".into(),
        authors: vec![],
        identifiers: BTreeMap::new(),
        language: Some("en".into()),
        tags: vec!["fixture".into()],
        storage: StorageMode::Managed,
        current_revision_id: revision_id.clone(),
        represented_nodes: vec![],
        created_at: now,
        updated_at: now,
        removed_at: None,
        revisions: vec![SourceRevision {
            id: revision_id.clone(),
            path: revision_path,
            mime_type: "text/markdown".into(),
            encoding: None,
            sha256: sha256(original.as_bytes()),
            byte_size: original.len() as u64,
            captured_at: now,
            url: None,
        }],
    };
    fs::write(
        source_dir.join("source.yaml"),
        serde_yaml::to_string(&source).unwrap(),
    )
    .unwrap();
    let metadata = |id, node_type: &str, name: &str| NodeMetadata {
        version: 1,
        id,
        kind: NodeKind::Node,
        schema: "research@1".into(),
        node_type: node_type.into(),
        name: name.into(),
        aliases: vec!["Shared alias".into()],
        tags: vec!["fixture".into()],
        lifecycle_status: LifecycleStatus::Active,
        created_at: now,
        updated_at: now,
        properties: BTreeMap::new(),
        extra: BTreeMap::new(),
    };
    let evidence = Evidence {
        id: EvidenceId::new(),
        source_revision_id: revision_id.clone(),
        stance: EvidenceStance::Supports,
        quote: quote.into(),
        quote_sha256: sha256(quote.as_bytes()),
        locator: Locator {
            section_path: vec!["Fixture".into()],
            paragraph: Some(1),
            ..Locator::default()
        },
        extraction_method: ExtractionMethod::Parser,
        confidence: 1.0,
    };
    let claim = ClaimRecord {
        id: ClaimId::new(),
        statement: quote.into(),
        lifecycle_status: LifecycleStatus::Active,
        evidence_status: EvidenceStatus::Supported,
        confidence: Some(1.0),
        qualifiers: BTreeMap::new(),
        evidence: vec![evidence.clone()],
    };
    let relation = RelationRecord {
        id: RelationId::new(),
        predicate: "evaluated_on".into(),
        target_node_id: dataset_id.clone(),
        directed: true,
        lifecycle_status: LifecycleStatus::Active,
        evidence_status: EvidenceStatus::Supported,
        confidence: Some(1.0),
        qualifiers: BTreeMap::new(),
        evidence: vec![evidence.clone()],
    };
    let mut model = NodeDocument::create(metadata(model_id.clone(), "Model", "Model A"), &format!("# Model A\n\n## Summary\n\nA fictional model.\n\n## Notes\n\nSee [[{dataset_id}|Dataset B]], [[Shared alias]], and [[Missing node]].")).unwrap();
    model.claims.push(claim.clone());
    model.relations.push(relation);
    fs::write(
        temp.path().join("knowledge/nodes/model-a.md"),
        model.render().unwrap(),
    )
    .unwrap();
    let dataset = NodeDocument::create(
        metadata(dataset_id.clone(), "Dataset", "Dataset B"),
        "# Dataset B\n\n## Summary\n\nA fictional dataset.",
    )
    .unwrap();
    fs::write(
        temp.path().join("knowledge/nodes/dataset-b.md"),
        dataset.render().unwrap(),
    )
    .unwrap();
    let synthesis = SynthesisDocument::create(
        SynthesisMetadata {
            version: 1,
            id: SynthesisId::new(),
            kind: SynthesisKind::Synthesis,
            schema: "research@1".into(),
            title: "Fixture comparison".into(),
            question: "Where was the model evaluated?".into(),
            status: SynthesisStatus::Reviewed,
            created_at: now,
            updated_at: now,
            generated_by: None,
            related_nodes: vec![model_id.clone(), dataset_id],
            evidence_ids: vec![evidence.id.clone()],
            dependency_snapshot: Some(DependencySnapshot {
                version: 1,
                assertions: vec![AssertionDependency::Claim {
                    id: claim.id.clone(),
                    semantic_sha256: claim.semantic_hash(&model_id).unwrap(),
                }],
                source_heads: vec![SourceHead {
                    source_id,
                    revision_id,
                }],
            }),
            extra: BTreeMap::new(),
        },
        &format!("# Fixture comparison\n\n{quote} [@{}]", evidence.id),
    )
    .unwrap();
    fs::write(
        temp.path().join("knowledge/syntheses/comparison.md"),
        synthesis.render().unwrap(),
    )
    .unwrap();
    (temp, workspace)
}
