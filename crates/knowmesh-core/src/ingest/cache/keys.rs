use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    domain::{SourceRevision, SourceRevisionId, TextEncoding, sha256, valid_sha256},
    error::{AppError, AppResult, ErrorType},
    ingest::{
        ParserDescriptor,
        chunking::{ChunkOptions, CounterDescriptor},
    },
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelIdentity {
    pub provider: String,
    pub model: String,
    pub config_sha256: String,
}

impl ModelIdentity {
    fn valid(&self) -> bool {
        bounded(&self.provider) && bounded(&self.model) && valid_sha256(&self.config_sha256)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CacheStage {
    Parse,
    Chunk,
    CandidateExtract,
    Knowledge,
    Embedding,
}

impl CacheStage {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Parse => "parse",
            Self::Chunk => "chunk",
            Self::CandidateExtract => "candidate_extract",
            Self::Knowledge => "knowledge",
            Self::Embedding => "embedding",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "stage", rename_all = "snake_case", deny_unknown_fields)]
pub enum StageKey {
    Parse {
        revision_id: SourceRevisionId,
        blob_sha256: String,
        mime_type: String,
        encoding: Option<TextEncoding>,
        parser: ParserDescriptor,
    },
    Chunk {
        parsed_sha256: String,
        chunker_version: String,
        options: ChunkOptions,
        counter: CounterDescriptor,
    },
    CandidateExtract {
        revision_id: SourceRevisionId,
        input_sha256: String,
        prompt_sha256: String,
        schema_sha256: String,
        purpose_sha256: Option<String>,
        model: ModelIdentity,
        sampling_sha256: String,
    },
    Knowledge {
        operation: String,
        candidate_sha256: String,
        query_sha256: String,
        generation: u64,
        context_sha256: String,
        rules_version: String,
        model: Option<ModelIdentity>,
    },
    Embedding {
        input_sha256: String,
        model: ModelIdentity,
        dimensions: u32,
        preprocessing_version: String,
    },
}

impl StageKey {
    pub fn parse(revision: &SourceRevision, parser: ParserDescriptor) -> Self {
        Self::Parse {
            revision_id: revision.id.clone(),
            blob_sha256: revision.sha256.clone(),
            mime_type: revision.mime_type.clone(),
            encoding: revision.encoding.clone(),
            parser,
        }
    }

    pub fn stage(&self) -> CacheStage {
        match self {
            Self::Parse { .. } => CacheStage::Parse,
            Self::Chunk { .. } => CacheStage::Chunk,
            Self::CandidateExtract { .. } => CacheStage::CandidateExtract,
            Self::Knowledge { .. } => CacheStage::Knowledge,
            Self::Embedding { .. } => CacheStage::Embedding,
        }
    }

    pub fn fingerprint(&self) -> AppResult<String> {
        let valid = match self {
            Self::Parse {
                blob_sha256,
                mime_type,
                parser,
                ..
            } => {
                valid_sha256(blob_sha256)
                    && bounded(mime_type)
                    && bounded(&parser.name)
                    && bounded(&parser.version)
                    && valid_sha256(&parser.config_sha256)
            }
            Self::Chunk {
                parsed_sha256,
                chunker_version,
                options,
                counter,
            } => {
                valid_sha256(parsed_sha256)
                    && bounded(chunker_version)
                    && options.validate().is_ok()
                    && bounded(&counter.name)
                    && bounded(&counter.version)
                    && valid_sha256(&counter.config_sha256)
            }
            Self::CandidateExtract {
                input_sha256,
                prompt_sha256,
                schema_sha256,
                purpose_sha256,
                model,
                sampling_sha256,
                ..
            } => {
                [input_sha256, prompt_sha256, schema_sha256, sampling_sha256]
                    .iter()
                    .all(|hash| valid_sha256(hash))
                    && purpose_sha256.as_deref().is_none_or(valid_sha256)
                    && model.valid()
            }
            Self::Knowledge {
                operation,
                candidate_sha256,
                query_sha256,
                context_sha256,
                rules_version,
                model,
                ..
            } => {
                ["entity-resolution", "dedup", "conflict"].contains(&operation.as_str())
                    && [candidate_sha256, query_sha256, context_sha256]
                        .iter()
                        .all(|hash| valid_sha256(hash))
                    && bounded(rules_version)
                    && model.as_ref().is_none_or(ModelIdentity::valid)
            }
            Self::Embedding {
                input_sha256,
                model,
                dimensions,
                preprocessing_version,
            } => {
                valid_sha256(input_sha256)
                    && model.valid()
                    && (1..=65536).contains(dimensions)
                    && bounded(preprocessing_version)
            }
        };
        if !valid {
            return Err(invalid_key());
        }
        #[derive(Serialize)]
        struct VersionedKey<'a> {
            version: u32,
            key: &'a StageKey,
        }
        let json = serde_json::to_vec(&VersionedKey {
            version: 1,
            key: self,
        })
        .map_err(|_| invalid_key())?;
        Ok(sha256(&json))
    }
}

fn bounded(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 1024
}
fn invalid_key() -> AppError {
    AppError::new(
        ErrorType::Validation,
        "INVALID_CACHE_KEY",
        "The cache key has invalid stage dependencies or configuration.",
    )
}
