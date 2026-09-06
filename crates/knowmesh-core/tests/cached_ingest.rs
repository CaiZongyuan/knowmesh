use std::sync::atomic::{AtomicUsize, Ordering};

use knowmesh_core::{
    canonical::workspace::{InitOptions, Workspace, initialize},
    domain::{SourceRevision, SourceRevisionId, sha256},
    error::AppResult,
    ingest::{
        ParseLimits, ParsedSource, ParserDescriptor, TextParser,
        cache::{FileStageCache, StageKey, chunk_cached, parse_cached},
        chunking::ChunkOptions,
    },
    ports::SourceParser,
};

struct CountingParser {
    calls: AtomicUsize,
    inner: TextParser,
}
impl SourceParser for CountingParser {
    fn descriptor(&self, mime: &str) -> AppResult<ParserDescriptor> {
        self.inner.descriptor(mime)
    }
    fn parse(&self, revision: &SourceRevision, bytes: &[u8]) -> AppResult<ParsedSource> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.inner.parse(revision, bytes)
    }
}

#[test]
fn cached_parse_and_chunk_revalidate_artifacts_and_respect_configuration_and_revision() {
    let temp = tempfile::tempdir().unwrap();
    initialize(temp.path(), &InitOptions::default()).unwrap();
    let workspace = Workspace::load(temp.path()).unwrap();
    let canonical_before = std::fs::read(temp.path().join("knowmesh.yaml")).unwrap();
    let cache = FileStageCache::new(&workspace, 1024 * 1024).unwrap();
    let parser = CountingParser {
        calls: AtomicUsize::new(0),
        inner: TextParser::default(),
    };
    let text = format!("# Source\n\n{}", "Evidence sentence. ".repeat(30));
    let mut revision = SourceRevision {
        id: SourceRevisionId::new(),
        path: "fixture.md".into(),
        mime_type: "text/markdown".into(),
        encoding: None,
        sha256: sha256(text.as_bytes()),
        byte_size: text.len() as u64,
        captured_at: "2026-09-06T00:00:00Z".parse().unwrap(),
        url: None,
    };
    let first = parse_cached(&cache, &parser, &revision, text.as_bytes()).unwrap();
    assert!(!first.cache_hit);
    assert!(
        parse_cached(&cache, &parser, &revision, text.as_bytes())
            .unwrap()
            .cache_hit
    );
    assert_eq!(parser.calls.load(Ordering::Relaxed), 1);
    assert!(parse_cached(&cache, &parser, &revision, b"changed bytes").is_err());
    let key = StageKey::parse(&revision, parser.descriptor(&revision.mime_type).unwrap());
    let mut corrupt = first.value.clone();
    corrupt.blocks[0].char_end = usize::MAX;
    cache.store(&key, &corrupt).unwrap();
    let repaired = parse_cached(&cache, &parser, &revision, text.as_bytes()).unwrap();
    assert!(!repaired.cache_hit);
    assert_eq!(parser.calls.load(Ordering::Relaxed), 2);
    let options = ChunkOptions {
        target_tokens: 30,
        max_tokens: 40,
        overlap_tokens: 5,
    };
    let chunks = chunk_cached(&cache, &revision, &repaired.value, &options, None).unwrap();
    assert!(!chunks.cache_hit);
    assert!(
        chunk_cached(&cache, &revision, &repaired.value, &options, None)
            .unwrap()
            .cache_hit
    );
    let changed = ChunkOptions {
        target_tokens: 70,
        max_tokens: 90,
        overlap_tokens: 5,
    };
    assert!(
        !chunk_cached(&cache, &revision, &repaired.value, &changed, None)
            .unwrap()
            .cache_hit
    );
    let constrained = CountingParser {
        calls: AtomicUsize::new(0),
        inner: TextParser::new(ParseLimits {
            max_bytes: 1024 * 1024,
            max_blocks: 10,
        })
        .unwrap(),
    };
    assert!(
        !parse_cached(&cache, &constrained, &revision, text.as_bytes())
            .unwrap()
            .cache_hit
    );
    revision.id = SourceRevisionId::new();
    let new = parse_cached(&cache, &parser, &revision, text.as_bytes()).unwrap();
    assert!(!new.cache_hit);
    let new_chunks = chunk_cached(&cache, &revision, &new.value, &options, None).unwrap();
    assert_ne!(chunks.value.chunks[0].id, new_chunks.value.chunks[0].id);
    assert_eq!(
        std::fs::read(temp.path().join("knowmesh.yaml")).unwrap(),
        canonical_before
    );
    assert_eq!(
        std::fs::read_dir(temp.path().join("sources"))
            .unwrap()
            .count(),
        0
    );
}
