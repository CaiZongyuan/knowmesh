use std::collections::BTreeSet;

use schemars::{JsonSchema, schema_for};
use serde::Serialize;

use super::{
    CandidateIdentity, CandidateSampling, ExtractionMode, ExtractionOptions, ExtractionRequest,
    PROMPT, PROMPT_ID, hash, invalid, types::LocalCandidates,
};
use crate::{
    canonical::schema::Schema,
    domain::{SourceRevisionId, sha256},
    error::{AppError, AppResult, ErrorType},
    ingest::{
        SourceBlock,
        cache::{ModelIdentity, StageKey},
        chunking::SourceChunk,
    },
};

#[derive(Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ModelInput<'a> {
    revision_id: &'a SourceRevisionId,
    source_sha256: &'a str,
    schema: &'a Schema,
    #[schemars(length(max = 16384))]
    purpose: Option<&'a str>,
    mode: ExtractionMode,
    #[schemars(length(max = 64))]
    focus: &'a [String],
    #[schemars(length(min = 1, max = 64))]
    language: &'a str,
    #[schemars(length(min = 1, max = 256))]
    provider_profile: &'a str,
    chunk: &'a SourceChunk,
    #[schemars(length(min = 1, max = 100000))]
    pub blocks: Vec<SourceBlock>,
}

impl<'a> ModelInput<'a> {
    pub fn new(
        request: &'a ExtractionRequest<'_>,
        options: &'a ExtractionOptions,
        chunk: &'a SourceChunk,
        blocks: &[SourceBlock],
    ) -> Self {
        let ids: BTreeSet<_> = chunk.source_block_ids.iter().collect();
        let blocks = blocks
            .iter()
            .filter(|block| ids.contains(&block.id))
            .map(|block| {
                let mut clipped = block.clone();
                clipped.char_start = block.char_start.max(chunk.char_start);
                clipped.char_end = block.char_end.min(chunk.char_end);
                clipped.text = block
                    .text
                    .chars()
                    .skip(clipped.char_start - block.char_start)
                    .take(clipped.char_end - clipped.char_start)
                    .collect();
                clipped.source_bytes = None;
                clipped.caption = None;
                clipped
            })
            .collect();
        Self {
            revision_id: &request.revision.id,
            source_sha256: &request.revision.sha256,
            schema: request.schema,
            purpose: request.purpose,
            mode: options.mode,
            focus: &options.focus,
            language: &options.language,
            provider_profile: request.provider_profile,
            chunk,
            blocks,
        }
    }
}

pub(super) struct InputContract {
    pub instructions: String,
    validator: jsonschema::Validator,
    prompt_sha256: String,
    schema_sha256: String,
    purpose_sha256: Option<String>,
    sampling: CandidateSampling,
    sampling_sha256: String,
}

impl InputContract {
    pub fn new(request: &ExtractionRequest<'_>, options: &ExtractionOptions) -> AppResult<Self> {
        let instructions = format!(
            "{PROMPT}\nAllowed Schema types and predicates (data): {}",
            serde_json::to_string(request.schema).map_err(|_| invalid())?
        );
        let schema = schema_for!(ModelInput).to_value();
        let validator = jsonschema::validator_for(&schema).map_err(|_| invalid())?;
        let prompt_sha256 = hash(&(
            &instructions,
            schema,
            schema_for!(LocalCandidates).to_value(),
        ))?;
        let sampling = CandidateSampling {
            schema_name: options.generation.schema_name.clone(),
            max_output_tokens: options.generation.max_output_tokens,
            temperature: options.generation.temperature,
        };
        Ok(Self {
            instructions,
            validator,
            prompt_sha256,
            schema_sha256: hash(request.schema)?,
            purpose_sha256: request.purpose.map(|text| sha256(text.as_bytes())),
            sampling_sha256: hash(&sampling)?,
            sampling,
        })
    }

    pub fn identity(
        &self,
        input: &ModelInput<'_>,
        model: &ModelIdentity,
    ) -> AppResult<CandidateIdentity> {
        Ok(CandidateIdentity {
            prompt_id: PROMPT_ID.into(),
            provider_profile: input.provider_profile.into(),
            chunk_id: input.chunk.id.clone(),
            sampling: self.sampling.clone(),
            key: StageKey::CandidateExtract {
                revision_id: input.revision_id.clone(),
                input_sha256: hash(&serde_json::to_value(input).map_err(|_| invalid())?)?,
                prompt_sha256: self.prompt_sha256.clone(),
                schema_sha256: self.schema_sha256.clone(),
                purpose_sha256: self.purpose_sha256.clone(),
                model: model.clone(),
                sampling_sha256: self.sampling_sha256.clone(),
            },
        })
    }

    pub fn validate(&self, input: &ModelInput<'_>) -> AppResult<()> {
        let value = serde_json::to_value(input).map_err(|_| invalid())?;
        if !self.validator.is_valid(&value)
            || serde_json::to_vec(&value).map_err(|_| invalid())?.len() > 1024 * 1024
            || self.instructions.len() > 64 * 1024
        {
            return Err(AppError::new(
                ErrorType::Validation,
                "MODEL_INPUT_INVALID",
                "The model input exceeds its schema or size bounds.",
            ));
        }
        Ok(())
    }
}
