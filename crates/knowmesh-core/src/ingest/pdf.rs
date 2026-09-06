use lopdf::{Document, LoadOptions};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    ParseLimits, ParseStatus, ParseWarning, ParsedMetadata, ParsedSource, ParserDescriptor,
    assemble, builder::Builder, limit_error, plain,
};
use crate::{
    domain::{SourceRevision, sha256},
    error::{AppError, AppResult, ErrorType},
    ports::SourceParser,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PdfOptions {
    pub max_pages: u32,
    pub max_decompressed_bytes: usize,
    pub max_text_bytes: usize,
    pub min_visible_characters: usize,
    pub max_suspicious_ratio: f64,
    pub min_text_page_ratio: f64,
    pub require_page_map: bool,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            max_pages: 10_000,
            max_decompressed_bytes: 64 * 1024 * 1024,
            max_text_bytes: 100 * 1024 * 1024,
            min_visible_characters: 40,
            max_suspicious_ratio: 0.02,
            min_text_page_ratio: 0.5,
            require_page_map: true,
        }
    }
}

#[derive(Debug, Default)]
pub struct PdfParser {
    limits: ParseLimits,
    options: PdfOptions,
}

impl PdfParser {
    pub fn new(limits: ParseLimits, options: PdfOptions) -> AppResult<Self> {
        super::TextParser::new(limits)?;
        if options.max_pages == 0
            || options.max_decompressed_bytes == 0
            || options.max_text_bytes == 0
            || options.min_visible_characters == 0
            || !options.max_suspicious_ratio.is_finite()
            || !(0.0..=1.0).contains(&options.max_suspicious_ratio)
            || !options.min_text_page_ratio.is_finite()
            || !(0.0..=1.0).contains(&options.min_text_page_ratio)
        {
            return Err(AppError::new(
                ErrorType::Validation,
                "INVALID_PDF_OPTIONS",
                "PDF budgets and quality thresholds are invalid.",
            ));
        }
        Ok(Self { limits, options })
    }

    fn empty(
        &self,
        revision: &SourceRevision,
        parser: ParserDescriptor,
        status: ParseStatus,
        warning: &str,
    ) -> AppResult<ParsedSource> {
        let mut parsed = assemble(
            revision,
            parser,
            0,
            ParsedMetadata::default(),
            Builder::new(self.limits.max_blocks),
        )?;
        parsed.status = status;
        parsed.warnings.clear();
        parsed.warnings.push(warning_for(warning));
        parsed.validate(revision)?;
        Ok(parsed)
    }
}

