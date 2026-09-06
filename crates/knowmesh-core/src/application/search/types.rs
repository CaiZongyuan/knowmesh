use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::ranking::{ChannelSummary, Explanation};
use crate::{
    application::lexical::{LexicalCandidates, LexicalHit, QuerySyntax, RecordType},
    domain::{
        DependencySnapshot, EvidenceId, NodeId, SourceId, WorkspaceId,
        freshness::{FreshnessContext, FreshnessReport},
    },
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SearchInput {
    pub query: String,
    pub query_syntax: QuerySyntax,
    pub record_types: Vec<RecordType>,
    pub node_types: Vec<String>,
    pub source_ids: Vec<SourceId>,
    pub tags: Vec<String>,
    pub statuses: Vec<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    pub include_graph_paths: bool,
    pub explain: bool,
    pub no_sync: bool,
}

impl Default for SearchInput {
    fn default() -> Self {
        Self {
            query: String::new(),
            query_syntax: QuerySyntax::Literal,
            record_types: vec![
                RecordType::Node,
                RecordType::Claim,
                RecordType::Source,
                RecordType::Synthesis,
            ],
            node_types: vec![],
            source_ids: vec![],
            tags: vec![],
            statuses: vec!["active".into()],
            limit: None,
            cursor: None,
            include_graph_paths: false,
            explain: false,
            no_sync: false,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchHit {
    pub unit_id: String,
    pub record_type: RecordType,
    pub record_id: String,
    pub title: String,
    pub aliases: Vec<String>,
    pub preview: String,
    pub score: f64,
    pub exact_id_tier: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explain: Option<Explanation>,
    #[serde(flatten)]
    pub freshness: Option<FreshnessReport>,
}

#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct SearchGroups {
    pub knowledge: Vec<SearchHit>,
    pub claims: Vec<SearchHit>,
    pub sources: Vec<SearchHit>,
    pub syntheses: Vec<SearchHit>,
    pub chunks: Vec<SearchHit>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchCapabilities {
    pub word_fts: bool,
    pub trigram_fts: bool,
    pub short_text: bool,
    pub vector: bool,
    pub graph_paths: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchQueryText {
    pub original: String,
    pub normalized: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntityMatch {
    ExactId,
    CanonicalName,
    Alias,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ResolvedEntity {
    pub node_id: NodeId,
    pub matched_by: EntityMatch,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchReport {
    pub generation: u64,
    pub index_complete: bool,
    pub query: SearchQueryText,
    pub groups: SearchGroups,
    pub resolved_entities: Vec<ResolvedEntity>,
    pub capabilities: SearchCapabilities,
    pub channels: Vec<ChannelSummary>,
    pub warnings: Vec<String>,
    pub next_cursor: Option<String>,
}

pub enum KnowledgeDependencies {
    Assertion(Vec<EvidenceId>),
    Synthesis {
        evidence_ids: Vec<EvidenceId>,
        snapshot: Option<DependencySnapshot>,
    },
}

pub struct SearchData {
    pub workspace_id: WorkspaceId,
    pub lexical: LexicalCandidates,
    pub exact_candidates: Vec<LexicalHit>,
    pub dependencies: BTreeMap<String, KnowledgeDependencies>,
    pub context: FreshnessContext,
}
