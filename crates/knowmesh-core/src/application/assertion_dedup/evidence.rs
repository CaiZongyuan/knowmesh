use std::collections::{BTreeMap, BTreeSet};

use super::{error, hash};
use crate::{
    domain::{Evidence, EvidenceId},
    error::AppResult,
};

pub(super) struct EvidencePool<'a> {
    raw: BTreeMap<EvidenceId, &'a Evidence>,
    canonical_ids: BTreeSet<EvidenceId>,
    identities: BTreeMap<String, EvidenceId>,
    pub aliases: BTreeMap<EvidenceId, EvidenceId>,
}

impl<'a> EvidencePool<'a> {
    pub fn new(
        existing: impl Iterator<Item = &'a Evidence>,
        incoming: impl Iterator<Item = &'a Evidence>,
    ) -> AppResult<Self> {
        let mut pool = Self {
            raw: BTreeMap::new(),
            canonical_ids: BTreeSet::new(),
            identities: BTreeMap::new(),
            aliases: BTreeMap::new(),
        };
        for evidence in existing {
            pool.insert_raw(evidence)?;
            pool.canonical_ids.insert(evidence.id.clone());
        }
        for evidence in incoming {
            pool.insert_raw(evidence)?;
        }
        for id in &pool.canonical_ids {
            pool.identities
                .entry(identity(pool.raw[id])?)
                .or_insert_with(|| id.clone());
        }
        Ok(pool)
    }

    fn insert_raw(&mut self, evidence: &'a Evidence) -> AppResult<()> {
        evidence.validate()?;
        if let Some(previous) = self.raw.insert(evidence.id.clone(), evidence)
            && previous != evidence
        {
            return Err(error(
                "EVIDENCE_ID_CONFLICT",
                "A shared Evidence ID must retain all its original fields.",
            ));
        }
        Ok(())
    }

    pub fn merge(
        &mut self,
        existing: &[Evidence],
        incoming: &[Evidence],
    ) -> AppResult<Vec<Evidence>> {
        let mut result = existing.to_vec();
        let mut used: BTreeSet<_> = existing
            .iter()
            .map(|evidence| evidence.id.clone())
            .collect();
        let mut preferred = BTreeMap::<String, EvidenceId>::new();
        for evidence in existing {
            let key = identity(evidence)?;
            preferred
                .entry(key)
                .and_modify(|id| {
                    if evidence.id < *id {
                        *id = evidence.id.clone();
                    }
                })
                .or_insert_with(|| evidence.id.clone());
        }
        let mut incoming: Vec<_> = incoming.iter().collect();
        incoming.sort_by(|left, right| left.id.cmp(&right.id));
        for evidence in incoming {
            let key = identity(evidence)?;
            // Existing canonical IDs stay explicit; newly generated IDs may reuse physical evidence.
            let id = if self.canonical_ids.contains(&evidence.id) {
                evidence.id.clone()
            } else if let Some(id) = self.aliases.get(&evidence.id) {
                id.clone()
            } else if let Some(id) = preferred.get(&key).or_else(|| self.identities.get(&key)) {
                id.clone()
            } else {
                evidence.id.clone()
            };
            if id != evidence.id {
                self.aliases.insert(evidence.id.clone(), id.clone());
            }
            self.identities.entry(key).or_insert_with(|| id.clone());
            if used.insert(id.clone()) {
                result.push(
                    (*self.raw.get(&id).ok_or_else(|| {
                        error(
                            "EVIDENCE_DEDUP_INVALID",
                            "A deduplication target has no evidence payload.",
                        )
                    })?)
                    .clone(),
                );
            }
        }
        Ok(result)
    }
}

fn identity(evidence: &Evidence) -> AppResult<String> {
    hash(&(
        "evidence-identity-v1",
        &evidence.source_revision_id,
        &evidence.quote_sha256,
        &evidence.locator,
        evidence.stance,
    ))
}
