use knowmesh_core::{
    application::evidence_verify::{EvidenceInput, EvidenceVerifier, VerificationOptions},
    domain::{EvidenceStance, ExtractionMethod, Locator, SourceRevision, SourceRevisionId, sha256},
    ingest::{ParsedSource, TextParser},
    ports::SourceParser,
};

fn source(text: &str) -> (SourceRevision, ParsedSource) {
    let revision = SourceRevision {
        id: SourceRevisionId::new(),
        path: "fixture.md".into(),
        mime_type: "text/markdown".into(),
        encoding: None,
        sha256: sha256(text.as_bytes()),
        byte_size: text.len() as u64,
        captured_at: "2026-09-06T00:00:00Z".parse().unwrap(),
        url: None,
    };
    let parsed = TextParser::default()
        .parse(&revision, text.as_bytes())
        .unwrap();
    (revision, parsed)
}

fn input(
    revision: &SourceRevision,
    parsed: &ParsedSource,
    block: usize,
    quote: &str,
) -> EvidenceInput {
    let block = &parsed.blocks[block];
    EvidenceInput {
        source_revision_id: revision.id.clone(),
        quote: quote.into(),
        stance: EvidenceStance::Supports,
        extraction_method: ExtractionMethod::Model,
        confidence: 0.9,
        locator: Locator {
            page: block.page,
            section_path: block.section_path.clone(),
            paragraph: block.paragraph,
            char_start: Some(block.char_start),
            char_end: Some(block.char_end),
        },
    }
}

#[test]
fn exact_unicode_spans_produce_canonical_evidence_and_stable_quote_hashes() {
    let (revision, parsed) = source("# Results\n\n细胞 TP53 prediction is supported.\n");
    let verifier =
        EvidenceVerifier::new(&revision, &parsed, VerificationOptions::default()).unwrap();
    let request = input(&revision, &parsed, 1, "细胞 TP53 prediction is supported.");
    let verified = verifier.verify(&request).unwrap();
    assert!(!verified.locator_repaired);
    assert_eq!(verified.evidence().quote, request.quote);
    assert_eq!(
        verified.evidence().quote_sha256,
        sha256(request.quote.as_bytes())
    );
    assert_eq!(verified.evidence().source_revision_id, revision.id);
    assert_eq!(verified.evidence().locator, request.locator);
    verified.evidence().validate().unwrap();
}

#[test]
fn whitespace_normalization_is_allowed_but_words_case_and_punctuation_are_not_invented() {
    let (revision, parsed) = source("# Results\n\nAlpha beta 结果.\n");
    let verifier =
        EvidenceVerifier::new(&revision, &parsed, VerificationOptions::default()).unwrap();
    let request = input(&revision, &parsed, 1, " Alpha\n\t beta\u{a0}结果. ");
    let verified = verifier.verify(&request).unwrap();
    assert_eq!(verified.evidence().quote, "Alpha beta 结果.");
    for quote in [
        "alpha beta 结果.",
        "Alpha beta 结果!",
        "Alpha invented 结果.",
    ] {
        let mut request = request.clone();
        request.quote = quote.into();
        assert_eq!(
            verifier.verify(&request).unwrap_err().code,
            "EVIDENCE_QUOTE_NOT_FOUND"
        );
    }
}

#[test]
fn bounded_unique_repairs_move_offsets_without_crossing_section_or_page() {
    let (revision, mut parsed) =
        source("# A\n\nPrefix. Unique evidence here. Suffix.\n\n# B\n\nOther evidence.\n");
    for block in &mut parsed.blocks {
        block.page = Some(if block.section_path == ["A"] { 1 } else { 2 });
    }
    parsed.quality.page_count = Some(2);
    parsed.quality.page_map_reliable = true;
    let verifier = EvidenceVerifier::new(
        &revision,
        &parsed,
        VerificationOptions {
            repair_window_chars: 12,
            ..Default::default()
        },
    )
    .unwrap();
    let mut request = input(&revision, &parsed, 1, "Unique evidence here.");
    request.locator.char_start = Some(parsed.blocks[1].char_start + 9);
    request.locator.char_end = Some(parsed.blocks[1].char_start + 29);
    let verified = verifier.verify(&request).unwrap();
    assert!(verified.locator_repaired);
    assert_eq!(
        verified.evidence().locator.char_start,
        Some(parsed.blocks[1].char_start + 8)
    );
    assert_eq!(verified.evidence().locator.page, Some(1));
    request.locator.page = Some(2);
    assert_eq!(
        verifier.verify(&request).unwrap_err().code,
        "EVIDENCE_SCOPE_MISMATCH"
    );
    request.locator.page = Some(1);
    request.locator.section_path = vec!["B".into()];
    assert_eq!(
        verifier.verify(&request).unwrap_err().code,
        "EVIDENCE_SCOPE_MISMATCH"
    );
}

