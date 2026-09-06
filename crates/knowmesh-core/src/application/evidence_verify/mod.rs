mod text;
mod types;

use std::ops::Range;

pub use types::{EvidenceInput, VerificationOptions, VerifiedEvidence};

use crate::{
    domain::{Evidence, EvidenceId, Locator, SourceRevision, sha256},
    error::{AppError, AppResult, ErrorType},
    ingest::{ParsedSource, SourceBlock},
};
use text::{CharacterIndex, QuotePattern, normalize};

pub const VERIFIER_VERSION: &str = "1";

/// Reuse one verifier for all candidate quotes from the same immutable parse artifact.
pub struct EvidenceVerifier<'a> {
    revision: &'a SourceRevision,
    parsed: &'a ParsedSource,
    text: CharacterIndex<'a>,
    regions: Vec<Range<usize>>,
    options: VerificationOptions,
}

impl<'a> EvidenceVerifier<'a> {
    pub fn new(
        revision: &'a SourceRevision,
        parsed: &'a ParsedSource,
        options: VerificationOptions,
    ) -> AppResult<Self> {
        if options.repair_window_chars > 10_000
            || !(1..=1_000_000).contains(&options.max_search_chars)
        {
            return Err(error(
                "INVALID_EVIDENCE_OPTIONS",
                "Evidence verification requires a repair radius <= 10000 and a search limit in 1..=1000000.",
            ));
        }
        parsed.validate(revision)?;
        if !parsed.quality.usable_for_compile {
            return Err(AppError::new(
                ErrorType::Policy,
                "SOURCE_NOT_COMPILABLE",
                "Source extraction did not pass its quality gate.",
            ));
        }
        let mut regions: Vec<Range<usize>> = vec![];
        for (index, block) in parsed.blocks.iter().enumerate() {
            if let Some(last) = regions.last_mut()
                && same_scope(&parsed.blocks[last.start], block)
            {
                last.end = index + 1;
            } else {
                regions.push(index..index + 1);
            }
        }
        Ok(Self {
            revision,
            parsed,
            text: CharacterIndex::new(&parsed.normalized_text),
            regions,
            options,
        })
    }

    pub fn verify(&self, input: &EvidenceInput) -> AppResult<VerifiedEvidence> {
        self.validate_input(input)?;
        let quote = normalize(&input.quote);
        let (span, locator_repaired) =
            if let Some((start, end)) = input.locator.char_start.zip(input.locator.char_end) {
                let supplied = start..end;
                let scope = self.offset_scope(&input.locator, &supplied)?;
                self.check_search_size(supplied.len())?;
                if normalize(self.text.slice(supplied.clone())) == quote {
                    (supplied, false)
                } else if self.options.repair_window_chars == 0 {
                    return Err(not_found());
                } else {
                    let window = start
                        .saturating_sub(self.options.repair_window_chars)
                        .max(scope.start)
                        ..end
                            .saturating_add(self.options.repair_window_chars)
                            .min(scope.end);
                    (self.unique_match(&quote, &[window])?, true)
                }
            } else {
                let scopes = self.explicit_scopes(&input.locator)?;
                (self.unique_match(&quote, &scopes)?, true)
            };
        let evidence = Evidence {
            id: EvidenceId::new(),
            source_revision_id: self.revision.id.clone(),
            stance: input.stance,
            quote_sha256: sha256(quote.as_bytes()),
            quote,
            locator: self.locator(span),
            extraction_method: input.extraction_method,
            confidence: input.confidence,
        };
        evidence.validate()?;
        Ok(VerifiedEvidence {
            evidence,
            locator_repaired,
        })
    }

    fn validate_input(&self, input: &EvidenceInput) -> AppResult<()> {
        if input.source_revision_id != self.revision.id {
            return Err(error(
                "EVIDENCE_REVISION_MISMATCH",
                "Evidence must name the revision used by the verifier.",
            ));
        }
        if input.quote.trim().is_empty() || input.quote.chars().take(1001).count() > 1000 {
            return Err(error(
                "INVALID_EVIDENCE_QUOTE",
                "Evidence quotes must be nonempty and contain at most 1000 Unicode scalar values.",
            ));
        }
        if !input.confidence.is_finite() || !(0.0..=1.0).contains(&input.confidence) {
            return Err(error(
                "INVALID_CONFIDENCE",
                "Confidence must be finite and in 0..=1.",
            ));
        }
        let locator = &input.locator;
        if locator.page == Some(0)
            || locator.paragraph == Some(0)
            || locator.section_path.len() > 64
            || locator
                .section_path
                .iter()
                .any(|s| s.trim().is_empty() || s.len() > 2048)
            || locator.char_start.is_some() != locator.char_end.is_some()
        {
            return Err(error(
                "INVALID_EVIDENCE_LOCATOR",
                "Evidence locator fields are invalid.",
            ));
        }
        if let Some((start, end)) = locator.char_start.zip(locator.char_end)
            && (start >= end || end > self.text.len)
        {
            return Err(error(
                "EVIDENCE_LOCATOR_OUT_OF_BOUNDS",
                "Evidence character offsets must identify a nonempty span within normalized text.",
            ));
        }
        Ok(())
    }

