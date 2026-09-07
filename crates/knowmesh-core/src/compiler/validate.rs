use std::collections::{BTreeMap, BTreeSet};

use super::{
    CandidateArtifact, Candidates, ExtractionMode, ExtractionOptions, ExtractionRequest,
    QualifierValue,
};
use crate::{
    canonical::schema::Schema,
    domain::Locator,
    error::{AppError, AppResult, ErrorType},
    ingest::{SourceBlock, chunking::SourceChunk},
};

pub(super) struct CandidateValidator<'a> {
    schema: &'a Schema,
    mode: ExtractionMode,
    output_schema: jsonschema::Validator,
}

pub(super) fn context(
    request: &ExtractionRequest<'_>,
    options: &ExtractionOptions,
) -> AppResult<()> {
    if options.language.trim().is_empty()
        || options.language.len() > 64
        || options.language.chars().any(char::is_control)
        || options.focus.len() > 64
        || options
            .focus
            .iter()
            .any(|name| !request.schema.node_types.contains_key(name))
        || request.provider_profile.trim().is_empty()
        || request.provider_profile.len() > 256
        || request.purpose.is_some_and(|text| text.len() > 16384)
    {
        return Err(AppError::new(
            ErrorType::Validation,
            "COMPILER_INPUT_INVALID",
            "The extraction context exceeds its supported bounds.",
        ));
    }
    // Cache hits bypass generate, so they also need the existing model option bounds.
    let generation = &options.generation;
    if generation.schema_name.is_empty()
        || generation.schema_name.len() > 64
        || !generation
            .schema_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        || !(1..=1_000_000).contains(&generation.max_output_tokens)
        || generation.max_total_tokens == 0
        || !(1..=9).contains(&generation.max_calls)
        || generation.max_repairs > 2
        || generation.max_retries > 2
        || !(1..=3_600_000).contains(&generation.timeout_ms)
        || generation.retry_backoff_ms > 5000
        || generation
            .temperature
            .is_some_and(|value| !value.is_finite() || !(0.0..=2.0).contains(&value))
    {
        return Err(AppError::new(
            ErrorType::Validation,
            "INVALID_MODEL_OPTIONS",
            "Model request limits or sampling options are invalid.",
        ));
    }
    Ok(())
}

pub(super) fn accounting(artifact: &CandidateArtifact) -> AppResult<()> {
    let usage = &artifact.usage;
    if usage.requests == 0
        || usage.requests > 9
        || usage.repairs > 2
        || usage.retries > 2 * (usage.repairs + 1)
        || usage.requests != 1 + usage.repairs + usage.retries
        || usage.input_tokens.checked_add(usage.output_tokens) != Some(usage.total_tokens)
        || artifact.diagnostics.len() != usage.repairs as usize
        || artifact
            .diagnostics
            .iter()
            .enumerate()
            .any(|(index, diagnostic)| {
                diagnostic.attempt != index as u32 + 1
                    || !["invalid_json", "schema_mismatch", "decode_failed"]
                        .contains(&diagnostic.reason.as_str())
                    || !crate::domain::valid_sha256(&diagnostic.response_sha256)
            })
    {
        return Err(super::invalid());
    }
    Ok(())
}

impl<'a> CandidateValidator<'a> {
    pub fn new(schema: &'a Schema, mode: ExtractionMode) -> AppResult<Self> {
        Ok(Self {
            schema,
            mode,
            output_schema: jsonschema::validator_for(
                &schemars::schema_for!(super::types::LocalCandidates).to_value(),
            )
            .map_err(|_| rejected("CANDIDATE_SCHEMA_INVALID"))?,
        })
    }

    pub fn local(
        &self,
        value: &Candidates,
        chunk: &SourceChunk,
        blocks: &[SourceBlock],
    ) -> AppResult<()> {
        if !self.output_schema.is_valid(
            &serde_json::to_value(value).map_err(|_| rejected("CANDIDATE_SCHEMA_INVALID"))?,
        ) {
            return Err(rejected("CANDIDATE_SCHEMA_INVALID"));
        }
        if self.mode == ExtractionMode::Entities
            && (!value.claims.is_empty() || !value.relations.is_empty())
        {
            return Err(rejected("CANDIDATE_MODE_INVALID"));
        }
        let mut types = BTreeMap::new();
        for entity in &value.entities {
            local_id(&entity.temp_id, "ent_")?;
            if !self.schema.node_types.contains_key(&entity.node_type) {
                return Err(rejected("CANDIDATE_TYPE_INVALID"));
            }
            types.insert(&entity.temp_id, &entity.node_type);
            text(&entity.canonical_name, 256)?;
            for alias in &entity.aliases {
                text(alias, 256)?;
            }
            for mention in &entity.mentions {
                locator(&mention.quote, &mention.locator, chunk, blocks)?;
            }
        }
        references(value)?;
        for claim in &value.claims {
            local_id(&claim.temp_id, "claim_")?;
            text(&claim.statement, 4096)?;
            qualifiers(&claim.qualifiers)?;
            for evidence in &claim.evidence {
                locator(&evidence.quote, &evidence.locator, chunk, blocks)?;
            }
        }
        for relation in &value.relations {
            local_id(&relation.temp_id, "relation_")?;
            self.schema
                .validate_relation(
                    &relation.predicate,
                    types[&relation.source_ref],
                    types[&relation.target_ref],
                    !relation.evidence.is_empty(),
                )
                .map_err(|_| rejected("CANDIDATE_PREDICATE_INVALID"))?;
            qualifiers(&relation.qualifiers)?;
            for evidence in &relation.evidence {
                locator(&evidence.quote, &evidence.locator, chunk, blocks)?;
            }
        }
        for warning in &value.warnings {
            text(&warning.code, 64)?;
            text(&warning.message, 1024)?;
        }
        Ok(())
    }
}