#[test]
fn ambiguous_and_overlapping_repairs_are_rejected_but_exact_duplicate_quotes_are_valid() {
    let (revision, parsed) = source("# A\n\nEcho. Echo.\n\naaaa\n");
    let verifier =
        EvidenceVerifier::new(&revision, &parsed, VerificationOptions::default()).unwrap();
    let mut request = input(&revision, &parsed, 1, "Echo.");
    assert_eq!(
        verifier.verify(&request).unwrap_err().code,
        "EVIDENCE_QUOTE_AMBIGUOUS"
    );
    request.locator.char_end = Some(request.locator.char_start.unwrap() + 5);
    assert!(!verifier.verify(&request).unwrap().locator_repaired);
    let request = input(&revision, &parsed, 2, "aaa");
    assert_eq!(
        verifier.verify(&request).unwrap_err().code,
        "EVIDENCE_QUOTE_AMBIGUOUS"
    );
}

#[test]
fn explicit_paragraph_locators_can_be_completed_and_unbounded_search_is_rejected() {
    let (revision, parsed) = source("# A\n\nUnique evidence.\n");
    let verifier =
        EvidenceVerifier::new(&revision, &parsed, VerificationOptions::default()).unwrap();
    let mut request = input(&revision, &parsed, 1, "Unique evidence.");
    request.locator.char_start = None;
    request.locator.char_end = None;
    let verified = verifier.verify(&request).unwrap();
    assert!(verified.locator_repaired);
    assert_eq!(
        verified.evidence().locator.char_start,
        Some(parsed.blocks[1].char_start)
    );
    request.locator = Locator::default();
    assert_eq!(
        verifier.verify(&request).unwrap_err().code,
        "EVIDENCE_LOCATOR_REQUIRED"
    );
}

#[test]
fn revision_mismatch_invalid_bounds_and_invalid_assertion_metadata_never_create_evidence() {
    let (revision, parsed) = source("# A\n\nEvidence.\n");
    let verifier =
        EvidenceVerifier::new(&revision, &parsed, VerificationOptions::default()).unwrap();
    let base = input(&revision, &parsed, 1, "Evidence.");
    let mut changed = base.clone();
    changed.source_revision_id = SourceRevisionId::new();
    assert_eq!(
        verifier.verify(&changed).unwrap_err().code,
        "EVIDENCE_REVISION_MISMATCH"
    );
    let mut changed = base.clone();
    changed.locator.char_end = Some(usize::MAX);
    assert_eq!(
        verifier.verify(&changed).unwrap_err().code,
        "EVIDENCE_LOCATOR_OUT_OF_BOUNDS"
    );
    let mut changed = base.clone();
    changed.confidence = f64::NAN;
    assert!(verifier.verify(&changed).is_err());
    let mut changed = base;
    changed.quote = "x".repeat(1001);
    assert_eq!(
        verifier.verify(&changed).unwrap_err().code,
        "INVALID_EVIDENCE_QUOTE"
    );
}

#[test]
fn repairs_preserve_unicode_offsets_and_original_whitespace_spans() {
    let (revision, parsed) =
        source("# A\n\n```text\n前置. e\u{301} 🧬 Alpha\t  beta. 后置.\n```\n");
    let verifier =
        EvidenceVerifier::new(&revision, &parsed, VerificationOptions::default()).unwrap();
    let request = input(&revision, &parsed, 1, "e\u{301} 🧬 Alpha beta.");
    let verified = verifier.verify(&request).unwrap();
    let locator = &verified.evidence().locator;
    let actual: String = parsed
        .normalized_text
        .chars()
        .skip(locator.char_start.unwrap())
        .take(locator.char_end.unwrap() - locator.char_start.unwrap())
        .collect();
    assert_eq!(actual, "e\u{301} 🧬 Alpha\t  beta.");
    assert_eq!(verified.evidence().quote, request.quote);
    let mut composed = request;
    composed.quote = "é 🧬 Alpha beta.".into();
    assert_eq!(
        verifier.verify(&composed).unwrap_err().code,
        "EVIDENCE_QUOTE_NOT_FOUND"
    );
}

