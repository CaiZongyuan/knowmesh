use std::collections::{BTreeMap, BTreeSet};

use super::Documents;
use crate::{
    application::proposal::error,
    canonical::schema::Schema,
    domain::{AssertionDependency, Evidence, EvidenceId, SynthesisMetadata},
    error::AppResult,
};

impl Documents {
    pub(super) fn validate_synthesis(
        &self,
        metadata: &SynthesisMetadata,
        schema: &Schema,
        evidence: &BTreeMap<EvidenceId, Evidence>,
    ) -> AppResult<()> {
        if !schema
            .packs
            .iter()
            .any(|pack| pack.key() == metadata.schema)
        {
            return Err(error(
                "SCHEMA_PACK_NOT_FOUND",
                "The Synthesis references an unavailable Schema Pack.",
            ));
        }
        let snapshot = metadata.dependency_snapshot.as_ref().ok_or_else(|| {
            error(
                "SYNTHESIS_SNAPSHOT_REQUIRED",
                "New Syntheses require an explicit dependency snapshot.",
            )
        })?;
        for assertion in &snapshot.assertions {
            let exists = match assertion {
                AssertionDependency::Claim { id, .. } => self.claim_owners.contains_key(id),
                AssertionDependency::Relation { id, .. } => self.relation_owners.contains_key(id),
            };
            if !exists {
                return Err(error(
                    "SYNTHESIS_DEPENDENCY_NOT_FOUND",
                    "A Synthesis assertion dependency is absent.",
                ));
            }
        }
        let mut source_ids = BTreeSet::new();
        for head in &snapshot.source_heads {
            let source = self.sources.get(&head.source_id).ok_or_else(|| {
                error(
                    "SYNTHESIS_SOURCE_HEAD_INVALID",
                    "A dependency source head must belong to an existing source.",
                )
            })?;
            if !source
                .manifest
                .revisions
                .iter()
                .any(|revision| revision.id == head.revision_id)
            {
                return Err(error(
                    "SYNTHESIS_SOURCE_HEAD_INVALID",
                    "A dependency source head must belong to its declared source.",
                ));
            }
            source_ids.insert(&head.source_id);
        }
        let revisions: BTreeMap<_, _> = self
            .sources
            .values()
            .flat_map(|source| {
                source
                    .manifest
                    .revisions
                    .iter()
                    .map(|revision| (&revision.id, &source.manifest.id))
            })
            .collect();
        for id in &metadata.evidence_ids {
            let reference = evidence.get(id).ok_or_else(|| {
                error(
                    "EVIDENCE_NOT_FOUND",
                    "Synthesis references absent Evidence.",
                )
            })?;
            if revisions
                .get(&reference.source_revision_id)
                .is_none_or(|source| !source_ids.contains(source))
            {
                return Err(error(
                    "SYNTHESIS_SOURCE_HEAD_MISSING",
                    "The dependency snapshot must record each cited source head.",
                ));
            }
        }
        // Historical hashes and heads are retained; freshness is evaluated separately.
        Ok(())
    }
}
