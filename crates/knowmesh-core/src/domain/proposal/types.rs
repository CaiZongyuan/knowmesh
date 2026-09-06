use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::{EvidenceId, ProposalId, ProposalItemId, RunId, SourceRevisionId, Timestamp};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PatchOp {
    CreateNode,
    UpdateNodeSummary,
    AddAlias,
    AddClaim,
    SupersedeClaim,
    RetractClaim,
    AddRelation,
    SupersedeRelation,
    RetractRelation,
    AddEvidence,
    RecordClaimConflict,
    CreateSynthesis,
    UpdateSourceMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProposalKind {
    Manual,
    Compile,
    Refresh,
    Synthesis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProposalState {
    Draft,
    Reviewing,
    Approved,
    Applied,
    Rejected,
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Pending,
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewMethod {
    Explicit,
    Bulk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposalIssue {
    pub code: String,
    pub message: String,
    pub blocking: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposalItem {
    pub id: ProposalItemId,
    pub op: PatchOp,
    pub target_id: String,
    pub payload: Value,
    pub before_sha256: Option<String>,
    pub evidence_ids: Vec<EvidenceId>,
    pub compiler_confidence: Option<f64>,
    pub risk: Risk,
    pub decision: Decision,
    pub decision_reason: Option<String>,
    #[serde(rename = "warnings")]
    pub issues: Vec<ProposalIssue>,
    pub reviewed_sha256: Option<String>,
    pub reviewed_at: Option<Timestamp>,
    pub reviewed_by: Option<String>,
    pub review_method: Option<ReviewMethod>,
    pub human_verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposalInput {
    pub kind: ProposalKind,
    pub base_generation: u64,
    pub schema_hash: String,
    pub source_revision_id: Option<SourceRevisionId>,
    pub compiler_run_id: Option<RunId>,
    pub summary: String,
    pub items: Vec<ProposalItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub version: u32,
    pub id: ProposalId,
    pub kind: ProposalKind,
    pub state: ProposalState,
    pub revision: u32,
    pub base_generation: u64,
    pub schema_hash: String,
    pub source_revision_id: Option<SourceRevisionId>,
    pub compiler_run_id: Option<RunId>,
    pub summary: String,
    pub items: Vec<ProposalItem>,
    pub created_by: String,
    pub updated_by: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub state_reason: Option<String>,
    pub applied_at: Option<Timestamp>,
    pub applied_generation: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ReviewPolicy {
    pub strict: bool,
    pub allow_accept_all: bool,
    pub human_verification_required: bool,
}

impl Default for ReviewPolicy {
    fn default() -> Self {
        Self {
            strict: false,
            allow_accept_all: true,
            human_verification_required: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DecisionChange {
    pub item_id: ProposalItemId,
    pub decision: Decision,
    pub reason: Option<String>,
    pub human_verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewInput {
    pub expected_revision: u32,
    pub accept_all: bool,
    pub decisions: Vec<DecisionChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposalRevision {
    pub expected_revision: u32,
    pub base_generation: u64,
    pub schema_hash: String,
    pub summary: String,
    pub items: Vec<ProposalItem>,
}
