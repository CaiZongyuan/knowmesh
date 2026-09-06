use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::NodeId;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EntityInput {
    pub name: String,
    pub node_type: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub properties: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ResolutionOptions {
    pub candidate_limit: usize,
    pub max_catalog_nodes: usize,
}

impl Default for ResolutionOptions {
    fn default() -> Self {
        Self {
            candidate_limit: 20,
            max_catalog_nodes: 100_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionDecision {
    Existing,
    New,
    Ambiguous,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResolutionCandidate {
    pub node_id: NodeId,
    pub name: String,
    pub node_type: String,
    pub matched_by: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResolutionReport {
    pub decision: ResolutionDecision,
    pub selected_node_id: Option<NodeId>,
    pub automatic: bool,
    pub candidates: Vec<ResolutionCandidate>,
    pub total_candidates: usize,
    pub candidates_truncated: bool,
    pub catalog_sha256: String,
    pub input_sha256: String,
    pub options_sha256: String,
    pub warnings: Vec<String>,
}
