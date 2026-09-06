use knowmesh_core::{
    domain::{SourceRevision, SourceRevisionId, Timestamp, sha256},
    ingest::{BlockKind, ParseLimits, ParsedSource, TextParser},
    ports::SourceParser,
};
use proptest::prelude::*;

fn revision(bytes: &[u8], mime: &str) -> SourceRevision {
    SourceRevision {
        id: SourceRevisionId::new(),
        path: "fixture".into(),
        mime_type: mime.into(),
        sha256: sha256(bytes),
        byte_size: bytes.len() as u64,
        captured_at: "2026-09-06T00:00:00Z".parse::<Timestamp>().unwrap(),
        url: None,
    }
}

fn check_spans(parsed: &ParsedSource, raw: &str) {
    assert_eq!(
        parsed.text_sha256,
        sha256(parsed.normalized_text.as_bytes())
    );
    let chars: Vec<_> = parsed.normalized_text.chars().collect();
    let mut end = 0;
    for block in &parsed.blocks {
        assert!(block.char_start >= end);
        assert!(block.char_end > block.char_start);
        assert_eq!(
            chars[block.char_start..block.char_end]
                .iter()
                .collect::<String>(),
            block.text
        );
        if let Some(span) = &block.source_bytes {
            assert!(span.start <= span.end && span.end <= raw.len());
            assert!(raw.is_char_boundary(span.start) && raw.is_char_boundary(span.end));
        }
        end = block.char_end;
    }
}

#[test]
fn markdown_preserves_structure_sections_code_tables_and_unicode_spans() {
    let text = include_str!("fixtures/parse/structured.md");
    let parsed = TextParser::default()
        .parse(&revision(text.as_bytes(), "text/markdown"), text.as_bytes())
        .unwrap();
    let actual: Vec<_> = parsed
        .blocks
        .iter()
        .map(|block| (block.kind, block.text.as_str()))
        .collect();
    assert_eq!(
        actual,
        [
            (BlockKind::Heading, "模型"),
            (BlockKind::Paragraph, "Intro with TP53 and paper."),
            (BlockKind::Heading, "Results"),
            (BlockKind::ListItem, "first"),
            (BlockKind::ListItem, "second"),
            (BlockKind::Quote, "Caution."),
            (BlockKind::Code, "x <- 1"),
            (BlockKind::Table, "Gene\tEffect\nTP53\tup"),
        ]
    );
    assert_eq!(parsed.metadata.title.as_deref(), Some("模型"));
    assert_eq!(parsed.blocks[1].section_path, ["模型"]);
    assert_eq!(parsed.blocks[3].section_path, ["模型", "Results"]);
    assert_eq!(parsed.blocks[1].paragraph, Some(1));
    assert_eq!(parsed.blocks[6].language.as_deref(), Some("r"));
    assert!(parsed.blocks.iter().all(|block| block.page.is_none()));
    assert!(parsed.quality.usable_for_compile);
    check_spans(&parsed, text);
}

#[test]
fn html5_parsing_keeps_captions_and_repairs_markup_without_executing_or_fetching_content() {
    let text = include_str!("fixtures/parse/structured.html");
    let parsed = TextParser::default()
        .parse(&revision(text.as_bytes(), "text/html"), text.as_bytes())
        .unwrap();
    let actual: Vec<_> = parsed
        .blocks
        .iter()
        .map(|block| (block.kind, block.text.as_str()))
        .collect();
    assert_eq!(
        actual,
        [
            (BlockKind::Heading, "研究"),
            (
                BlockKind::Paragraph,
                "Cell models & evidence.\nSecond line."
            ),
            (BlockKind::Heading, "Results"),
            (BlockKind::ListItem, "first"),
            (BlockKind::ListItem, "second"),
            (BlockKind::Table, "Expression\nGene\tEffect\nTP53\tup"),
            (BlockKind::FigureCaption, "Figure 1: comparison."),
            (BlockKind::Code, "x <- 1\n  y <- 2"),
        ]
    );
    assert_eq!(
        parsed.metadata.title.as_deref(),
        Some("Research & evidence")
    );
    assert_eq!(parsed.metadata.language.as_deref(), Some("zh-CN"));
    assert_eq!(parsed.blocks[5].caption.as_deref(), Some("Expression"));
    assert_eq!(
        parsed.blocks[6].caption.as_deref(),
        Some("Figure 1: comparison.")
    );
    assert!(
        parsed
            .blocks
            .iter()
            .all(|block| block.source_bytes.is_none())
    );
    assert!(!parsed.normalized_text.contains("private"));
    assert!(!parsed.normalized_text.contains("credentials"));
    check_spans(&parsed, text);
}