    fn offset_scope(&self, locator: &Locator, span: &Range<usize>) -> AppResult<Range<usize>> {
        let blocks = &self.parsed.blocks;
        let index = blocks.partition_point(|block| block.char_end <= span.start);
        let anchor = blocks.get(index).ok_or_else(scope_mismatch)?;
        if anchor.char_start > span.start || !matches_locator(anchor, locator) {
            return Err(scope_mismatch());
        }
        let region_index = self.regions.partition_point(|range| range.end <= index);
        let mut region = self.regions[region_index].clone();
        if locator.paragraph.is_some() {
            region.start = index;
            region.end = index + 1;
            let containing = &self.regions[region_index];
            while region.start > containing.start
                && blocks[region.start - 1].paragraph == locator.paragraph
            {
                region.start -= 1;
            }
            while region.end < containing.end && blocks[region.end].paragraph == locator.paragraph {
                region.end += 1;
            }
        }
        let scope = blocks[region.start].char_start..blocks[region.end - 1].char_end;
        if span.end > scope.end {
            return Err(scope_mismatch());
        }
        Ok(scope)
    }

    fn explicit_scopes(&self, locator: &Locator) -> AppResult<Vec<Range<usize>>> {
        if locator.page.is_none() && locator.paragraph.is_none() && locator.section_path.is_empty()
        {
            return Err(error(
                "EVIDENCE_LOCATOR_REQUIRED",
                "Offset-free verification requires an explicit page, section, or paragraph.",
            ));
        }
        let mut scopes: Vec<Range<usize>> = vec![];
        let mut previous: Option<&SourceBlock> = None;
        for block in &self.parsed.blocks {
            if !matches_locator(block, locator) {
                previous = None;
                continue;
            }
            if previous.is_some_and(|prev| same_scope(prev, block)) {
                scopes.last_mut().expect("a previous block has a scope").end = block.char_end;
            } else {
                scopes.push(block.char_start..block.char_end);
            }
            previous = Some(block);
        }
        if scopes.is_empty() {
            return Err(scope_mismatch());
        }
        Ok(scopes)
    }

    fn unique_match(&self, quote: &str, scopes: &[Range<usize>]) -> AppResult<Range<usize>> {
        let total = scopes
            .iter()
            .fold(0usize, |sum, range| sum.saturating_add(range.len()));
        self.check_search_size(total)?;
        let pattern = QuotePattern::new(quote);
        let mut found = None;
        for scope in scopes {
            for span in pattern.find(self.text.slice(scope.clone()), scope.start) {
                if found.replace(span).is_some() {
                    return Err(error(
                        "EVIDENCE_QUOTE_AMBIGUOUS",
                        "Multiple quote occurrences match the permitted locator scope.",
                    ));
                }
            }
        }
        found.ok_or_else(not_found)
    }

    fn check_search_size(&self, size: usize) -> AppResult<()> {
        if size > self.options.max_search_chars {
            return Err(error(
                "EVIDENCE_SEARCH_LIMIT",
                "Evidence scope exceeds the character budget; supply narrower coordinates.",
            ));
        }
        Ok(())
    }

    fn locator(&self, span: Range<usize>) -> Locator {
        let blocks = &self.parsed.blocks;
        let first = blocks.partition_point(|block| block.char_end <= span.start);
        let end = blocks.partition_point(|block| block.char_start < span.end);
        let anchor = &blocks[first];
        let paragraph = blocks[first..end]
            .iter()
            .all(|block| block.paragraph == anchor.paragraph)
            .then_some(anchor.paragraph)
            .flatten();
        Locator {
            page: anchor.page,
            section_path: anchor.section_path.clone(),
            paragraph,
            char_start: Some(span.start),
            char_end: Some(span.end),
        }
    }
}

fn matches_locator(block: &SourceBlock, locator: &Locator) -> bool {
    (locator.page.is_none() || locator.page == block.page)
        && (locator.section_path.is_empty() || locator.section_path == block.section_path)
        && (locator.paragraph.is_none() || locator.paragraph == block.paragraph)
}

fn same_scope(left: &SourceBlock, right: &SourceBlock) -> bool {
    left.page == right.page && left.section_path == right.section_path
}

fn error(code: &str, message: &str) -> AppError {
    AppError::new(ErrorType::Validation, code, message)
}

fn not_found() -> AppError {
    error(
        "EVIDENCE_QUOTE_NOT_FOUND",
        "The quote does not occur within its permitted locator scope.",
    )
}

fn scope_mismatch() -> AppError {
    error(
        "EVIDENCE_SCOPE_MISMATCH",
        "The locator does not match the source page, section, or paragraph.",
    )
}
