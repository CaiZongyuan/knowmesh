use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    application::assertion_dedup::AssertionChange,
    domain::{Claim, ClaimId, ConflictGroup, ConflictGroupId},
    model::{OutputDiagnostic, UsageSummary},
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimPair {
    pub left_id: ClaimId,
    pub right_id: ClaimId,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PairSelection {
    pub focus_ids: Vec<ClaimId>,
    pub limit: usize,
    pub cursor: Option<String>,
}

impl Default for PairSelection {
    fn default() -> Self {
        Self {
            focus_ids: vec![],
            limit: 32,
            cursor: None,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PairPage {
    pub pairs: Vec<ClaimPair>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonVerdict {
    Independent,
    PossibleDuplicate,
    Conflicting,
    Undetermined,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimComparison {
    #[serde(flatten)]
    pub pair: ClaimPair,
    pub left_semantic_sha256: String,
    pub right_semantic_sha256: String,
    pub verdict: ComparisonVerdict,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComparisonReport {
    pub version: u32,
    pub context_sha256: String,
    pub input_sha256: String,
    pub prompt_sha256: String,
    pub comparisons: Vec<ClaimComparison>,
    pub usage: UsageSummary,
    pub diagnostics: Vec<OutputDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BlockedConflict {
    pub pair: ClaimPair,
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConflictPlan {
    pub context_sha256: String,
    pub report_sha256: String,
    pub groups: Vec<ConflictGroup>,
    pub claim_changes: Vec<AssertionChange<Claim>>,
    pub possible_duplicates: Vec<ClaimComparison>,
    pub undetermined: Vec<ClaimComparison>,
    pub blocked_conflicts: Vec<BlockedConflict>,
    pub existing_group_ids: Vec<ConflictGroupId>,
    pub requires_review: bool,
}
