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