pub(super) fn references(value: &Candidates) -> AppResult<()> {
    let mut ids = BTreeSet::new();
    for id in value
        .entities
        .iter()
        .map(|value| &value.temp_id)
        .chain(value.claims.iter().map(|value| &value.temp_id))
        .chain(value.relations.iter().map(|value| &value.temp_id))
    {
        if !ids.insert(id) {
            return Err(rejected("CANDIDATE_ID_INVALID"));
        }
    }
    let entities: BTreeSet<_> = value
        .entities
        .iter()
        .map(|entity| &entity.temp_id)
        .collect();
    if value
        .claims
        .iter()
        .any(|claim| !entities.contains(&claim.subject_ref))
        || value.relations.iter().any(|relation| {
            !entities.contains(&relation.source_ref) || !entities.contains(&relation.target_ref)
        })
    {
        return Err(rejected("CANDIDATE_REFERENCE_INVALID"));
    }
    Ok(())
}

fn local_id(value: &str, prefix: &str) -> AppResult<()> {
    if value.len() > 64
        || !value.strip_prefix(prefix).is_some_and(|suffix| {
            !suffix.is_empty()
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
    {
        return Err(rejected("CANDIDATE_ID_INVALID"));
    }
    Ok(())
}

fn text(value: &str, max: usize) -> AppResult<()> {
    if value.trim().is_empty() || value.len() > max {
        return Err(rejected("CANDIDATE_TEXT_INVALID"));
    }
    Ok(())
}

fn qualifiers(values: &BTreeMap<String, QualifierValue>) -> AppResult<()> {
    let valid = values.len() <= 32
        && values.iter().all(|(key, value)| {
            !key.trim().is_empty()
                && key.len() <= 64
                && !key.chars().any(char::is_control)
                && match value {
                    QualifierValue::Text(value) => value.len() <= 1024,
                    QualifierValue::Number(value) => value.is_finite(),
                    QualifierValue::Boolean(_) => true,
                    QualifierValue::TextList(values) => {
                        values.len() <= 32 && values.iter().all(|value| value.len() <= 1024)
                    }
                }
        });
    if !valid {
        return Err(rejected("CANDIDATE_QUALIFIERS_INVALID"));
    }
    Ok(())
}

// This checks supplied scope only. Quote matching and Evidence identity belong to verification.
fn locator(
    quote: &str,
    locator: &Locator,
    chunk: &SourceChunk,
    blocks: &[SourceBlock],
) -> AppResult<()> {
    text(quote, 4000)?;
    let (Some(start), Some(end)) = (locator.char_start, locator.char_end) else {
        return Err(rejected("CANDIDATE_LOCATOR_INVALID"));
    };
    if start >= end
        || start < chunk.char_start
        || end > chunk.char_end
        || locator.page == Some(0)
        || locator.paragraph == Some(0)
        || locator.section_path.len() > 64
        || locator.section_path.iter().any(|part| part.len() > 2048)
    {
        return Err(rejected("CANDIDATE_LOCATOR_INVALID"));
    }
    let overlapping: Vec<_> = blocks
        .iter()
        .filter(|block| block.char_start < end && block.char_end > start)
        .collect();
    if overlapping.is_empty()
        || overlapping.iter().any(|block| {
            locator.page.is_some_and(|page| block.page != Some(page))
                || locator
                    .paragraph
                    .is_some_and(|paragraph| block.paragraph != Some(paragraph))
                || (!locator.section_path.is_empty()
                    && !block.section_path.starts_with(&locator.section_path))
        })
    {
        return Err(rejected("CANDIDATE_LOCATOR_INVALID"));
    }
    Ok(())
}

pub(super) fn rejected(code: &str) -> AppError {
    AppError::new(
        ErrorType::Model,
        code,
        "The candidate output violates the extraction constraints.",
    )
}
