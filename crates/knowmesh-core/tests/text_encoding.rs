use knowmesh_core::{
    domain::{SourceRevision, SourceRevisionId, TextEncoding, decode_source_text, sha256},
    ingest::TextParser,
    ports::SourceParser,
};

#[test]
fn encoding_labels_are_canonical_and_decoding_never_invents_replacement_characters() {
    let encoding: TextEncoding = " LATIN1 ".parse().unwrap();
    assert_eq!(encoding.as_str(), "windows-1252");
    assert_eq!(serde_json::to_value(&encoding).unwrap(), "windows-1252");
    assert_eq!(
        decode_source_text(b"caf\xe9", Some(&encoding)).unwrap(),
        "café"
    );
    assert!(decode_source_text(b"caf\xe9", None).is_err());
    let shift_jis: TextEncoding = "shift_jis".parse().unwrap();
    assert!(decode_source_text(&[0x82], Some(&shift_jis)).is_err());
    let utf16: TextEncoding = "utf-16le".parse().unwrap();
    assert!(decode_source_text(&[0xff, 0xfe, b'A'], Some(&utf16)).is_err());
    assert!(decode_source_text(&[0xfe, 0xff, 0, b'A'], Some(&utf16)).is_err());
    for label in ["", "replacement", "utf-7", "not-an-encoding"] {
        assert!(label.parse::<TextEncoding>().is_err());
    }
}

#[test]
fn decoded_parser_artifacts_are_bound_to_revision_encoding() {
    let bytes = b"caf\xe9";
    let mut revision = SourceRevision {
        id: SourceRevisionId::new(),
        path: "fixture".into(),
        mime_type: "text/plain".into(),
        encoding: Some("windows-1252".parse().unwrap()),
        sha256: sha256(bytes),
        byte_size: bytes.len() as u64,
        captured_at: "2026-09-06T00:00:00Z".parse().unwrap(),
        url: None,
    };
    let parsed = TextParser::default().parse(&revision, bytes).unwrap();
    assert_eq!(parsed.normalized_text, "café");
    assert!(parsed.blocks[0].source_bytes.is_none());
    parsed.validate(&revision).unwrap();
    revision.encoding = Some("windows-1251".parse().unwrap());
    assert_eq!(
        parsed.validate(&revision).unwrap_err().code,
        "INVALID_PARSED_SOURCE"
    );
}
