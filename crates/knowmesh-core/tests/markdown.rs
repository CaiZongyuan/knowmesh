use std::collections::BTreeSet;

use knowmesh_core::{
    canonical::{node::NodeDocument, synthesis::SynthesisDocument},
    domain::{ClaimId, EvidenceId, NodeId, RunId, SourceId, SourceRevisionId, SynthesisId, sha256},
};
use proptest::prelude::*;

fn node(notes: &str) -> String {
    format!(
        "---\n# Human metadata comment\nversion: 1\nid: {}\nkind: node\nschema: research@1\ntype: Model\nname: 'Original'\naliases: [Original model]\ntags: []\nlifecycle_status: active\ncreated_at: 2026-09-05T00:00:00Z\nupdated_at: 2026-09-05T00:00:00Z\nx-owner: 'Human' # Preserve this exact line\n---\n\n# Original\n\n## Summary\n\nSummary text.\n\n## My Notes\n\n{notes}\n\n<!-- knowmesh:claims:begin -->\n```yaml\nversion: 1\nitems:\n  - id: {}\n    statement: Original statement.\n    lifecycle_status: active\n    evidence_status: unreviewed\n    confidence: 0.8\n    qualifiers: {{}}\n    evidence: []\n```\n<!-- knowmesh:claims:end -->\n\n<!-- knowmesh:relations:begin -->\n```yaml\nversion: 1\nitems: []\n```\n<!-- knowmesh:relations:end -->\n\n## Human Appendix\n\nKeep trailing spaces.  \n",
        NodeId::new(),
        ClaimId::new()
    )
}

#[test]
fn unchanged_markdown_round_trips_with_crlf_and_unknown_metadata() {
    for original in [
        node("Human content."),
        node("Human content.").replace('\n', "\r\n"),
    ] {
        let document = NodeDocument::parse(&original).unwrap();
        assert_eq!(document.render().unwrap(), original);
        assert_eq!(document.metadata.extra["x-owner"], "Human");
    }
}

#[test]
fn changing_one_claim_preserves_every_byte_outside_its_managed_content() {
    let original = node("Human notes.");
    let mut document = NodeDocument::parse(&original).unwrap();
    document.claims[0].statement = "Updated statement.\n```\nStill part of the statement.".into();
    let updated = document.render().unwrap();
    let prefix = original
        .split("<!-- knowmesh:claims:begin -->")
        .next()
        .unwrap();
    let suffix = original
        .split("<!-- knowmesh:claims:end -->")
        .nth(1)
        .unwrap();
    assert!(updated.starts_with(prefix));
    assert!(updated.ends_with(suffix));
    let parsed = NodeDocument::parse(&updated).unwrap();
    assert_eq!(parsed.claims[0].statement, document.claims[0].statement);
    assert_eq!(parsed.render().unwrap(), updated);
}

#[test]
fn editing_known_frontmatter_preserves_unknown_values_and_comments() {
    let original = node("Human notes.");
    let mut document = NodeDocument::parse(&original).unwrap();
    document.metadata.name = "Renamed: model".into();
    document.metadata.aliases.push("New alias".into());
    let updated = document.render().unwrap();
    assert!(updated.contains("# Human metadata comment\n"));
    assert!(updated.contains("x-owner: 'Human' # Preserve this exact line\n"));
    assert!(updated.ends_with(original.split("---\n\n").nth(1).unwrap()));
    assert_eq!(
        NodeDocument::parse(&updated).unwrap().metadata.name,
        "Renamed: model"
    );
}

#[test]
fn markers_in_code_examples_are_ignored_and_real_marker_errors_are_typed() {
    let original =
        node("```html\n<!-- knowmesh:claims:begin -->\n<!-- knowmesh:claims:end -->\n```\n");
    assert_eq!(
        NodeDocument::parse(&original).unwrap().render().unwrap(),
        original
    );
    let original = node("Normal text.");
    for invalid in [
        original.replace("<!-- knowmesh:claims:end -->", ""),
        original.replace(
            "<!-- knowmesh:relations:begin -->",
            "<!-- knowmesh:claims:begin -->",
        ),
    ] {
        assert_eq!(
            NodeDocument::parse(&invalid).unwrap_err().code,
            "INVALID_MANAGED_BLOCK"
        );
    }
    assert_eq!(
        NodeDocument::parse(&original.replacen("version: 1", "version: 99", 1))
            .unwrap_err()
            .code,
        "UNSUPPORTED_FORMAT_VERSION"
    );
}

