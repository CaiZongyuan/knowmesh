#[path = "../../../tests/support/mod.rs"]
mod support;

use std::fs;

use knowmesh_core::{
    application::{
        evidence_verify::{EvidenceInput, EvidenceVerifier},
        proposal::prepare,
    },
    canonical::{snapshot::CanonicalSnapshot, workspace::Workspace},
    domain::{
        ClaimId, Evidence, EvidenceStance, ExtractionMethod, Locator, NodeId, Timestamp,
        proposal::{PatchOp, ProposalInput, ProposalItem, ProposalKind, ReviewInput, ReviewPolicy},
    },
    ingest::TextParser,
    ports::SourceParser,
};
use serde_json::json;

fn fixture() -> (tempfile::TempDir, Workspace, CanonicalSnapshot, Evidence) {
    let (temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let source = &snapshot.sources[0];
    let revision = &source.manifest.revisions[0];
    let bytes = fs::read(
        workspace
            .root
            .join(source.manifest_path.parent().unwrap())
            .join(&revision.path),
    )
    .unwrap();
    let parsed = TextParser::default().parse(revision, &bytes).unwrap();
    let block = &parsed.blocks[1];
    let evidence = EvidenceVerifier::new(revision, &parsed, Default::default())
        .unwrap()
        .verify(&EvidenceInput {
            source_revision_id: revision.id.clone(),
            quote: block.text.clone(),
            locator: Locator {
                page: block.page,
                section_path: block.section_path.clone(),
                paragraph: block.paragraph,
                char_start: Some(block.char_start),
                char_end: Some(block.char_end),
            },
            stance: EvidenceStance::Supports,
            extraction_method: ExtractionMethod::Model,
            confidence: 0.9,
        })
        .unwrap()
        .into_evidence();
    (temp, workspace, snapshot, evidence)
}

fn input(snapshot: &CanonicalSnapshot, items: Vec<ProposalItem>) -> ProposalInput {
    ProposalInput {
        kind: ProposalKind::Manual,
        base_generation: 1,
        schema_hash: snapshot.schema_hash.clone(),
        source_revision_id: None,
        compiler_run_id: None,
        summary: "Proposed knowledge changes.".into(),
        items,
    }
}
fn now() -> Timestamp {
    "2026-09-06T01:00:00Z".parse().unwrap()
}

#[test]
fn valid_patches_verify_real_source_quotes_and_preview_exact_canonical_changes() {
    let (temp, workspace, snapshot, evidence) = fixture();
    let model = snapshot
        .nodes
        .iter()
        .find(|node| node.metadata.node_type == "Model")
        .unwrap();
    let mut claim = snapshot.claims[0].claim.assertion.clone();
    claim.id = ClaimId::new();
    claim.statement = "An evaluation of Model A on Dataset B was recorded.".into();
    claim.evidence = vec![evidence];
    let items = vec![
        ProposalItem::new(
            PatchOp::AddAlias,
            model.metadata.id.to_string(),
            json!({"alias":"New alias"}),
        )
        .unwrap(),
        ProposalItem::new(
            PatchOp::UpdateNodeSummary,
            model.metadata.id.to_string(),
            json!({"summary":"A revised summary."}),
        )
        .unwrap(),
        ProposalItem::new(
            PatchOp::AddClaim,
            model.metadata.id.to_string(),
            json!({"claim":claim}),
        )
        .unwrap(),
    ];
    let original = fs::read(workspace.root.join(&model.canonical_path)).unwrap();
    let prepared = prepare(&workspace, &input(&snapshot, items), "local-user", now()).unwrap();
    assert!(
        prepared
            .proposal
            .items
            .iter()
            .all(|item| !item.issues.iter().any(|issue| issue.blocking))
    );
    assert_eq!(prepared.documents().len(), 1);
    assert!(
        prepared
            .proposal
            .items
            .iter()
            .all(|item| item.before_sha256.as_deref() == Some(&model.content_sha256))
    );
    assert_eq!(prepared.preview.as_ref().unwrap().claims().len(), 2);
    assert_eq!(
        fs::read(workspace.root.join(&model.canonical_path)).unwrap(),
        original
    );
    assert!(!workspace.index_path().unwrap().exists());
    for (path, bytes) in prepared.documents() {
        fs::write(temp.path().join(path), bytes).unwrap();
    }
    assert_eq!(
        prepared.preview.as_ref().unwrap().content_sha256(),
        CanonicalSnapshot::scan(&workspace).unwrap().content_sha256
    );
}

#[test]
fn unverifiable_quotes_are_blocked_and_cannot_be_accepted() {
    let (_temp, workspace, snapshot, mut evidence) = fixture();
    evidence.quote = "This invented sentence is absent from the source.".into();
    evidence.quote_sha256 = knowmesh_core::domain::sha256(evidence.quote.as_bytes());
    let mut claim = snapshot.claims[0].claim.assertion.clone();
    claim.id = ClaimId::new();
    claim.statement = "An invented assertion.".into();
    claim.evidence = vec![evidence];
    let item = ProposalItem::new(
        PatchOp::AddClaim,
        snapshot.claims[0].claim.subject_node_id.to_string(),
        json!({"claim":claim}),
    )
    .unwrap();
    let prepared = prepare(
        &workspace,
        &input(&snapshot, vec![item]),
        "local-user",
        now(),
    )
    .unwrap();
    assert!(
        prepared.proposal.items[0]
            .issues
            .iter()
            .any(|issue| issue.code == "EVIDENCE_QUOTE_NOT_FOUND" && issue.blocking)
    );
    assert!(prepared.documents().is_empty());
    assert_eq!(
        prepared
            .proposal
            .review(
                &ReviewInput {
                    expected_revision: 1,
                    accept_all: true,
                    decisions: vec![]
                },
                &ReviewPolicy::default(),
                "reviewer",
                now()
            )
            .unwrap_err()
            .code,
        "PROPOSAL_ITEM_BLOCKED"
    );
}

#[test]
fn unknown_payload_fields_missing_targets_and_wrong_before_hashes_do_not_write() {
    let (_temp, workspace, snapshot, _) = fixture();
    let target = snapshot.nodes[0].metadata.id.to_string();
    let mut wrong_hash =
        ProposalItem::new(PatchOp::AddAlias, target.clone(), json!({"alias":"B"})).unwrap();
    wrong_hash.before_sha256 = Some("0".repeat(64));
    let items = vec![
        ProposalItem::new(
            PatchOp::AddAlias,
            target,
            json!({"alias":"A", "path":"../../outside"}),
        )
        .unwrap(),
        ProposalItem::new(
            PatchOp::AddAlias,
            NodeId::new().to_string(),
            json!({"alias":"C"}),
        )
        .unwrap(),
        wrong_hash,
    ];
    let prepared = prepare(&workspace, &input(&snapshot, items), "local-user", now()).unwrap();
    assert!(
        prepared
            .proposal
            .items
            .iter()
            .all(|item| item.issues.iter().any(|issue| issue.blocking))
    );
    assert!(prepared.documents().is_empty());
}
