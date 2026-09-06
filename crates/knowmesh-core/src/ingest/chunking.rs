use std::{collections::BTreeSet, ops::Range};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use text_splitter::{ChunkConfig, ChunkSizer, TextSplitter};

use super::{BlockKind, ParsedSource, SourceBlock};

pub const CHUNKER_VERSION: &str = "1";
use crate::{
    domain::{ChunkId, SourceBlockId, SourceRevision, SourceRevisionId, sha256, valid_sha256},
    error::{AppError, AppResult, ErrorType},
    ports::TokenCounter,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ChunkOptions {
    pub target_tokens: usize,
    pub max_tokens: usize,
    pub overlap_tokens: usize,
}

impl Default for ChunkOptions {
    fn default() -> Self {
        Self {
            target_tokens: 700,
            max_tokens: 1000,
            overlap_tokens: 100,
        }
    }
}

impl ChunkOptions {
    pub fn validate(&self) -> AppResult<()> {
        if self.target_tokens == 0
            || self.target_tokens > self.max_tokens
            || self.max_tokens > 1_000_000
            || self.overlap_tokens >= self.target_tokens
        {
            return Err(AppError::new(
                ErrorType::Validation,
                "INVALID_CHUNK_OPTIONS",
                "Chunk budgets require 0 <= overlap < target <= max <= 1000000.",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CounterDescriptor {
    pub name: String,
    pub version: String,
    pub config_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SourceChunk {
    pub id: ChunkId,
    pub source_revision_id: SourceRevisionId,
    pub ordinal: u32,
    pub text: String,
    pub content_sha256: String,
    pub char_start: usize,
    pub char_end: usize,
    pub page: Option<u32>,
    pub section_path: Vec<String>,
    pub source_block_ids: Vec<SourceBlockId>,
    pub token_count: usize,
    pub token_count_estimated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChunkReport {
    pub version: u32,
    pub source_revision_id: SourceRevisionId,
    pub parsed_sha256: String,
    pub config_sha256: String,
    pub counter: CounterDescriptor,
    pub options: ChunkOptions,
    pub chunks: Vec<SourceChunk>,
    pub warnings: Vec<String>,
}

struct Sizer<'a>(Option<&'a dyn TokenCounter>);
impl ChunkSizer for Sizer<'_> {
    fn size(&self, text: &str) -> usize {
        self.0
            .map_or_else(|| estimate(text), |counter| counter.count(text))
    }
}

fn estimate(text: &str) -> usize {
    let units: usize = text
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch.is_whitespace() {
                1
            } else if ch.is_ascii() {
                4
            } else if ch as u32 >= 0x1f000 {
                16
            } else {
                8
            }
        })
        .sum();
    units.div_ceil(4)
}

pub fn chunk(
    revision: &SourceRevision,
    parsed: &ParsedSource,
    options: &ChunkOptions,
    counter: Option<&dyn TokenCounter>,
) -> AppResult<ChunkReport> {
    options.validate()?;
    parsed.validate(revision)?;
    if !parsed.quality.usable_for_compile {
        return Err(AppError::new(
            ErrorType::Policy,
            "SOURCE_NOT_COMPILABLE",
            "Source extraction did not pass its quality gate.",
        ));
    }
    let descriptor = counter_descriptor(counter)?;
    let mut report = ChunkReport {
        version: 1,
        source_revision_id: revision.id.clone(),
        parsed_sha256: json_hash(parsed)?,
        config_sha256: config_hash(options, &descriptor)?,
        counter: descriptor,
        options: options.clone(),
        chunks: vec![],
        warnings: vec![],
    };
    split_chunks(revision, parsed, options, counter, &mut report)?;
    report.validate(revision, parsed)?;
    Ok(report)
}

pub(super) fn counter_descriptor(
    counter: Option<&dyn TokenCounter>,
) -> AppResult<CounterDescriptor> {
    let descriptor = match counter {
        Some(counter) => counter.descriptor(),
        None => CounterDescriptor {
            name: "language-aware-character-estimate".into(),
            version: "1".into(),
            config_sha256: json_hash(
                &serde_json::json!({"version":1,"ascii_word_or_space":1,"ascii_punctuation":4,"non_ascii":8,"high_plane":16,"units_per_token":4}),
            )?,
        },
    };
    if descriptor.name.is_empty()
        || descriptor.version.is_empty()
        || !valid_sha256(&descriptor.config_sha256)
    {
        return Err(invalid());
    }
    Ok(descriptor)
}

fn split_chunks(
    revision: &SourceRevision,
    parsed: &ParsedSource,
    options: &ChunkOptions,
    counter: Option<&dyn TokenCounter>,
    report: &mut ChunkReport,
) -> AppResult<()> {
    let block_bytes = byte_ranges(parsed);
    let config = ChunkConfig::new(options.target_tokens..=options.max_tokens)
        .with_overlap(options.overlap_tokens)
        .map_err(|_| invalid())?
        .with_sizer(Sizer(counter));
    let splitter = TextSplitter::new(config);
    for group in groups(parsed) {
        let start = block_bytes[group.start].start;
        let end = block_bytes[group.end - 1].end;
        let segment = &parsed.normalized_text[start..end];
        let atomic_table = parsed.blocks[group.start].kind == BlockKind::Table;
        let parts: Vec<_> = if atomic_table && Sizer(counter).size(segment) <= options.max_tokens {
            vec![(0, segment)]
        } else {
            splitter.chunk_indices(segment).collect()
        };
        if atomic_table
            && parts.len() > 1
            && !report.warnings.iter().any(|code| code == "TABLE_SPLIT")
        {
            report.warnings.push("TABLE_SPLIT".into());
        }
        let mut previous_byte = 0;
        let mut previous_char = parsed.blocks[group.start].char_start;
        for (offset, text) in parts {
            let char_start = previous_char + segment[previous_byte..offset].chars().count();
            let char_end = char_start + text.chars().count();
            previous_byte = offset;
            previous_char = char_start;
            let token_count = Sizer(counter).size(text);
            if token_count == 0 || token_count > options.max_tokens {
                return Err(AppError::new(
                    ErrorType::Policy,
                    "CHUNK_BUDGET_EXCEEDED",
                    "A chunk cannot fit the configured token budget.",
                ));
            }
            let relevant = overlapping(&parsed.blocks, char_start, char_end);
            let ordinal = u32::try_from(report.chunks.len()).map_err(|_| invalid())?;
            let hash = sha256(text.as_bytes());
            let identity = serde_json::to_vec(&("source-chunk-v1", &revision.id, ordinal, &hash))
                .map_err(|_| invalid())?;
            report.chunks.push(SourceChunk {
                id: ChunkId::from_digest(Sha256::digest(identity).into()),
                source_revision_id: revision.id.clone(),
                ordinal,
                text: text.into(),
                content_sha256: hash,
                char_start,
                char_end,
                page: relevant.first().and_then(|block| block.page),
                section_path: common_section(relevant),
                source_block_ids: relevant.iter().map(|block| block.id.clone()).collect(),
                token_count,
                token_count_estimated: counter.is_none(),
            });
        }
    }
    Ok(())
}

fn groups(parsed: &ParsedSource) -> Vec<Range<usize>> {
    let mut groups = Vec::new();
    let mut start = 0;
    let isolate = parsed.quality.page_count.is_some() && !parsed.quality.page_map_reliable;
    for index in 1..parsed.blocks.len() {
        let current = &parsed.blocks[index];
        let previous = &parsed.blocks[index - 1];
        if current.heading_level == Some(1)
            || current.page != previous.page
            || isolate
            || current.kind == BlockKind::Table
            || previous.kind == BlockKind::Table
        {
            groups.push(start..index);
            start = index;
        }
    }
    if start < parsed.blocks.len() {
        groups.push(start..parsed.blocks.len());
    }
    groups
}

fn byte_ranges(parsed: &ParsedSource) -> Vec<Range<usize>> {
    let mut position = 0;
    parsed
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| {
            if index > 0 {
                position += 2;
            }
            let start = position;
            position += block.text.len();
            start..position
        })
        .collect()
}

fn overlapping(blocks: &[SourceBlock], start: usize, end: usize) -> &[SourceBlock] {
    let first = blocks.partition_point(|block| block.char_end <= start);
    let last = blocks.partition_point(|block| block.char_start < end);
    &blocks[first..last]
}

fn common_section(blocks: &[SourceBlock]) -> Vec<String> {
    let Some(first) = blocks.first() else {
        return vec![];
    };
    let mut length = first.section_path.len();
    for block in &blocks[1..] {
        length = first
            .section_path
            .iter()
            .zip(&block.section_path)
            .take(length)
            .take_while(|(left, right)| left == right)
            .count();
    }
    first.section_path[..length].to_vec()
}

pub(super) fn json_hash(value: &impl Serialize) -> AppResult<String> {
    serde_json::to_vec(value)
        .map(|bytes| sha256(&bytes))
        .map_err(|_| invalid())
}

pub(super) fn config_hash(
    options: &ChunkOptions,
    counter: &CounterDescriptor,
) -> AppResult<String> {
    json_hash(&(CHUNKER_VERSION, options, counter))
}

impl ChunkReport {
    pub fn validate(&self, revision: &SourceRevision, parsed: &ParsedSource) -> AppResult<()> {
        parsed.validate(revision)?;
        self.options.validate()?;
        if self.version != 1
            || self.source_revision_id != revision.id
            || self.parsed_sha256 != json_hash(parsed)?
            || self.config_sha256 != config_hash(&self.options, &self.counter)?
        {
            return Err(invalid());
        }
        let mut ids = BTreeSet::new();
        let mut previous_char = 0;
        let mut previous_byte = 0;
        let mut covered_end = 0;
        if !parsed.quality.usable_for_compile || self.chunks.is_empty() {
            return Err(invalid());
        }
        for (ordinal, chunk) in self.chunks.iter().enumerate() {
            if chunk.char_start < previous_char
                || chunk.char_start >= chunk.char_end
                || chunk.source_revision_id != revision.id
                || chunk.ordinal as usize != ordinal
                || !ids.insert(&chunk.id)
                || chunk.content_sha256 != sha256(chunk.text.as_bytes())
                || chunk.token_count == 0
                || chunk.token_count > self.options.max_tokens
            {
                return Err(invalid());
            }
            let skip = chunk.char_start - previous_char;
            let offset = parsed.normalized_text[previous_byte..]
                .char_indices()
                .nth(skip)
                .map(|(byte, _)| byte)
                .ok_or_else(invalid)?;
            let start = previous_byte.checked_add(offset).ok_or_else(invalid)?;
            let end = start.checked_add(chunk.text.len()).ok_or_else(invalid)?;
            if parsed.normalized_text.get(start..end) != Some(chunk.text.as_str())
                || chunk.char_end - chunk.char_start != chunk.text.chars().count()
            {
                return Err(invalid());
            }
            if start > covered_end
                && !parsed.normalized_text[covered_end..start]
                    .chars()
                    .all(char::is_whitespace)
            {
                return Err(invalid());
            }
            covered_end = covered_end.max(end);
            previous_char = chunk.char_start;
            previous_byte = start;
            let blocks = overlapping(&parsed.blocks, chunk.char_start, chunk.char_end);
            if blocks.is_empty()
                || chunk.source_block_ids
                    != blocks
                        .iter()
                        .map(|block| block.id.clone())
                        .collect::<Vec<_>>()
                || chunk.section_path != common_section(blocks)
                || blocks.iter().any(|block| block.page != chunk.page)
                || blocks
                    .iter()
                    .skip(1)
                    .any(|block| block.heading_level == Some(1))
                || (blocks.len() > 1 && blocks.iter().any(|block| block.kind == BlockKind::Table))
                || (blocks.len() > 1
                    && parsed.quality.page_count.is_some()
                    && !parsed.quality.page_map_reliable)
            {
                return Err(invalid());
            }
        }
        if !parsed.normalized_text[covered_end..]
            .chars()
            .all(char::is_whitespace)
        {
            return Err(invalid());
        }
        Ok(())
    }
}

fn invalid() -> AppError {
    AppError::new(
        ErrorType::Validation,
        "INVALID_CHUNK_ARTIFACT",
        "Chunk data is inconsistent with its parsed source or configuration.",
    )
}
