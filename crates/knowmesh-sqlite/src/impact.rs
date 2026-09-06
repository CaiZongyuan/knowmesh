mod context;

use knowmesh_core::{
    application::impact::{ImpactCounts, ImpactData, ImpactObject, ImpactQuery, ImpactRow},
    error::{AppError, AppResult, ErrorType},
    ports::ImpactStore,
};
use rusqlite::{OptionalExtension, params};

use crate::{SqliteStore, database_error};

const EDGES: &str = include_str!("impact/edges.sql");

impl ImpactStore for SqliteStore {
    fn source_impact(&self, query: &ImpactQuery) -> AppResult<ImpactData> {
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(database_error)?;
        let (generation, snapshot_sha256): (u64, String) = tx
            .query_row(
                "SELECT indexed_generation,snapshot_sha256 FROM workspace_state WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(database_error)?;
        if let Some(position) = &query.position
            && (position.generation != generation || position.snapshot_sha256 != snapshot_sha256)
        {
            return Err(AppError::new(
                ErrorType::Conflict,
                "CURSOR_STALE",
                "The index changed after the previous impact page.",
            )
            .with_hint("Restart the query without a cursor."));
        }
        let source = query.source_id.as_str();
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sources WHERE id=?1)",
                [source],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        if !exists {
            return Err(AppError::new(
                ErrorType::NotFound,
                "SOURCE_NOT_FOUND",
                "The source is absent from the index.",
            ));
        }
        if let Some(revision) = &query.revision {
            let owner: Option<String> = tx
                .query_row(
                    "SELECT source_id FROM source_revisions WHERE id=?1",
                    [revision.as_str()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(database_error)?;
            match owner {
                None => {
                    return Err(AppError::new(
                        ErrorType::NotFound,
                        "SOURCE_REVISION_NOT_FOUND",
                        "The source revision is absent from the index.",
                    ));
                }
                Some(owner) if owner != source => {
                    return Err(AppError::new(
                        ErrorType::Validation,
                        "SOURCE_REVISION_MISMATCH",
                        "The revision belongs to another source.",
                    ));
                }
                _ => {}
            }
        }
        let revision = query.revision.as_ref().map(|id| id.as_str());
        let kind = query.kind.map(|kind| kind.as_str());
        let mut counts = ImpactCounts::default();
        let mut statement = tx.prepare(&format!("{EDGES} SELECT kind, count(DISTINCT id) FROM edges WHERE (?3 IS NULL OR kind=?3) GROUP BY kind")).map_err(database_error)?;
        let totals = statement
            .query_map(params![source, revision, kind], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
            })
            .map_err(database_error)?;
        for total in totals {
            let (kind, count) = total.map_err(database_error)?;
            match kind.as_str() {
                "claim" => counts.claims = count,
                "evidence" => counts.evidence = count,
                "relation" => counts.relations = count,
                "synthesis" => counts.syntheses = count,
                _ => return Err(invalid_projection()),
            }
        }
        let (after_kind, after_id) = query
            .position
            .as_ref()
            .map(|position| (position.after.kind().as_str(), position.after.id()))
            .unwrap_or(("", ""));
        let mut statement = tx.prepare(&format!("{EDGES} SELECT kind,id,json_group_array(DISTINCT dependency_id),json_group_array(DISTINCT reason) FROM edges WHERE (?3 IS NULL OR kind=?3) AND (kind,id)>(?4,?5) GROUP BY kind,id ORDER BY kind,id LIMIT ?6")).map_err(database_error)?;
        let values = statement
            .query_map(
                params![
                    source,
                    revision,
                    kind,
                    after_kind,
                    after_id,
                    u64::from(query.limit) + 1
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .map_err(database_error)?;
        let mut items = Vec::new();
        for value in values {
            let (kind, id, dependencies, reasons) = value.map_err(database_error)?;
            let object = match kind.as_str() {
                "claim" => ImpactObject::Claim(id.parse()?),
                "evidence" => ImpactObject::Evidence(id.parse()?),
                "relation" => ImpactObject::Relation(id.parse()?),
                "synthesis" => ImpactObject::Synthesis(id.parse()?),
                _ => return Err(invalid_projection()),
            };
            let mut dependency_ids: Vec<String> =
                serde_json::from_str(&dependencies).map_err(|_| invalid_projection())?;
            dependency_ids.sort();
            let mut reasons: Vec<_> =
                serde_json::from_str(&reasons).map_err(|_| invalid_projection())?;
            reasons.sort();
            items.push(ImpactRow {
                object,
                dependency_ids,
                reasons,
                evidence_ids: vec![],
                snapshot: None,
            });
        }
        let has_more = items.len() > query.limit as usize;
        items.truncate(query.limit as usize);
        let context = context::load(&tx, &mut items)?;
        Ok(ImpactData {
            generation,
            snapshot_sha256,
            counts,
            items,
            context,
            has_more,
        })
    }
}

fn invalid_projection() -> AppError {
    AppError::new(
        ErrorType::Validation,
        "INVALID_PROJECTION_PAYLOAD",
        "An impact dependency projection is invalid.",
    )
}
