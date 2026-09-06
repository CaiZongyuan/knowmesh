#[path = "../../../tests/support/mod.rs"]
mod support;

use std::fs;

use knowmesh_core::canonical::node::NodeDocument;

fn document() -> (tempfile::TempDir, NodeDocument, String) {
    let (temp, _) = support::fixture();
    let text = fs::read_to_string(temp.path().join("knowledge/nodes/model-a.md")).unwrap();
    let doc = NodeDocument::parse(&text).unwrap();
    (temp, doc, text)
}

#[test]
fn summary_edits_preserve_other_sections_frontmatter_and_managed_assertions() {
    let (_temp, mut doc, original) = document();
    assert!(
        doc.set_summary("A revised summary with **formatting**.")
            .unwrap()
    );
    let rendered = doc.render().unwrap();
    assert_eq!(
        rendered,
        original.replace(
            "A fictional model.",
            "A revised summary with **formatting**."
        )
    );
    let parsed = NodeDocument::parse(&rendered).unwrap();
    assert_eq!(parsed.claims, doc.claims);
    assert_eq!(parsed.relations, doc.relations);
    assert!(!doc.set_summary("A fictional model.").unwrap());
    assert_eq!(doc.render().unwrap(), original);
}

#[test]
fn an_absent_summary_is_inserted_without_absorbing_existing_notes() {
    let (_temp, doc, _) = document();
    let mut doc =
        NodeDocument::create(doc.metadata, "# Model A\n\n## Notes\n\nKeep this material.").unwrap();
    let before = doc.render().unwrap();
    doc.set_summary("New summary.").unwrap();
    let rendered = doc.render().unwrap();
    assert!(rendered.contains("## Notes\n\nKeep this material."));
    assert!(rendered.contains("## Summary\n\nNew summary."));
    assert!(rendered.ends_with(&before[before.find("<!-- knowmesh:claims:begin -->").unwrap()..]));
    let mut again = NodeDocument::parse(&rendered).unwrap();
    again.set_summary("Another summary.").unwrap();
    assert!(again.render().unwrap().contains("Keep this material."));
}

#[test]
fn summary_parsing_ignores_quoted_and_code_headings_and_preserves_crlf() {
    let (_temp, doc, _) = document();
    let mut doc = NodeDocument::create(doc.metadata, "# Model A\n\n> ## Summary\n> Quoted note.\n\n```md\n## Summary\n```\n\n## Summary\n\nOld text.\n\n## Notes\n\nUntouched.").unwrap();
    let original = doc.render().unwrap().replace('\n', "\r\n");
    doc = NodeDocument::parse(&original).unwrap();
    doc.set_summary("New text.\n\nSecond paragraph.").unwrap();
    assert_eq!(
        doc.render().unwrap(),
        original.replace("Old text.", "New text.\r\n\r\nSecond paragraph.")
    );
}

#[test]
fn duplicate_summary_sections_and_injected_document_structure_are_rejected() {
    let (_temp, doc, _) = document();
    let mut duplicate = NodeDocument::create(
        doc.metadata.clone(),
        "## Summary\n\nOne.\n\n## Summary\n\nTwo.",
    )
    .unwrap();
    assert_eq!(
        duplicate.set_summary("Replacement.").unwrap_err().code,
        "AMBIGUOUS_NODE_SUMMARY"
    );
    let mut doc = doc;
    let original = doc.render().unwrap();
    for text in [
        "## Other section\n\nInjected.",
        "<!-- knowmesh:claims:begin -->",
        "<script>injected()</script>",
        "<<<<<<< branch\ntext",
    ] {
        assert_eq!(
            doc.set_summary(text).unwrap_err().code,
            "INVALID_NODE_SUMMARY"
        );
        assert_eq!(doc.render().unwrap(), original);
    }
    doc.set_summary("```md\n## An example heading\n```\n\n### Details\n\nAllowed content.")
        .unwrap();
    NodeDocument::parse(&doc.render().unwrap()).unwrap();
}