impl SourceParser for PdfParser {
    fn descriptor(&self, mime_type: &str) -> AppResult<ParserDescriptor> {
        if mime_type != "application/pdf" {
            return Err(AppError::new(
                ErrorType::Configuration,
                "SOURCE_PARSER_UNAVAILABLE",
                "This parser accepts PDF revisions only.",
            ));
        }
        let config = serde_json::to_vec(&(&self.limits, &self.options)).map_err(|_| {
            AppError::new(
                ErrorType::Internal,
                "PARSER_CONFIG_INVALID",
                "Could not identify PDF configuration.",
            )
        })?;
        Ok(ParserDescriptor {
            name: "knowmesh-lopdf".into(),
            version: "1".into(),
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
                "The PDF bytes do not match their immutable revision.",
            ));
        }
        let parser = self.descriptor(&revision.mime_type)?;
        if revision.encoding.is_some() {
            return Err(AppError::new(
                ErrorType::Validation,
                "ENCODING_NOT_APPLICABLE",
                "PDF does not accept a text encoding override.",
            ));
        }
        if !bytes.starts_with(b"%PDF-") {
            return Err(invalid_pdf());
        }
        let mut document = match Document::load_mem_with_options(
            bytes,
            LoadOptions::with_max_decompressed_size(self.options.max_decompressed_bytes),
        ) {
            Ok(document) => document,
            Err(lopdf::Error::Decompress(lopdf::DecompressError::MemoryLimitExceeded {
                ..
            })) => return Err(limit_error()),
            Err(
                lopdf::Error::Decryption(_)
                | lopdf::Error::InvalidPassword
                | lopdf::Error::UnsupportedSecurityHandler(_),
            ) => return self.empty(revision, parser, ParseStatus::Blocked, "PDF_ENCRYPTED"),
            Err(_) => return Err(invalid_pdf()),
        };
        if document.is_encrypted() || document.was_encrypted() || document.trailer.has(b"Encrypt") {
            return self.empty(revision, parser, ParseStatus::Blocked, "PDF_ENCRYPTED");
        }
        super::pdf_fonts::prefer_unicode_maps(&mut document)?;
        let pages = document.get_pages();
        if pages.is_empty() {
            return self.empty(
                revision,
                parser,
                ParseStatus::NeedsOcr,
                "PDF_PAGE_MAP_UNRELIABLE",
            );
        }
        if pages.len() > self.options.max_pages as usize {
            return Err(limit_error());
        }
        let declared = document
            .catalog()
            .ok()
            .and_then(|catalog| catalog.get(b"Pages").ok())
            .and_then(|value| value.as_reference().ok())
            .and_then(|id| document.get_dictionary(id).ok())
            .and_then(|pages| pages.get(b"Count").ok())
            .and_then(|count| count.as_i64().ok());
        let reliable = declared == Some(pages.len() as i64);
        let metadata = ParsedMetadata {
            title: document
                .trailer
                .get(b"Info")
                .ok()
                .and_then(|value| value.as_reference().ok())
                .and_then(|id| document.get_dictionary(id).ok())
                .and_then(|info| info.get(b"Title").ok())
                .and_then(|value| lopdf::decode_text_string(value).ok()),
            language: document
                .catalog()
                .ok()
                .and_then(|catalog| catalog.get(b"Lang").ok())
                .and_then(|value| lopdf::decode_text_string(value).ok()),
            ..Default::default()
        };
        let mut builder = Builder::new(self.limits.max_blocks);
        let mut total_text = 0usize;
        let mut text_pages = 0u32;
        let mut extraction_failed = false;
        for (number, page_id) in &pages {
            let text = match super::pdf_fonts::validate(
                &document,
                *page_id,
                self.options.max_decompressed_bytes,
            )
            .and_then(|()| {
                document.extract_text_with_limit(&[*number], self.options.max_decompressed_bytes)
            }) {
                Ok(text) => text,
                Err(lopdf::Error::Decompress(lopdf::DecompressError::MemoryLimitExceeded {
                    ..
                })) => return Err(limit_error()),
                Err(_) => {
                    extraction_failed = true;
                    continue;
                }
            };
            total_text = total_text.checked_add(text.len()).ok_or_else(limit_error)?;
            if total_text > self.options.max_text_bytes {
                return Err(limit_error());
            }
            if text
                .chars()
                .any(|ch| !ch.is_whitespace() && !ch.is_control())
            {
                text_pages += 1;
            }
            let first = builder.blocks.len();
            plain::parse(&text, &mut builder)?;
            for block in &mut builder.blocks[first..] {
                block.source_bytes = None;
                block.page = reliable.then_some(*number);
            }
        }
        let mut parsed = assemble(revision, parser, 0, metadata, builder)?;
        parsed.quality.page_count = Some(pages.len() as u32);
        parsed.quality.text_pages = Some(text_pages);
        parsed.quality.page_map_reliable = reliable;
        parsed
            .warnings
            .retain(|warning| warning.code != "NO_EXTRACTED_TEXT");
        let mut blocked = false;
        for (condition, code, prevents_compile) in [
            (
                parsed.quality.visible_characters == 0,
                "PDF_TEXT_LAYER_MISSING",
                true,
            ),
            (
                parsed.quality.visible_characters > 0
                    && parsed.quality.visible_characters < self.options.min_visible_characters,
                "PDF_TEXT_TOO_SHORT",
                true,
            ),
            (
                text_pages as f64 / (pages.len() as f64) < self.options.min_text_page_ratio,
                "PDF_TEXT_PAGES_INSUFFICIENT",
                true,
            ),
            (
                parsed.quality.suspicious_ratio > self.options.max_suspicious_ratio,
                "PDF_TEXT_GARBLED",
                true,
            ),
            (
                !reliable,
                "PDF_PAGE_MAP_UNRELIABLE",
                self.options.require_page_map,
            ),
            (extraction_failed, "PDF_TEXT_EXTRACTION_FAILED", true),
        ] {
            if condition {
                parsed.warnings.push(warning_for(code));
                blocked |= prevents_compile;
            }
        }
        parsed.status = if blocked {
            ParseStatus::NeedsOcr
        } else {
            ParseStatus::Ready
        };
        parsed.quality.usable_for_compile = !blocked;
        parsed.validate(revision)?;
        Ok(parsed)
    }
}

fn invalid_pdf() -> AppError {
    AppError::new(
        ErrorType::Validation,
        "INVALID_PDF",
        "The source is not a readable PDF document.",
    )
}

fn warning_for(code: &str) -> ParseWarning {
    let hint = match code {
        "PDF_ENCRYPTED" => "Provide an authorized unencrypted copy as a new source revision.",
        "PDF_PAGE_MAP_UNRELIABLE" => {
            "Provide a PDF with a reliable page map before requiring page citations."
        }
        _ => {
            "Inspect the source and provide a reliable text layer or an externally OCR-processed revision."
        }
    };
    ParseWarning {
        code: code.into(),
        hint: hint.into(),
    }
}
