use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::{Evidence, EvidenceStance, ExtractionMethod, Locator, SourceRevisionId};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceInput {
    pub source_revision_id: SourceRevisionId,
    pub quote: String,
    pub locator: Locator,
    pub stance: EvidenceStance,
    pub extraction_method: ExtractionMethod,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct VerificationOptions {
    pub repair_window_chars: usize,
    pub max_search_chars: usize,
}

impl Default for VerificationOptions {
    fn default() -> Self {
        Self {
            repair_window_chars: 64,
            max_search_chars: 100_000,
        }
    }
}

/// A successful verification result; persisted Evidence must be verified again on load.
#[derive(Debug, Clone)]
pub struct VerifiedEvidence {
    pub(super) evidence: Evidence,
    pub locator_repaired: bool,
}

impl VerifiedEvidence {
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }

    pub fn into_evidence(self) -> Evidence {
        self.evidence
    }
}