#[test]
fn supplied_scope_and_cross_boundary_spans_are_checked_even_on_exact_matches() {
    let (revision, mut parsed) = source("# A\n\nFirst.\n\nSecond.\n\n# B\n\nLast.\n");
    for (index, block) in parsed.blocks.iter_mut().enumerate() {
        block.page = Some(if index <= 1 { 1 } else { 2 });
    }
    parsed.quality.page_count = Some(2);
    parsed.quality.page_map_reliable = true;
    let verifier =
        EvidenceVerifier::new(&revision, &parsed, VerificationOptions::default()).unwrap();
    let mut request = input(&revision, &parsed, 1, "First. Second.");
    request.locator.paragraph = None;
    request.locator.char_end = Some(parsed.blocks[2].char_end);
    assert_eq!(
        verifier.verify(&request).unwrap_err().code,
        "EVIDENCE_SCOPE_MISMATCH"
    );
    let mut request = input(&revision, &parsed, 2, "Second. B Last.");
    request.locator.paragraph = None;
    request.locator.char_end = Some(parsed.blocks[4].char_end);
    assert_eq!(
        verifier.verify(&request).unwrap_err().code,
        "EVIDENCE_SCOPE_MISMATCH"
    );
    let mut request = input(&revision, &parsed, 1, "First.");
    request.locator.paragraph = Some(999);
    assert_eq!(
        verifier.verify(&request).unwrap_err().code,
        "EVIDENCE_SCOPE_MISMATCH"
    );
}

#[test]
fn offset_free_scopes_are_unique_complete_and_bounded() {
    let (revision, parsed) = source("# A\n\nEvidence.\n\n# B\n\nElsewhere.\n\n# A\n\nEvidence.\n");
    let verifier =
        EvidenceVerifier::new(&revision, &parsed, VerificationOptions::default()).unwrap();
    let mut request = input(&revision, &parsed, 1, "Evidence.");
    request.locator = Locator {
        section_path: vec!["A".into()],
        ..Default::default()
    };
    assert_eq!(
        verifier.verify(&request).unwrap_err().code,
        "EVIDENCE_QUOTE_AMBIGUOUS"
    );
    request.locator.section_path = vec!["B".into()];
    request.quote = "Elsewhere.".into();
    let verified = verifier.verify(&request).unwrap();
    assert_eq!(
        verified.evidence().locator.char_start,
        Some(parsed.blocks[3].char_start)
    );
    request.locator.section_path = vec!["Missing".into()];
    assert_eq!(
        verifier.verify(&request).unwrap_err().code,
        "EVIDENCE_SCOPE_MISMATCH"
    );
    let verifier = EvidenceVerifier::new(
        &revision,
        &parsed,
        VerificationOptions {
            max_search_chars: 8,
            ..Default::default()
        },
    )
    .unwrap();
    request.locator.section_path = vec!["A".into()];
    request.quote = "A".into();
    assert_eq!(
        verifier.verify(&request).unwrap_err().code,
        "EVIDENCE_SEARCH_LIMIT"
    );
}

#[test]
fn repair_radius_does_not_find_a_distant_quote_in_the_same_block() {
    let (revision, parsed) = source("# A\n\nStart of a fairly long paragraph. Distant evidence.\n");
    let verifier = EvidenceVerifier::new(
        &revision,
        &parsed,
        VerificationOptions {
            repair_window_chars: 2,
            ..Default::default()
        },
    )
    .unwrap();
    let mut request = input(&revision, &parsed, 1, "Distant evidence.");
    request.locator.char_end = Some(parsed.blocks[1].char_start + 5);
    assert_eq!(
        verifier.verify(&request).unwrap_err().code,
        "EVIDENCE_QUOTE_NOT_FOUND"
    );
}

