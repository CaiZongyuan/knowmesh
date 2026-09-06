#[path = "support/pdf.rs"]
mod fixture;

use knowmesh_core::{
    domain::{SourceRevision, SourceRevisionId, sha256},
    ingest::{ParseLimits, ParseStatus, PdfOptions, PdfParser},
    ports::SourceParser,
};

const TEXT: &[u8] = b"Model A was evaluated on Dataset B. This synthetic report contains selectable evidence for parser tests.";

fn revision(bytes: &[u8]) -> SourceRevision {
    SourceRevision {
        id: SourceRevisionId::new(),
        path: "fixture.pdf".into(),
        mime_type: "application/pdf".into(),
        encoding: None,
        sha256: sha256(bytes),
        byte_size: bytes.len() as u64,
        captured_at: "2026-09-06T00:00:00Z".parse().unwrap(),
        url: None,
    }
}

#[test]
fn selectable_pdf_keeps_page_numbers_character_spans_and_metadata() {
    let bytes = fixture::bytes(fixture::document(&[Some(TEXT), Some(TEXT)], false));
    let revision = revision(&bytes);
    let parser = PdfParser::default();
    let parsed = parser.parse(&revision, &bytes).unwrap();
    assert_eq!(parsed.status, ParseStatus::Ready);
    assert!(parsed.quality.usable_for_compile);
    assert!(parsed.quality.page_map_reliable);
    assert_eq!(parsed.quality.page_count, Some(2));
    assert_eq!(parsed.quality.text_pages, Some(2));
    assert_eq!(parsed.metadata.title.as_deref(), Some("Synthetic report"));
    assert_eq!(
        parsed
            .blocks
            .iter()
            .map(|block| block.page.unwrap())
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(
        parsed
            .blocks
            .iter()
            .all(|block| block.source_bytes.is_none())
    );
    parsed.validate(&revision).unwrap();
    let again = parser.parse(&revision, &bytes).unwrap();
    assert_eq!(
        serde_json::to_value(parsed).unwrap(),
        serde_json::to_value(again).unwrap()
    );
}

#[test]
fn scanned_and_mostly_empty_pdfs_require_ocr_instead_of_compiling() {
    for (pages, code) in [
        (vec![None, None], "PDF_TEXT_LAYER_MISSING"),
        (vec![Some(TEXT), None, None], "PDF_TEXT_PAGES_INSUFFICIENT"),
        (vec![Some(b"short".as_slice())], "PDF_TEXT_TOO_SHORT"),
    ] {
        let bytes = fixture::bytes(fixture::document(&pages, false));
        let parsed = PdfParser::default()
            .parse(&revision(&bytes), &bytes)
            .unwrap();
        assert_eq!(parsed.status, ParseStatus::NeedsOcr);
        assert!(!parsed.quality.usable_for_compile);
        assert!(
            parsed.warnings.iter().any(|warning| warning.code == code),
            "{:?}",
            parsed.warnings
        );
    }
}

#[test]
fn encrypted_pdf_is_blocked_without_releasing_text() {
    let mut doc = fixture::document(&[Some(TEXT)], false);
    let encryption = lopdf::EncryptionState::try_from(lopdf::EncryptionVersion::V2 {
        document: &doc,
        owner_password: "fixture-owner",
        user_password: "fixture-user",
        key_length: 128,
        permissions: lopdf::Permissions::empty(),
    })
    .unwrap();
    doc.encrypt(&encryption).unwrap();
    let bytes = fixture::bytes(doc);
    let parsed = PdfParser::default()
        .parse(&revision(&bytes), &bytes)
        .unwrap();
    assert_eq!(parsed.status, ParseStatus::Blocked);
    assert!(!parsed.quality.usable_for_compile);
    assert!(parsed.blocks.is_empty());
    assert!(
        parsed
            .warnings
            .iter()
            .any(|warning| warning.code == "PDF_ENCRYPTED")
    );
}

#[test]
fn garbled_unicode_and_unreliable_page_maps_fail_quality_checks() {
    let text = [1; 100];
    let bytes = fixture::bytes(fixture::document(&[Some(&text)], true));
    let parsed = PdfParser::default()
        .parse(&revision(&bytes), &bytes)
        .unwrap();
    assert_eq!(parsed.status, ParseStatus::NeedsOcr);
    assert!(
        parsed
            .warnings
            .iter()
            .any(|warning| warning.code == "PDF_TEXT_GARBLED"),
        "{:?} {:?}",
        parsed.warnings,
        parsed.normalized_text,
    );

    let mut doc = fixture::document(&[Some(TEXT)], false);
    let pages = doc
        .catalog()
        .unwrap()
        .get(b"Pages")
        .unwrap()
        .as_reference()
        .unwrap();
    doc.get_object_mut(pages)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Count", 2);
    let bytes = fixture::bytes(doc);
    let parsed = PdfParser::default()
        .parse(&revision(&bytes), &bytes)
        .unwrap();
    assert!(!parsed.quality.page_map_reliable);
    assert_eq!(parsed.status, ParseStatus::NeedsOcr);
    assert!(parsed.blocks.iter().all(|block| block.page.is_none()));
    let relaxed = PdfParser::new(
        ParseLimits::default(),
        PdfOptions {
            require_page_map: false,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        relaxed
            .parse(&revision(&bytes), &bytes)
            .unwrap()
            .quality
            .usable_for_compile
    );
}

#[test]
fn malformed_revision_and_decompression_or_output_limits_return_typed_errors() {
    let parser = PdfParser::default();
    let bytes = b"%PDF-1.7\nnot a document";
    assert_eq!(
        parser.parse(&revision(bytes), bytes).unwrap_err().code,
        "INVALID_PDF"
    );
    let source = revision(bytes);
    assert_eq!(
        parser.parse(&source, b"changed").unwrap_err().code,
        "SOURCE_REVISION_CHANGED"
    );
    let text = vec![b'a'; 4096];
    let mut doc = fixture::document(&[Some(&text)], false);
    doc.compress();
    let bytes = fixture::bytes(doc);
    let parser = PdfParser::new(
        ParseLimits::default(),
        PdfOptions {
            max_decompressed_bytes: 128,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        parser.parse(&revision(&bytes), &bytes).unwrap_err().code,
        "SOURCE_PARSE_LIMIT"
    );
    let parser = PdfParser::new(
        ParseLimits::default(),
        PdfOptions {
            max_text_bytes: 128,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        parser.parse(&revision(&bytes), &bytes).unwrap_err().code,
        "SOURCE_PARSE_LIMIT"
    );
}

#[test]
fn explicit_unicode_maps_take_precedence_and_broken_maps_cannot_silently_fall_back() {
    let text = [1; 100];
    let mut doc = fixture::document(&[Some(&text)], true);
    for object in doc.objects.values_mut() {
        if let Ok(font) = object.as_dict_mut()
            && font.has_type(b"Font")
        {
            font.set("Encoding", "WinAnsiEncoding");
        }
    }
    let bytes = fixture::bytes(doc);
    let parsed = PdfParser::default()
        .parse(&revision(&bytes), &bytes)
        .unwrap();
    assert!(
        parsed
            .warnings
            .iter()
            .any(|warning| warning.code == "PDF_TEXT_GARBLED")
    );

    let mut doc = fixture::document(&[Some(TEXT)], false);
    let map = doc.add_object(lopdf::Stream::new(
        lopdf::dictionary! {},
        b"broken map".to_vec(),
    ));
    for object in doc.objects.values_mut() {
        if let Ok(font) = object.as_dict_mut()
            && font.has_type(b"Font")
        {
            font.set("ToUnicode", map);
        }
    }
    let bytes = fixture::bytes(doc);
    let parsed = PdfParser::default()
        .parse(&revision(&bytes), &bytes)
        .unwrap();
    assert_eq!(parsed.status, ParseStatus::NeedsOcr);
    assert!(
        parsed
            .warnings
            .iter()
            .any(|warning| warning.code == "PDF_TEXT_EXTRACTION_FAILED")
    );
}
