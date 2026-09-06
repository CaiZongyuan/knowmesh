use knowmesh_core::{
    domain::{SourceRevision, SourceRevisionId, sha256},
    ingest::{
        TextParser,
        chunking::{ChunkOptions, CounterDescriptor, chunk},
    },
    ports::{SourceParser, TokenCounter},
};

struct Characters;
impl TokenCounter for Characters {
    fn descriptor(&self) -> CounterDescriptor {
        CounterDescriptor {
            name: "fixture-character-counter".into(),
            version: "1".into(),
            config_sha256: sha256(b"characters"),
        }
    }
    fn count(&self, text: &str) -> usize {
        text.chars().count()
    }
}

fn revision(text: &str) -> SourceRevision {
    SourceRevision {
        id: SourceRevisionId::new(),
        path: "fixture.md".into(),
        mime_type: "text/markdown".into(),
        encoding: None,
        sha256: sha256(text.as_bytes()),
        byte_size: text.len() as u64,
        captured_at: "2026-09-06T00:00:00Z".parse().unwrap(),
        url: None,
    }
}

#[test]
fn chunks_keep_top_sections_and_exact_source_spans_with_bounded_overlap() {
    let text = format!(
        "# First\n\n{}\n\n# Second\n\n{}\n",
        "细胞 model. ".repeat(20),
        "Other evidence. ".repeat(20)
    );
    let revision = revision(&text);
    let parsed = TextParser::default()
        .parse(&revision, text.as_bytes())
        .unwrap();
    let options = ChunkOptions {
        target_tokens: 60,
        max_tokens: 90,
        overlap_tokens: 10,
    };
    let report = chunk(&revision, &parsed, &options, Some(&Characters)).unwrap();
    assert!(report.chunks.len() > 2);
    let chars: Vec<_> = parsed.normalized_text.chars().collect();
    let boundary = parsed
        .blocks
        .iter()
        .find(|block| block.text == "Second")
        .unwrap()
        .char_start;
    for (index, item) in report.chunks.iter().enumerate() {
        assert_eq!(item.ordinal as usize, index);
        assert!(item.token_count <= 90);
        assert!(!item.token_count_estimated);
        assert_eq!(
            item.text,
            chars[item.char_start..item.char_end]
                .iter()
                .collect::<String>()
        );
        assert!(item.char_end <= boundary || item.char_start >= boundary);
        assert!(!item.source_block_ids.is_empty());
        assert_eq!(item.content_sha256, sha256(item.text.as_bytes()));
    }
    assert!(
        report
            .chunks
            .windows(2)
            .any(|pair| pair[1].char_start < pair[0].char_end)
    );
    let again = chunk(&revision, &parsed, &options, Some(&Characters)).unwrap();
    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        serde_json::to_value(again).unwrap()
    );
    report.validate(&revision, &parsed).unwrap();
}

#[test]
fn chunk_configuration_changes_do_not_mutate_evidence_locator_coordinates() {
    let text = format!("# Source\n\n{}", "Evidence sentence. ".repeat(30));
    let revision = revision(&text);
    let parsed = TextParser::default()
        .parse(&revision, text.as_bytes())
        .unwrap();
    let before = serde_json::to_value(&parsed).unwrap();
    let short = chunk(
        &revision,
        &parsed,
        &ChunkOptions {
            target_tokens: 40,
            max_tokens: 50,
            overlap_tokens: 5,
        },
        Some(&Characters),
    )
    .unwrap();
    let long = chunk(
        &revision,
        &parsed,
        &ChunkOptions {
            target_tokens: 150,
            max_tokens: 180,
            overlap_tokens: 10,
        },
        Some(&Characters),
    )
    .unwrap();
    assert!(short.chunks.len() > long.chunks.len());
    assert_eq!(serde_json::to_value(&parsed).unwrap(), before);
    assert_ne!(short.config_sha256, long.config_sha256);
    let mut corrupt = short.clone();
    corrupt.chunks[0].char_end = usize::MAX;
    assert_eq!(
        corrupt.validate(&revision, &parsed).unwrap_err().code,
        "INVALID_CHUNK_ARTIFACT"
    );
    let mut missing = short.clone();
    missing.chunks.pop();
    assert_eq!(
        missing.validate(&revision, &parsed).unwrap_err().code,
        "INVALID_CHUNK_ARTIFACT"
    );
}

#[test]
fn absent_tokenizer_uses_language_aware_estimates_and_is_part_of_cache_identity() {
    let text = "# 研究\n\n细胞扰动预测与 gene expression 变化。";
    let revision = revision(text);
    let parsed = TextParser::default()
        .parse(&revision, text.as_bytes())
        .unwrap();
    let report = chunk(&revision, &parsed, &ChunkOptions::default(), None).unwrap();
    assert!(report.chunks.iter().all(|item| item.token_count_estimated));
    assert!(report.chunks[0].token_count > report.chunks[0].text.chars().count() / 4);
    let exact = chunk(
        &revision,
        &parsed,
        &ChunkOptions::default(),
        Some(&Characters),
    )
    .unwrap();
    assert_ne!(report.config_sha256, exact.config_sha256);
}

#[test]
fn tables_are_kept_whole_when_they_fit_and_page_boundaries_are_preserved() {
    let text = "# Source\n\nBefore.\n\n| Gene | Result |\n| --- | --- |\n| TP53 | up |\n\nAfter.";
    let revision = revision(text);
    let mut parsed = TextParser::default()
        .parse(&revision, text.as_bytes())
        .unwrap();
    let report = chunk(
        &revision,
        &parsed,
        &ChunkOptions::default(),
        Some(&Characters),
    )
    .unwrap();
    assert!(
        report
            .chunks
            .iter()
            .any(|item| item.text == "Gene\tResult\nTP53\tup")
    );
    for block in &mut parsed.blocks {
        block.page = Some(if block.text == "After." { 2 } else { 1 });
    }
    parsed.quality.page_count = Some(2);
    parsed.quality.page_map_reliable = true;
    let report = chunk(
        &revision,
        &parsed,
        &ChunkOptions::default(),
        Some(&Characters),
    )
    .unwrap();
    assert!(report.chunks.iter().all(|item| item.page.is_some()));
    assert!(
        report
            .chunks
            .iter()
            .any(|item| item.page == Some(2) && item.text == "After.")
    );
}

#[test]
fn invalid_budgets_and_noncompilable_or_corrupt_sources_do_not_produce_chunks() {
    let text = "# Source";
    let revision = revision(text);
    let mut parsed = TextParser::default()
        .parse(&revision, text.as_bytes())
        .unwrap();
    for options in [
        ChunkOptions {
            target_tokens: 0,
            max_tokens: 10,
            overlap_tokens: 0,
        },
        ChunkOptions {
            target_tokens: 20,
            max_tokens: 10,
            overlap_tokens: 1,
        },
        ChunkOptions {
            target_tokens: 10,
            max_tokens: 20,
            overlap_tokens: 10,
        },
    ] {
        assert_eq!(
            chunk(&revision, &parsed, &options, None).unwrap_err().code,
            "INVALID_CHUNK_OPTIONS"
        );
    }
    parsed.status = knowmesh_core::ingest::ParseStatus::NeedsOcr;
    parsed.quality.usable_for_compile = false;
    assert_eq!(
        chunk(&revision, &parsed, &ChunkOptions::default(), None)
            .unwrap_err()
            .code,
        "SOURCE_NOT_COMPILABLE"
    );
}
