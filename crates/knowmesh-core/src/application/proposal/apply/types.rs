use std::{collections::BTreeSet, path::PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    canonical::{
        snapshot::CanonicalSnapshot,
        transaction::{path_key, validate_canonical_path},
    },
    domain::{
        ProposalId, SourceId, SourceRevision, StorageMode, Timestamp, WorkspaceId, valid_sha256,
    },
    error::AppResult,
    ports::ReconcileReport,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyInput {
    pub proposal_id: ProposalId,
    pub expected_revision: u32,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub yes: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyFile {
    pub path: PathBuf,
    pub before_sha256: Option<String>,
    pub after_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyContext {
    pub version: u32,
    pub workspace_id: WorkspaceId,
    pub proposal_id: ProposalId,
    pub reviewed_revision: u32,
    pub record_sha256: String,
    pub base_generation: u64,
    pub base_snapshot_sha256: String,
    pub schema_hash: String,
    pub after_snapshot_sha256: String,
    pub files: Vec<ApplyFile>,
    pub sources: Vec<ApplySource>,
    pub actor: String,
    pub requested_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplySource {
    pub source_id: SourceId,
    pub storage: StorageMode,
    pub revision: SourceRevision,
}

impl ApplyContext {
    pub fn validate(&self) -> AppResult<()> {
        if self.version != 1
            || self.reviewed_revision == 0
            || self.reviewed_revision == u32::MAX
            || self.base_generation >= i64::MAX as u64
            || self.files.len() > 10_000
            || self.actor.trim().is_empty()
            || self.actor.len() > 256
            || self.actor.contains('\0')
            || [
                &self.record_sha256,
                &self.base_snapshot_sha256,
                &self.schema_hash,
                &self.after_snapshot_sha256,
            ]
            .iter()
            .any(|value| !valid_sha256(value))
            || (self.files.is_empty() != (self.base_snapshot_sha256 == self.after_snapshot_sha256))
        {
            return Err(super::conflict(
                "INVALID_PROPOSAL_APPLY_CONTEXT",
                "The Proposal Apply context is malformed.",
            ));
        }
        let mut paths = BTreeSet::new();
        for file in &self.files {
            validate_canonical_path(&file.path)?;
            if !paths.insert(path_key(&file.path))
                || !valid_sha256(&file.after_sha256)
                || file
                    .before_sha256
                    .as_ref()
                    .is_some_and(|value| !valid_sha256(value))
            {
                return Err(super::conflict(
                    "INVALID_PROPOSAL_APPLY_CONTEXT",
                    "Apply file preconditions are invalid.",
                ));
            }
        }
        if self
            .sources
            .iter()
            .map(|source| &source.revision.id)
            .collect::<BTreeSet<_>>()
            .len()
            != self.sources.len()
            || serde_json::to_vec(self)
                .map_err(|_| {
                    super::conflict(
                        "INVALID_PROPOSAL_APPLY_CONTEXT",
                        "Could not encode the Apply context.",
                    )
                })?
                .len()
                > 4 * 1024 * 1024
        {
            return Err(super::conflict(
                "INVALID_PROPOSAL_APPLY_CONTEXT",
                "Source bindings must be unique and the Apply context must fit within 4 MiB.",
            ));
        }
        for source in &self.sources {
            if !valid_sha256(&source.revision.sha256) {
                return Err(super::conflict(
                    "INVALID_PROPOSAL_APPLY_CONTEXT",
                    "Source bindings require valid immutable content hashes.",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct CanonicalApplication {
    pub snapshot: CanonicalSnapshot,
    pub transaction_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyReport {
    pub dry_run: bool,
    pub proposal_id: ProposalId,
    pub reviewed_revision: u32,
    pub applied_revision: Option<u32>,
    pub projection: Option<ReconcileReport>,
    pub changed_paths: Vec<PathBuf>,
    pub transaction_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyReceipt {
    pub context: ApplyContext,
    pub report: ApplyReport,
}

impl ApplyReceipt {
    pub fn validate(&self) -> AppResult<()> {
        self.context.validate()?;
        let expected_generation =
            self.context.base_generation + u64::from(!self.context.files.is_empty());
        if self.report.dry_run
            || self.report.proposal_id != self.context.proposal_id
            || self.report.reviewed_revision != self.context.reviewed_revision
            || self.report.applied_revision != self.context.reviewed_revision.checked_add(1)
            || self.report.projection.as_ref().is_none_or(|projection| {
                projection.generation != expected_generation
                    || projection.changed == self.context.files.is_empty()
            })
            || self.report.changed_paths
                != self
                    .context
                    .files
                    .iter()
                    .map(|file| file.path.clone())
                    .collect::<Vec<_>>()
            || self.report.transaction_id.is_none() != self.context.files.is_empty()
            || self.report.transaction_id.as_ref().is_some_and(|id| {
                id.len() != 26
                    || id.to_ascii_uppercase() != *id
                    || ulid::Ulid::from_string(id).is_err()
            })
        {
            return Err(super::conflict(
                "PROPOSAL_RECEIPT_INVALID",
                "The stored Apply receipt does not match its context.",
            ));
        }
        Ok(())
    }
}
