use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{canonical::snapshot::CanonicalSnapshot, error::AppResult};

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ReconcileReport {
    pub generation: u64,
    pub changed: bool,
    pub source_count: usize,
    pub node_count: usize,
    pub claim_count: usize,
    pub relation_count: usize,
    pub evidence_count: usize,
    pub synthesis_count: usize,
}

pub trait ProjectionStore: Send {
    fn reconcile(&mut self, snapshot: &CanonicalSnapshot) -> AppResult<ReconcileReport>;
}
