use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    canonical::schema::Schema,
    domain::{ChunkId, EvidenceStance, Locator, SourceRevision},
    ingest::{
        cache::{ArtifactReference, ModelIdentity, StageKey},
        chunking::ChunkOptions,
    },
    model::{GenerationOptions, OutputDiagnostic, UsageSummary},
};

pub struct ExtractionRequest<'a> {
    pub revision: &'a SourceRevision,
    pub bytes: &'a [u8],
    pub schema: &'a Schema,
    pub purpose: Option<&'a str>,
    pub provider_profile: &'a str,
    pub model: &'a ModelIdentity,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionMode {
    #[default]
    Full,
    Entities,
    Assertions,
    Refresh,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ExtractionOptions {
    pub mode: ExtractionMode,
    pub focus: Vec<String>,
    pub language: String,
    pub chunking: ChunkOptions,
    pub generation: GenerationOptions,
}

impl Default for ExtractionOptions {
    fn default() -> Self {
        Self {
            mode: ExtractionMode::Full,
            focus: vec![],
            language: "auto".into(),
            chunking: ChunkOptions::default(),
            generation: GenerationOptions {
                schema_name: "compiler_candidates_v1".into(),
                ..Default::default()
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CandidateMention {
    #[schemars(length(min = 1, max = 1000))]
    pub quote: String,
    pub locator: Locator,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CandidateEvidence {
    pub stance: EvidenceStance,
    #[schemars(length(min = 1, max = 1000))]
    pub quote: String,
    pub locator: Locator,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CandidateEntity {
    #[schemars(length(min = 1, max = 64))]
    pub temp_id: String,
    #[serde(rename = "type")]
    #[schemars(length(min = 1, max = 64))]
    pub node_type: String,
    #[schemars(length(min = 1, max = 256))]
    pub canonical_name: String,
    #[schemars(length(max = 32))]
    pub aliases: Vec<String>,
    #[schemars(length(max = 4096))]
    pub description: String,
    #[schemars(length(min = 1, max = 32))]
    pub mentions: Vec<CandidateMention>,
    #[schemars(range(min = 0.0, max = 1.0))]
    pub confidence: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AssertionBasis {
    Stated,
    Inferred,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum QualifierValue {
    Text(#[schemars(length(max = 1024))] String),
    Number(f64),
    Boolean(bool),
    TextList(#[schemars(length(max = 32))] Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CandidateClaim {
    #[schemars(length(min = 1, max = 64))]
    pub temp_id: String,
    #[schemars(length(min = 1, max = 64))]
    pub subject_ref: String,
    #[schemars(length(min = 1, max = 4096))]
    pub statement: String,
    pub basis: AssertionBasis,
    pub qualifiers: BTreeMap<String, QualifierValue>,
    #[schemars(length(min = 1, max = 32))]
    pub evidence: Vec<CandidateEvidence>,
    #[schemars(range(min = 0.0, max = 1.0))]
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CandidateRelation {
    #[schemars(length(min = 1, max = 64))]
    pub temp_id: String,
    #[schemars(length(min = 1, max = 64))]
    pub source_ref: String,
    #[schemars(length(min = 1, max = 64))]
    pub predicate: String,
    #[schemars(length(min = 1, max = 64))]
    pub target_ref: String,
    pub basis: AssertionBasis,
    pub qualifiers: BTreeMap<String, QualifierValue>,
    #[schemars(length(min = 1, max = 32))]
    pub evidence: Vec<CandidateEvidence>,
    #[schemars(range(min = 0.0, max = 1.0))]
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CandidateWarning {
    #[schemars(length(min = 1, max = 64))]
    pub code: String,
    #[schemars(length(min = 1, max = 1024))]
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Candidates {
    #[schemars(length(max = 4096))]
    pub entities: Vec<CandidateEntity>,
    #[schemars(length(max = 4096))]
    pub claims: Vec<CandidateClaim>,
    #[schemars(length(max = 4096))]
    pub relations: Vec<CandidateRelation>,
    #[schemars(length(max = 1024))]
    pub warnings: Vec<CandidateWarning>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct LocalCandidates {
    #[schemars(length(max = 128))]
    pub entities: Vec<CandidateEntity>,
    #[schemars(length(max = 256))]
    pub claims: Vec<CandidateClaim>,
    #[schemars(length(max = 256))]
    pub relations: Vec<CandidateRelation>,
    #[schemars(length(max = 128))]
    pub warnings: Vec<CandidateWarning>,
}

impl From<LocalCandidates> for Candidates {
    fn from(value: LocalCandidates) -> Self {
        Self {
            entities: value.entities,
            claims: value.claims,
            relations: value.relations,
            warnings: value.warnings,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CandidateIdentity {
    pub prompt_id: String,
    pub provider_profile: String,
    pub chunk_id: ChunkId,
    pub sampling: CandidateSampling,
    pub key: StageKey,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CandidateSampling {
    pub schema_name: String,
    pub max_output_tokens: u32,
    pub temperature: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CandidateArtifact {
    pub version: u32,
    pub identity: CandidateIdentity,
    pub candidates: Candidates,
    pub usage: UsageSummary,
    pub diagnostics: Vec<OutputDiagnostic>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ChunkExtraction {
    pub identity: CandidateIdentity,
    pub artifact: ArtifactReference,
    pub cache_hit: bool,
    /// Original generation usage, including when this artifact is reused.
    pub usage: UsageSummary,
    pub diagnostics: Vec<OutputDiagnostic>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ExtractionDiagnostic {
    pub chunk_id: ChunkId,
    pub attempt: u32,
    pub reason: String,
    pub response_sha256: Option<String>,
    pub candidate_sha256: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ExtractionReport {
    pub candidates: Candidates,
    pub candidates_sha256: String,
    pub requires_review: bool,
    pub parsed: ArtifactReference,
    pub chunked: ArtifactReference,
    pub chunks: Vec<ChunkExtraction>,
    /// Only new requests made by this invocation consume its budget.
    pub usage: UsageSummary,
    pub diagnostics: Vec<ExtractionDiagnostic>,
    pub warnings: Vec<String>,
}
