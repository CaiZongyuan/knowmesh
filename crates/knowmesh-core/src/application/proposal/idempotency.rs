use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ProposalRecord, workflow::MutationReport};
use crate::{
    domain::{sha256, valid_sha256},
    error::{AppError, AppResult, ErrorType},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IdempotencyRequest {
    pub key: String,
    pub operation: String,
    pub input_sha256: String,
}

impl IdempotencyRequest {
    pub fn new(key: &str, operation: &str, input: &impl Serialize) -> AppResult<Self> {
        let mut value = serde_json::to_value(input).map_err(|_| invalid())?;
        value.as_object_mut().ok_or_else(invalid)?.remove("dry_run");
        if operation == "proposal.apply" {
            value.as_object_mut().ok_or_else(invalid)?.remove("yes");
        }
        let bytes = serde_json::to_vec(&("proposal-request-v1", operation, value))
            .map_err(|_| invalid())?;
        if bytes.len() > super::MAX_PROPOSAL_RECORD_BYTES {
            return Err(invalid());
        }
        let request = Self {
            key: key.into(),
            operation: operation.into(),
            input_sha256: sha256(&bytes),
        };
        request.validate()?;
        Ok(request)
    }
    pub fn validate(&self) -> AppResult<()> {
        if self.key.trim().is_empty()
            || self.key.len() > 256
            || self.key.chars().any(char::is_control)
            || !matches!(
                self.operation.as_str(),
                "proposal.create"
                    | "proposal.edit"
                    | "proposal.review"
                    | "proposal.revalidate"
                    | "proposal.reject"
                    | "proposal.apply"
            )
            || !valid_sha256(&self.input_sha256)
        {
            return Err(invalid());
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ProposalMutation {
    pub record: ProposalRecord,
    pub expected_revision: Option<u32>,
    pub idempotency: Option<IdempotencyRequest>,
    pub error: Option<AppError>,
}

#[derive(Debug, Clone)]
pub struct MutationResult {
    pub record: ProposalRecord,
    pub error: Option<AppError>,
}

impl MutationResult {
    pub fn report(self, dry_run: bool) -> AppResult<MutationReport> {
        match self.error {
            Some(error) => Err(error),
            None => Ok(MutationReport {
                dry_run,
                record: self.record,
            }),
        }
    }
}

pub(crate) fn unavailable() -> AppError {
    AppError::new(
        ErrorType::Configuration,
        "IDEMPOTENCY_UNAVAILABLE",
        "This store cannot atomically save Proposal idempotency results.",
    )
}
fn invalid() -> AppError {
    AppError::new(
        ErrorType::Validation,
        "INVALID_IDEMPOTENCY_KEY",
        "Proposal idempotency requires a bounded nonempty key and recognized request fingerprint.",
    )
}