#[test]
fn managed_blocks_sort_changed_items_by_identity_and_reject_duplicates() {
    let original = node("Notes.");
    let mut document = NodeDocument::parse(&original).unwrap();
    let mut other = document.claims[0].clone();
    other.id = "clm_00000000000000000000000000".parse().unwrap();
    document.claims.push(other.clone());
    let parsed = NodeDocument::parse(&document.render().unwrap()).unwrap();
    assert_eq!(parsed.claims[0].id, other.id);
    document.claims.push(other);
    assert_eq!(
        document.render().unwrap_err().code,
        "DUPLICATE_ASSERTION_ID"
    );
}

#[test]
fn wiki_links_use_markdown_structure_and_ignore_code() {
    let target = NodeId::new();
    let original = node(&format!(
        "See [[{target}|Named model]] and [[Alias]]. `[[Not a link]]`\n\n```text\n[[Also not a link]]\n```"
    ));
    let document = NodeDocument::parse(&original).unwrap();
    let links = document.links();
    assert_eq!(links.len(), 2);
    assert_eq!(links[0].target, target.to_string());
    assert_eq!(links[0].display, "Named model");
    assert_eq!(links[1].target, "Alias");
}

fn synthesis(evidence: &EvidenceId, snapshot: bool) -> String {
    let snapshot = if snapshot {
        format!(
            "dependency_snapshot:\n  version: 1\n  assertions:\n    - kind: claim\n      id: {}\n      semantic_sha256: {}\n  source_heads:\n    - source_id: {}\n      revision_id: {}\n",
            ClaimId::new(),
            sha256(b"assertion"),
            SourceId::new(),
            SourceRevisionId::new()
        )
    } else {
        String::new()
    };
    format!(
        "---\nversion: 1\nid: {}\nkind: synthesis\nschema: research@1\ntitle: Comparison\nquestion: What differs?\nstatus: reviewed\ncreated_at: 2026-09-05T00:00:00Z\nupdated_at: 2026-09-05T00:00:00Z\ngenerated_by:\n  run_id: {}\n  model: fixture/model\nrelated_nodes: []\nevidence_ids: [{evidence}]\n{snapshot}---\n\n# Comparison\n\nSupported statement. [@{evidence}]\n\n`[@not-an-evidence]`\n",
        SynthesisId::new(),
        RunId::new()
    )
}

#[test]
fn synthesis_citations_are_validated_without_inventing_dependency_snapshots() {
    let evidence = EvidenceId::new();
    for has_snapshot in [false, true] {
        let original = synthesis(&evidence, has_snapshot);
        let document = SynthesisDocument::parse(&original).unwrap();
        assert_eq!(document.render().unwrap(), original);
        assert_eq!(
            document.metadata.dependency_snapshot.is_some(),
            has_snapshot
        );
        assert_eq!(
            document.citations().unwrap(),
            BTreeSet::from([evidence.clone()])
        );
        assert_eq!(
            document
                .validate_citations(&BTreeSet::new())
                .unwrap_err()
                .code,
            "EVIDENCE_NOT_FOUND"
        );
        document
            .validate_citations(&BTreeSet::from([evidence.clone()]))
            .unwrap();
    }
}

proptest! {
    #[test]
    fn arbitrary_user_notes_survive_round_trips(notes in "[^\\p{C}<`]{0,256}") {
        let original = node(&format!("Human note: {notes}"));
        prop_assert_eq!(NodeDocument::parse(&original).unwrap().render().unwrap(), original);
    }

    #[test]
    fn arbitrary_unicode_input_never_panics(text in any::<String>()) {
        if let Ok(document) = NodeDocument::parse(&text) { prop_assert_eq!(document.render().unwrap(), text); }
    }
}
