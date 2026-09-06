use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    domain::SourceId,
    error::{AppError, AppResult, ErrorType},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QuerySyntax {
    #[default]
    Literal,
    Advanced,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RecordType {
    Node,
    Claim,
    Source,
    Synthesis,
    Chunk,
}

impl std::str::FromStr for RecordType {
    type Err = AppError;
    fn from_str(value: &str) -> AppResult<Self> {
        match value {
            "node" => Ok(Self::Node),
            "claim" => Ok(Self::Claim),
            "source" => Ok(Self::Source),
            "synthesis" => Ok(Self::Synthesis),
            "chunk" => Ok(Self::Chunk),
            _ => Err(invalid(
                "INVALID_RECORD_TYPE",
                "Choose node, claim, source, synthesis, or chunk.",
                "record_types",
            )),
        }
    }
}

impl std::str::FromStr for QuerySyntax {
    type Err = AppError;
    fn from_str(value: &str) -> AppResult<Self> {
        match value {
            "literal" => Ok(Self::Literal),
            "advanced" => Ok(Self::Advanced),
            _ => Err(invalid(
                "INVALID_QUERY_SYNTAX",
                "Choose literal or advanced query syntax.",
                "query_syntax",
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct LexicalQuery {
    pub query: String,
    pub query_syntax: QuerySyntax,
    pub record_types: Vec<RecordType>,
    pub node_types: Vec<String>,
    pub source_ids: Vec<SourceId>,
    pub tags: Vec<String>,
    pub statuses: Vec<String>,
    pub candidate_limit: u32,
    pub timeout_ms: u64,
}

impl Default for LexicalQuery {
    fn default() -> Self {
        Self {
            query: String::new(),
            query_syntax: QuerySyntax::Literal,
            record_types: vec![],
            node_types: vec![],
            source_ids: vec![],
            tags: vec![],
            statuses: vec!["active".into()],
            candidate_limit: 100,
            timeout_ms: 200,
        }
    }
}

impl LexicalQuery {
    pub fn validate(&self) -> AppResult<()> {
        if self.query.trim().is_empty()
            || self.query.len() > 4096
            || self.query.contains('\0')
            || self.query.split_whitespace().count() > 64
        {
            return Err(invalid(
                "INVALID_SEARCH_QUERY",
                "Search requires nonempty text of at most 4096 UTF-8 bytes and 64 whitespace terms, without NUL.",
                "query",
            ));
        }
        if !(1..=500).contains(&self.candidate_limit) {
            return Err(invalid(
                "INVALID_CANDIDATE_LIMIT",
                "Each search channel requires a candidate limit between 1 and 500.",
                "candidate_limit",
            ));
        }
        if !(1..=5000).contains(&self.timeout_ms) {
            return Err(invalid(
                "INVALID_SEARCH_TIMEOUT",
                "The lexical execution timeout must be between 1 and 5000 milliseconds.",
                "timeout_ms",
            ));
        }
        if self.record_types.len() > 5
            || self.node_types.len() > 64
            || self.source_ids.len() > 64
            || self.tags.len() > 64
            || self
                .node_types
                .iter()
                .any(|value| value.is_empty() || value.len() > 64)
            || self
                .tags
                .iter()
                .any(|value| value.is_empty() || value.len() > 256)
            || self.statuses.len() > 16
            || self
                .statuses
                .iter()
                .any(|status| status.is_empty() || status.len() > 64)
        {
            return Err(invalid(
                "INVALID_SEARCH_FILTER",
                "Search filters exceed their supported bounds.",
                "filters",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LexicalChannel {
    Word,
    Trigram,
    ShortText,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LexicalHit {
    pub unit_id: String,
    pub record_type: RecordType,
    pub record_id: String,
    pub title: String,
    pub aliases: Vec<String>,
    pub preview: String,
    pub rank: u32,
    pub bm25: Option<f64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ChannelCandidates {
    pub channel: LexicalChannel,
    pub hits: Vec<LexicalHit>,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct LexicalCandidates {
    pub generation: u64,
    pub snapshot_sha256: String,
    pub channels: Vec<ChannelCandidates>,
}

fn invalid(code: &str, message: &str, param: &str) -> AppError {
    AppError::new(ErrorType::Validation, code, message).with_param(param)
}
