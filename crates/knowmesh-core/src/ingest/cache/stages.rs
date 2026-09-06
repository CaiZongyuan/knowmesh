use super::{CacheHit, FileStageCache, StageKey};
use crate::{
    domain::{SourceRevision, sha256},
    error::{AppError, AppResult, ErrorType},
    ingest::{
        ParsedSource, ParserDescriptor,
        chunking::{self, ChunkOptions, ChunkReport},
    },
    ports::{SourceParser, TokenCounter},
};

pub fn parse_cached(
    cache: &FileStageCache,
    parser: &dyn SourceParser,
    revision: &SourceRevision,
    bytes: &[u8],
) -> AppResult<CacheHit<ParsedSource>> {
    if bytes.len() as u64 != revision.byte_size || sha256(bytes) != revision.sha256 {
        return Err(AppError::new(
            ErrorType::Conflict,
            "SOURCE_REVISION_CHANGED",
            "Source bytes do not match the requested revision.",
        ));
    }
    let descriptor = parser.descriptor(&revision.mime_type)?;
    let key = StageKey::parse(revision, descriptor.clone());
    let validate = |parsed: &ParsedSource| validate_parsed(parsed, revision, &descriptor);
    if let Some(cached) = cache.load(&key, validate)? {
        return Ok(cached);
    }
    let value = parser.parse(revision, bytes)?;
    validate_parsed(&value, revision, &descriptor)?;
    let reference = cache.store(&key, &value)?;
    Ok(CacheHit {
        value,
        reference,
        cache_hit: false,
    })
}

pub fn chunk_cached(
    cache: &FileStageCache,
    revision: &SourceRevision,
    parsed: &ParsedSource,
    options: &ChunkOptions,
    counter: Option<&dyn TokenCounter>,
) -> AppResult<CacheHit<ChunkReport>> {
    options.validate()?;
    parsed.validate(revision)?;
    if !parsed.quality.usable_for_compile {
        return Err(AppError::new(
            ErrorType::Policy,
            "SOURCE_NOT_COMPILABLE",
            "Source extraction did not pass its quality gate.",
        ));
    }
    let descriptor = chunking::counter_descriptor(counter)?;
    let config = chunking::config_hash(options, &descriptor)?;
    let key = StageKey::Chunk {
        parsed_sha256: chunking::json_hash(parsed)?,
        chunker_version: chunking::CHUNKER_VERSION.into(),
        options: options.clone(),
        counter: descriptor,
    };
    if let Some(cached) = cache.load(&key, |report: &ChunkReport| {
        report.validate(revision, parsed)?;
        if report.config_sha256 != config {
            return Err(AppError::new(
                ErrorType::Validation,
                "INVALID_CHUNK_ARTIFACT",
                "Cached chunk configuration does not match the request.",
            ));
        }
        Ok(())
    })? {
        return Ok(cached);
    }
    let value = chunking::chunk(revision, parsed, options, counter)?;
    let reference = cache.store(&key, &value)?;
    Ok(CacheHit {
        value,
        reference,
        cache_hit: false,
    })
}

fn validate_parsed(
    parsed: &ParsedSource,
    revision: &SourceRevision,
    descriptor: &ParserDescriptor,
) -> AppResult<()> {
    parsed.validate(revision)?;
    if parsed.parser_name != descriptor.name
        || parsed.parser_version != descriptor.version
        || parsed.parser_config_sha256 != descriptor.config_sha256
    {
        return Err(AppError::new(
            ErrorType::Validation,
            "INVALID_PARSED_SOURCE",
            "Parsed source does not match the requested parser profile.",
        ));
    }
    Ok(())
}