#[test]
fn plain_text_preserves_paragraph_content_and_revision_scoped_deterministic_ids() {
    let text = "\u{feff}Alpha\r\n  细胞\tdata\r\n\r\nSecond paragraph.\r\n";
    let source = revision(text.as_bytes(), "text/plain");
    let parser = TextParser::default();
    let first = parser.parse(&source, text.as_bytes()).unwrap();
    let again = parser.parse(&source, text.as_bytes()).unwrap();
    assert_eq!(
        first.normalized_text,
        "Alpha\n  细胞\tdata\n\nSecond paragraph."
    );
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(again).unwrap()
    );
    let other = parser
        .parse(&revision(text.as_bytes(), "text/plain"), text.as_bytes())
        .unwrap();
    assert_ne!(first.blocks[0].id, other.blocks[0].id);
    assert!(first.blocks[0].id.as_str().starts_with("blk_"));
    assert_eq!(first.source_revision_id, source.id);
    assert_eq!(first.source_sha256, source.sha256);
    check_spans(&first, text);
}

#[test]
fn parsing_rejects_revision_changes_bad_encoding_unsupported_types_and_resource_overruns() {
    let parser = TextParser::default();
    let mut source = revision(b"content", "text/plain");
    assert_eq!(
        parser.parse(&source, b"changed").unwrap_err().code,
        "SOURCE_REVISION_CHANGED"
    );
    source.byte_size += 1;
    assert_eq!(
        parser.parse(&source, b"content").unwrap_err().code,
        "SOURCE_REVISION_CHANGED"
    );
    assert_eq!(
        parser
            .parse(&revision(&[0xff], "text/plain"), &[0xff])
            .unwrap_err()
            .code,
        "INVALID_SOURCE_ENCODING"
    );
    assert_eq!(
        parser
            .parse(&revision(b"%PDF-1.7", "application/pdf"), b"%PDF-1.7")
            .unwrap_err()
            .code,
        "SOURCE_PARSER_UNAVAILABLE"
    );
    let parser = TextParser::new(ParseLimits {
        max_bytes: 16,
        max_blocks: 1,
    })
    .unwrap();
    let text = b"one\n\ntwo";
    assert_eq!(
        parser
            .parse(&revision(text, "text/plain"), text)
            .unwrap_err()
            .code,
        "SOURCE_PARSE_LIMIT"
    );
    let text = b"more than sixteen bytes";
    assert_eq!(
        parser
            .parse(&revision(text, "text/plain"), text)
            .unwrap_err()
            .code,
        "SOURCE_PARSE_LIMIT"
    );
    let empty = TextParser::default()
        .parse(&revision(b" \n\t", "text/plain"), b" \n\t")
        .unwrap();
    assert!(!empty.quality.usable_for_compile);
    assert!(
        empty
            .warnings
            .iter()
            .any(|warning| warning.code == "NO_EXTRACTED_TEXT")
    );
}

proptest! {
    #[test]
    fn arbitrary_unicode_text_keeps_valid_character_spans(text in ".{0,512}") {
        let parsed = TextParser::default().parse(&revision(text.as_bytes(), "text/plain"), text.as_bytes()).unwrap();
        check_spans(&parsed, &text);
    }
}
