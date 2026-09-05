use std::{collections::BTreeSet, sync::LazyLock};

use regex::Regex;

use super::markdown::{MarkdownFile, code_ranges};
use crate::{
    domain::{EvidenceId, SynthesisMetadata, knowledge_error},
    error::{AppError, AppResult, ErrorType},
};

static CITATIONS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[@([^\]\s]+)\]").expect("constant citation grammar"));

#[derive(Debug)]
pub struct SynthesisDocument {
    pub metadata: SynthesisMetadata,
    pub body: String,
    file: MarkdownFile,
    before_metadata: SynthesisMetadata,
}

impl SynthesisDocument {
    pub fn parse(text: &str) -> AppResult<Self> {
        let file = MarkdownFile::parse(text)?;
        let metadata: SynthesisMetadata = file.metadata()?;
        metadata.validate()?;
        let document = Self {
            before_metadata: metadata.clone(),
            metadata,
            body: file.body().to_owned(),
            file,
        };
        document.citations()?;
        Ok(document)
    }

    pub fn create(metadata: SynthesisMetadata, body: &str) -> AppResult<Self> {
        metadata.validate()?;
        let yaml = serde_yaml::to_string(&metadata).map_err(|_| {
            knowledge_error(
                "SYNTHESIS_ENCODE_FAILED",
                "Could not encode synthesis metadata.",
            )
        })?;
        Self::parse(&format!("---\n{yaml}---\n\n{body}\n"))
    }

    pub fn citations(&self) -> AppResult<BTreeSet<EvidenceId>> {
        let excluded = code_ranges(&self.body);
        let mut ids = BTreeSet::new();
        for capture in CITATIONS.captures_iter(&self.body) {
            let span = capture
                .get(0)
                .ok_or_else(|| knowledge_error("INVALID_CITATION", "Invalid evidence citation."))?;
            if excluded
                .iter()
                .any(|range| range.start <= span.start() && span.start() < range.end)
            {
                continue;
            }
            ids.insert(capture[1].parse().map_err(|_| {
                knowledge_error(
                    "INVALID_CITATION",
                    "Synthesis citations must contain canonical Evidence IDs.",
                )
            })?);
        }
        Ok(ids)
    }

    pub fn validate_citations(&self, available: &BTreeSet<EvidenceId>) -> AppResult<()> {
        let citations = self.citations()?;
        let declared: BTreeSet<_> = self.metadata.evidence_ids.iter().cloned().collect();
        for id in citations.iter().chain(&declared) {
            if !available.contains(id) {
                return Err(AppError::new(
                    ErrorType::NotFound,
                    "EVIDENCE_NOT_FOUND",
                    "Synthesis references evidence absent from the canonical knowledge.",
                ));
            }
        }
        if !citations.is_subset(&declared) {
            return Err(knowledge_error(
                "UNDECLARED_CITATION",
                "Every body citation must appear in synthesis evidence_ids.",
            ));
        }
        Ok(())
    }

    pub fn render(&self) -> AppResult<String> {
        self.metadata.validate()?;
        self.citations()?;
        if self.metadata.id != self.before_metadata.id
            || self.metadata.created_at != self.before_metadata.created_at
        {
            return Err(knowledge_error(
                "SYNTHESIS_IDENTITY_CHANGED",
                "A synthesis writer cannot replace an existing identity or creation timestamp.",
            ));
        }
        let mut replacements = Vec::new();
        if self.metadata != self.before_metadata {
            replacements.push((
                self.file.header.clone(),
                self.file
                    .render_metadata(&self.before_metadata, &self.metadata)?,
            ));
        }
        if self.body != self.file.body() {
            replacements.push((
                self.file.body_start..self.file.original.len(),
                self.body.clone(),
            ));
        }
        self.file.render(replacements)
    }
}
