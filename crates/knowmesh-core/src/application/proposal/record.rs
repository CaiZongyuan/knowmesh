use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    domain::{proposal::Proposal, valid_sha256},
    error::AppResult,
};

pub const MAX_PROPOSAL_RECORD_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposalRecord {
    pub proposal: Proposal,
    pub base_snapshot_sha256: String,
}

impl ProposalRecord {
    pub fn validate(&self) -> AppResult<()> {
        self.proposal.validate()?;
        if !valid_sha256(&self.base_snapshot_sha256)
            || self.proposal.base_generation > i64::MAX as u64
            || self
                .proposal
                .applied_generation
                .is_some_and(|generation| generation > i64::MAX as u64)
            || serde_json::to_vec(self).map_err(|_| invalid())?.len() > MAX_PROPOSAL_RECORD_BYTES
        {
            return Err(invalid());
        }
        Ok(())
    }
}

fn invalid() -> crate::error::AppError {
    super::error(
        "INVALID_PROPOSAL_RECORD",
        "Proposal records require a valid base hash and bounded snapshot/generation.",
    )
}
