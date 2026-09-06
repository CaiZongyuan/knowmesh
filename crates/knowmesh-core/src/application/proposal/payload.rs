use std::collections::BTreeMap;

use schemars::{JsonSchema, Schema, schema_for};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use super::error;
use crate::{
    domain::{
        Author, ClaimId, ClaimRecord, ConflictGroup, Evidence, EvidenceStatus, NodeId,
        NodeMetadata, RelationId, RelationRecord, SynthesisMetadata,
        proposal::{PatchOp, ProposalItem},
    },
    error::AppResult,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchemaInput {
    pub op: crate::domain::proposal::PatchOp,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateNode {
    pub metadata: NodeMetadata,
    pub summary: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub summary: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Alias {
    pub alias: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddClaim {
    pub claim: ClaimRecord,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplaceClaim {
    pub replacement_id: ClaimId,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Retraction {
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddRelation {
    pub relation: RelationRecord,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplaceRelation {
    pub replacement_id: RelationId,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddEvidence {
    pub evidence: Vec<Evidence>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordConflict {
    pub group: ConflictGroup,
    #[serde(default)]
    pub member_statuses: BTreeMap<ClaimId, EvidenceStatus>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSynthesis {
    pub metadata: SynthesisMetadata,
    pub body: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceMetadata {
    pub title: String,
    pub kind: String,
    pub authors: Vec<Author>,
    pub identifiers: BTreeMap<String, String>,
    pub language: Option<String>,
    pub tags: Vec<String>,
    pub represented_nodes: Vec<NodeId>,
}

#[derive(Debug, Clone)]
pub(super) enum Payload {
    CreateNode(CreateNode),
    Summary(Summary),
    Alias(Alias),
    AddClaim(AddClaim),
    ReplaceClaim(ReplaceClaim),
    RetractClaim(Retraction),
    AddRelation(AddRelation),
    ReplaceRelation(ReplaceRelation),
    RetractRelation(Retraction),
    AddEvidence(AddEvidence),
    RecordConflict(RecordConflict),
    CreateSynthesis(CreateSynthesis),
    SourceMetadata(SourceMetadata),
}

impl Payload {
    pub fn decode(item: &ProposalItem) -> AppResult<Self> {
        Ok(match item.op {
            PatchOp::CreateNode => Self::CreateNode(decode(&item.payload)?),
            PatchOp::UpdateNodeSummary => Self::Summary(decode(&item.payload)?),
            PatchOp::AddAlias => Self::Alias(decode(&item.payload)?),
            PatchOp::AddClaim => Self::AddClaim(decode(&item.payload)?),
            PatchOp::SupersedeClaim => Self::ReplaceClaim(decode(&item.payload)?),
            PatchOp::RetractClaim => Self::RetractClaim(decode(&item.payload)?),
            PatchOp::AddRelation => Self::AddRelation(decode(&item.payload)?),
            PatchOp::SupersedeRelation => Self::ReplaceRelation(decode(&item.payload)?),
            PatchOp::RetractRelation => Self::RetractRelation(decode(&item.payload)?),
            PatchOp::AddEvidence => Self::AddEvidence(decode(&item.payload)?),
            PatchOp::RecordClaimConflict => Self::RecordConflict(decode(&item.payload)?),
            PatchOp::CreateSynthesis => Self::CreateSynthesis(decode(&item.payload)?),
            PatchOp::UpdateSourceMetadata => Self::SourceMetadata(decode(&item.payload)?),
        })
    }
    pub fn evidence(&self) -> &[Evidence] {
        match self {
            Self::AddClaim(value) => &value.claim.evidence,
            Self::AddRelation(value) => &value.relation.evidence,
            Self::AddEvidence(value) => &value.evidence,
            _ => &[],
        }
    }
    pub fn phase(&self) -> u8 {
        match self {
            Self::CreateNode(_) => 0,
            Self::AddClaim(_) | Self::AddRelation(_) => 1,
            Self::RecordConflict(_) => 3,
            Self::CreateSynthesis(_) => 4,
            _ => 2,
        }
    }
}

fn decode<T: DeserializeOwned>(value: &serde_json::Value) -> AppResult<T> {
    serde_json::from_value(value.clone()).map_err(|_| {
        error(
            "INVALID_PROPOSAL_PAYLOAD",
            "The payload does not match the closed operation contract.",
        )
    })
}

pub fn schema(op: PatchOp) -> Schema {
    match op {
        PatchOp::CreateNode => schema_for!(CreateNode),
        PatchOp::UpdateNodeSummary => schema_for!(Summary),
        PatchOp::AddAlias => schema_for!(Alias),
        PatchOp::AddClaim => schema_for!(AddClaim),
        PatchOp::SupersedeClaim => schema_for!(ReplaceClaim),
        PatchOp::RetractClaim | PatchOp::RetractRelation => schema_for!(Retraction),
        PatchOp::AddRelation => schema_for!(AddRelation),
        PatchOp::SupersedeRelation => schema_for!(ReplaceRelation),
        PatchOp::AddEvidence => schema_for!(AddEvidence),
        PatchOp::RecordClaimConflict => schema_for!(RecordConflict),
        PatchOp::CreateSynthesis => schema_for!(CreateSynthesis),
        PatchOp::UpdateSourceMetadata => schema_for!(SourceMetadata),
    }
}
