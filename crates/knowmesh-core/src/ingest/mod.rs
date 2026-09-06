mod builder;
mod frontmatter;
mod html;
mod markdown;
mod plain;
mod types;
mod validate;

pub use types::*;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    domain::{SourceBlockId, SourceRevision, decode_source_text, sha256},
    error::{AppError, AppResult, ErrorType},
    ports::SourceParser,
};

const PARSER_VERSION: &str = "2";

#[derive(Debug, Default)]
pub struct TextParser {
    limits: ParseLimits,
}

impl TextParser {
    pub fn new(limits: ParseLimits) -> AppResult<Self> {
        if limits.max_bytes == 0 || limits.max_blocks == 0 || limits.max_blocks > u32::MAX as usize
        {
            return Err(limit_error());
        }
        Ok(Self { limits })
    }
}

impl SourceParser for TextParser {
    fn descriptor(&self, mime_type: &str) -> AppResult<ParserDescriptor> {
        let name = match mime_type {
            "text/plain" => "knowmesh-text",
            "text/markdown" => "knowmesh-commonmark",
            "text/html" => "knowmesh-html5",
            _ => {
                return Err(AppError::new(
                    ErrorType::Configuration,
                    "SOURCE_PARSER_UNAVAILABLE",
                    "No text parser is available for this source type.",
                ));
            }
        };
        let config = serde_json::to_vec(&self.limits).map_err(|_| {
            AppError::new(
                ErrorType::Internal,
                "PARSER_CONFIG_INVALID",
                "Could not identify parser configuration.",
            )
        })?;
        Ok(ParserDescriptor {
            name: name.into(),
            version: PARSER_VERSION.into(),
            config_sha256: sha256(&config),
        })
    }

    fn parse(&self, revision: &SourceRevision, bytes: &[u8]) -> AppResult<ParsedSource> {
        if bytes.len() > self.limits.max_bytes {
            return Err(limit_error());
        }
        if bytes.len() as u64 != revision.byte_size || sha256(bytes) != revision.sha256 {
            return Err(AppError::new(
                ErrorType::Conflict,
                "SOURCE_REVISION_CHANGED",
                "The parser input does not match the immutable revision.",
            ));
        }
        let parser = self.descriptor(&revision.mime_type)?;
        let text = decode_source_text(bytes, revision.encoding.as_ref())?;
        let source = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let prefix = text.len() - source.len();
        let mut builder = builder::Builder::new(self.limits.max_blocks);
        let metadata = match revision.mime_type.as_str() {
            "text/plain" => {
                plain::parse(source, &mut builder)?;
                ParsedMetadata::default()
            }
            "text/markdown" => markdown::parse(source, &mut builder)?,
            "text/html" => html::parse(source, &mut builder)?,
            _ => unreachable!("parser dispatch was validated"),
        };
        builder.finish(Some(source.len()))?;
        if revision
            .encoding
            .as_ref()
            .is_some_and(|encoding| !encoding.is_utf8())
        {
            for block in &mut builder.blocks {
                block.source_bytes = None;
            }
        }
        let parsed = assemble(revision, parser, prefix, metadata, builder)?;
        parsed.validate(revision)?;
        Ok(parsed)
    }
}

fn assemble(
    revision: &SourceRevision,
    parser: ParserDescriptor,
    prefix: usize,
    mut metadata: ParsedMetadata,
    builder: builder::Builder,
) -> AppResult<ParsedSource> {
    let mut normalized_text = String::new();
    let mut char_position = 0;
    let mut paragraph = 0;
    let mut blocks = Vec::new();
    if metadata.title.is_none() {
        metadata.title = builder
            .blocks
            .iter()
            .find(|block| block.heading_level == Some(1))
            .or_else(|| {
                builder
                    .blocks
                    .iter()
                    .find(|block| block.kind == BlockKind::Heading)
            })
            .map(|block| block.text.clone());
    }
    for (ordinal, draft) in builder.blocks.into_iter().enumerate() {
        if !normalized_text.is_empty() {
            normalized_text.push_str("\n\n");
            char_position += 2;
        }
        let char_start = char_position;
        normalized_text.push_str(&draft.text);
        char_position += draft.text.chars().count();
        let number = if draft.kind == BlockKind::Heading {
            None
        } else {
            paragraph += 1;
            Some(paragraph)
        };
        #[derive(Serialize)]
        struct Identity<'a> {
            version: u32,
            revision: &'a SourceRevision,
            parser: &'a ParserDescriptor,
            ordinal: usize,
            text_sha256: String,
        }
        let identity = serde_json::to_vec(&Identity {
            version: 1,
            revision,
            parser: &parser,
            ordinal,
            text_sha256: sha256(draft.text.as_bytes()),
        })
        .map_err(|_| {
            AppError::new(
                ErrorType::Internal,
                "PARSE_ID_FAILED",
                "Could not identify a parsed block.",
            )
        })?;
        blocks.push(SourceBlock {
            id: SourceBlockId::from_digest(Sha256::digest(identity).into()),
            kind: draft.kind,
            text: draft.text,
            page: None,
            section_path: draft.section_path,
            paragraph: number,
            char_start,
            char_end: char_position,
            source_bytes: draft.source_bytes.map(|span| ByteSpan {
                start: span.start + prefix,
                end: span.end + prefix,
            }),
            heading_level: draft.heading_level,
            language: draft.language,
            caption: draft.caption,
        });
    }
    let visible_characters = normalized_text
        .chars()
        .filter(|ch| !ch.is_whitespace() && !ch.is_control())
        .count();
    let replacement_characters = normalized_text
        .chars()
        .filter(|ch| *ch == '\u{fffd}')
        .count();
    let mut warnings = builder.warnings;
    if visible_characters == 0 {
        warnings.push(ParseWarning {
            code: "NO_EXTRACTED_TEXT".into(),
            hint: "Inspect the source and provide extractable text before compiling it.".into(),
        });
    }
    Ok(ParsedSource {
        version: 1,
        source_revision_id: revision.id.clone(),
        source_sha256: revision.sha256.clone(),
        source_encoding: revision.encoding.clone(),
        text_sha256: sha256(normalized_text.as_bytes()),
        normalized_text,
        metadata,
        blocks,
        warnings,
        quality: ExtractionQuality {
            usable_for_compile: visible_characters > 0,
            visible_characters,
            replacement_characters,
            replacement_ratio: if visible_characters == 0 {
                0.0
            } else {
                replacement_characters as f64 / visible_characters as f64
            },
            page_count: None,
            text_pages: None,
            page_map_reliable: false,
        },
        parser_name: parser.name,
        parser_version: parser.version,
        parser_config_sha256: parser.config_sha256,
    })
}

fn limit_error() -> AppError {
    AppError::new(
        ErrorType::Policy,
        "SOURCE_PARSE_LIMIT",
        "The source exceeds the configured parser limits.",
    )
    .with_hint("Reduce source size or adjust the bounded parser limits.")
}