#[test]
fn invalid_parse_artifacts_and_failed_quality_gates_cannot_mint_evidence() {
    let (revision, parsed) = source("# A\n\nEvidence.\n");
    let mut changed = parsed.clone();
    changed.text_sha256 = sha256(b"different");
    assert_eq!(
        EvidenceVerifier::new(&revision, &changed, Default::default())
            .err()
            .unwrap()
            .code,
        "INVALID_PARSED_SOURCE"
    );
    let mut changed = parsed;
    changed.status = knowmesh_core::ingest::ParseStatus::Blocked;
    changed.quality.usable_for_compile = false;
    assert_eq!(
        EvidenceVerifier::new(&revision, &changed, Default::default())
            .err()
            .unwrap()
            .code,
        "SOURCE_NOT_COMPILABLE"
    );
}

#[test]
fn exact_and_repaired_long_unicode_spans_are_independent_of_chunk_configuration() {
    use knowmesh_core::ingest::chunking::{ChunkOptions, chunk};

    let (revision, parsed) = source(&format!("# A\n\n{}Unique 结论.\n", "界🧬".repeat(1100)));
    let verifier = EvidenceVerifier::new(&revision, &parsed, Default::default()).unwrap();
    let mut request = input(&revision, &parsed, 1, "Unique 结论.");
    request.locator.char_start = Some(parsed.blocks[1].char_start + 2200);
    let before = verifier.verify(&request).unwrap();
    assert!(!before.locator_repaired);
    for target in [100, 200] {
        chunk(
            &revision,
            &parsed,
            &ChunkOptions {
                target_tokens: target,
                max_tokens: target * 2,
                overlap_tokens: 10,
            },
            None,
        )
        .unwrap();
        request.locator.char_start = Some(parsed.blocks[1].char_start + 2199);
        let after = verifier.verify(&request).unwrap();
        assert!(after.locator_repaired);
        assert_eq!(before.evidence().locator, after.evidence().locator);
        assert_eq!(
            before.evidence().quote_sha256,
            after.evidence().quote_sha256
        );
    }
}

#[test]
fn cross_paragraph_quotes_use_character_spans_without_inventing_one_paragraph() {
    let (revision, parsed) = source("# A\n\nFirst.\n\nSecond.\n");
    let verifier = EvidenceVerifier::new(&revision, &parsed, Default::default()).unwrap();
    let mut request = input(&revision, &parsed, 1, "First. Second.");
    request.locator.paragraph = None;
    request.locator.char_end = Some(parsed.blocks[2].char_end);
    let verified = verifier.verify(&request).unwrap();
    assert!(!verified.locator_repaired);
    assert_eq!(verified.evidence().locator.paragraph, None);
    request.locator.char_start = None;
    request.locator.char_end = None;
    assert_eq!(
        verifier.verify(&request).unwrap().evidence().locator,
        verified.evidence().locator
    );
}

#[test]
fn incomplete_offsets_unknown_pages_and_zero_repair_radius_are_explicit() {
    let (revision, parsed) = source("# A\n\nPrefix. Evidence.\n");
    let verifier = EvidenceVerifier::new(
        &revision,
        &parsed,
        VerificationOptions {
            repair_window_chars: 0,
            ..Default::default()
        },
    )
    .unwrap();
    let base = input(&revision, &parsed, 1, "Evidence.");
    assert_eq!(
        verifier.verify(&base).unwrap_err().code,
        "EVIDENCE_QUOTE_NOT_FOUND"
    );
    let mut changed = base.clone();
    changed.locator.char_start = None;
    assert_eq!(
        verifier.verify(&changed).unwrap_err().code,
        "INVALID_EVIDENCE_LOCATOR"
    );
    let mut changed = base;
    changed.locator.page = Some(1);
    assert_eq!(
        verifier.verify(&changed).unwrap_err().code,
        "EVIDENCE_SCOPE_MISMATCH"
    );
    assert_eq!(
        EvidenceVerifier::new(
            &revision,
            &parsed,
            VerificationOptions {
                repair_window_chars: usize::MAX,
                ..Default::default()
            },
        )
        .err()
        .unwrap()
        .code,
        "INVALID_EVIDENCE_OPTIONS"
    );
}
