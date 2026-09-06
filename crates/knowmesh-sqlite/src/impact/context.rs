use std::collections::{BTreeMap, BTreeSet};

use knowmesh_core::{
    application::impact::{ImpactObject, ImpactRow},
    canonical::snapshot::{ClaimProjection, RelationProjection, SynthesisProjection},
    domain::{
        AssertionDependency, SourceId, SourceRevisionId,
        freshness::{AssertionState, FreshnessContext, SourceState},
    },
    error::AppResult,
};
use rusqlite::Connection;
use serde::de::DeserializeOwned;

use super::invalid_projection;
use crate::{database_error, reconcile::json_text};

pub(crate) fn load(db: &Connection, items: &mut [ImpactRow]) -> AppResult<FreshnessContext> {
    let mut context = FreshnessContext::default();
    let mut claims = BTreeSet::new();
    let mut relations = BTreeSet::new();
    let mut synthesis_ids = BTreeSet::new();
    let mut evidence_ids = BTreeSet::new();
    for item in items.iter() {
        match &item.object {
            ImpactObject::Claim(id) => {
                claims.insert(id.to_string());
            }
            ImpactObject::Relation(id) => {
                relations.insert(id.to_string());
            }
            ImpactObject::Synthesis(id) => {
                synthesis_ids.insert(id.to_string());
            }
            ImpactObject::Evidence(id) => {
                evidence_ids.insert(id.clone());
            }
        }
    }
    let mut syntheses = BTreeMap::new();
    let mut source_ids = BTreeSet::new();
    let mut revision_ids = BTreeSet::new();
    for synthesis in payloads::<SynthesisProjection>(db, "syntheses", &synthesis_ids)? {
        if let Some(snapshot) = &synthesis.metadata.dependency_snapshot {
            for dependency in &snapshot.assertions {
                match dependency {
                    AssertionDependency::Claim { id, .. } => {
                        claims.insert(id.to_string());
                    }
                    AssertionDependency::Relation { id, .. } => {
                        relations.insert(id.to_string());
                    }
                }
            }
            for head in &snapshot.source_heads {
                source_ids.insert(head.source_id.clone());
                revision_ids.insert(head.revision_id.clone());
            }
        }
        evidence_ids.extend(synthesis.metadata.evidence_ids.iter().cloned());
        syntheses.insert(synthesis.metadata.id.clone(), synthesis.metadata);
    }
    for claim in payloads::<ClaimProjection>(db, "claims", &claims)? {
        let id = claim.claim.assertion.id;
        let evidence = claim
            .claim
            .assertion
            .evidence
            .into_iter()
            .map(|evidence| evidence.id)
            .collect::<Vec<_>>();
        evidence_ids.extend(evidence.iter().cloned());
        context.assertions.insert(
            id.to_string(),
            AssertionState {
                dependency: AssertionDependency::Claim {
                    id,
                    semantic_sha256: claim.semantic_sha256,
                },
                evidence_ids: evidence,
            },
        );
    }
    for relation in payloads::<RelationProjection>(db, "relations", &relations)? {
        let id = relation.relation.assertion.id;
        let evidence = relation
            .relation
            .assertion
            .evidence
            .into_iter()
            .map(|evidence| evidence.id)
            .collect::<Vec<_>>();
        evidence_ids.extend(evidence.iter().cloned());
        context.assertions.insert(
            id.to_string(),
            AssertionState {
                dependency: AssertionDependency::Relation {
                    id,
                    semantic_sha256: relation.semantic_sha256,
                },
                evidence_ids: evidence,
            },
        );
    }
    for item in items {
        match &item.object {
            ImpactObject::Claim(_) | ImpactObject::Relation(_) => {
                item.evidence_ids = context
                    .assertions
                    .get(item.object.id())
                    .ok_or_else(invalid_projection)?
                    .evidence_ids
                    .clone();
            }
            ImpactObject::Synthesis(id) => {
                let synthesis = syntheses.get(id).ok_or_else(invalid_projection)?;
                item.evidence_ids = synthesis.evidence_ids.clone();
                item.snapshot = synthesis.dependency_snapshot.clone();
            }
            ImpactObject::Evidence(id) => item.evidence_ids = vec![id.clone()],
        }
    }
    let mut statement = db.prepare("SELECT id,source_revision_id FROM evidence WHERE id IN (SELECT value FROM json_each(?1))").map_err(database_error)?;
    for row in statement
        .query_map([json_text(&evidence_ids)?], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(database_error)?
    {
        let (id, revision) = row.map_err(database_error)?;
        let revision: SourceRevisionId = revision.parse()?;
        revision_ids.insert(revision.clone());
        context.evidence.insert(id.parse()?, revision);
    }
    let mut statement = db.prepare("SELECT id,source_id FROM source_revisions WHERE id IN (SELECT value FROM json_each(?1))").map_err(database_error)?;
    for row in statement
        .query_map([json_text(&revision_ids)?], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(database_error)?
    {
        let (id, source) = row.map_err(database_error)?;
        let source: SourceId = source.parse()?;
        source_ids.insert(source.clone());
        context.revisions.insert(id.parse()?, source);
    }
    let mut statement = db.prepare("SELECT id,current_revision_id,removed_at IS NOT NULL FROM sources WHERE id IN (SELECT value FROM json_each(?1))").map_err(database_error)?;
    for row in statement
        .query_map([json_text(&source_ids)?], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, bool>(2)?,
            ))
        })
        .map_err(database_error)?
    {
        let (id, revision, removed) = row.map_err(database_error)?;
        if let Some(revision) = revision {
            context.sources.insert(
                id.parse()?,
                SourceState {
                    current_revision_id: revision.parse()?,
                    removed,
                },
            );
        }
    }
    Ok(context)
}

fn payloads<T: DeserializeOwned>(
    db: &Connection,
    table: &str,
    ids: &BTreeSet<String>,
) -> AppResult<Vec<T>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let mut statement = db.prepare(&format!("SELECT canonical_json FROM {table} WHERE id IN (SELECT value FROM json_each(?1)) ORDER BY id")).map_err(database_error)?;
    statement
        .query_map([json_text(ids)?], |row| row.get::<_, String>(0))
        .map_err(database_error)?
        .map(|row| {
            serde_json::from_str(&row.map_err(database_error)?).map_err(|_| invalid_projection())
        })
        .collect()
}
