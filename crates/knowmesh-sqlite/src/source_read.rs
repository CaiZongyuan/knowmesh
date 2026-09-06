use knowmesh_core::{
    application::source_read::{ContentId, ListData, ListQuery, SourceData, SourceSummary},
    canonical::snapshot::SourceProjection,
    error::{AppError, AppResult, ErrorType},
    ports::SourceReadStore,
};
use rusqlite::{OptionalExtension, Transaction, params};

use crate::{SqliteStore, database_error};

const FILTER: &str = "(?1 OR removed_at IS NULL) AND (?2 IS NULL OR kind=?2)
    AND (?3 IS NULL OR EXISTS(SELECT 1 FROM json_each(tags_json) WHERE value=?3))";

impl SourceReadStore for SqliteStore {
    fn source_list(&self, query: &ListQuery) -> AppResult<ListData> {
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(database_error)?;
        let (generation, snapshot_sha256) = state(&tx)?;
        if let Some(position) = &query.position
            && (position.generation != generation || position.snapshot_sha256 != snapshot_sha256)
        {
            return Err(AppError::new(
                ErrorType::Conflict,
                "CURSOR_STALE",
                "The index changed after the previous source page.",
            )
            .with_hint("Restart the source list without a cursor."));
        }
        let total = tx
            .query_row(
                &format!("SELECT count(*) FROM sources WHERE {FILTER}"),
                params![query.include_removed, query.kind, query.tag],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        let after = query
            .position
            .as_ref()
            .map(|position| position.after.as_str())
            .unwrap_or("");
        let mut statement = tx.prepare(&format!("SELECT json_object(
            'id',id,'title',title,'kind',kind,'tags',json(tags_json),'storage',storage_mode,
            'current_revision_id',current_revision_id,'status',status,'updated_at',updated_at,'removed_at',removed_at)
            FROM sources WHERE {FILTER} AND id>?4 ORDER BY id LIMIT ?5")).map_err(database_error)?;
        let rows = statement
            .query_map(
                params![
                    query.include_removed,
                    query.kind,
                    query.tag,
                    after,
                    u64::from(query.limit) + 1
                ],
                |row| row.get::<_, String>(0),
            )
            .map_err(database_error)?;
        let mut items = Vec::new();
        for row in rows {
            let item: SourceSummary = serde_json::from_str(&row.map_err(database_error)?)
                .map_err(|_| invalid_projection())?;
            items.push(item);
        }
        let has_more = items.len() > query.limit as usize;
        items.truncate(query.limit as usize);
        Ok(ListData {
            generation,
            snapshot_sha256,
            total,
            items,
            has_more,
        })
    }

    fn source_get(&self, id: &ContentId) -> AppResult<SourceData> {
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(database_error)?;
        let (generation, snapshot_sha256) = state(&tx)?;
        let (sql, value, code) = match id {
            ContentId::Source(id) => (
                "SELECT canonical_json,id,manifest_path FROM sources WHERE id=?1",
                id.as_str(),
                "SOURCE_NOT_FOUND",
            ),
            ContentId::Revision(id) => (
                "SELECT s.canonical_json,s.id,s.manifest_path FROM sources s JOIN source_revisions r ON r.source_id=s.id WHERE r.id=?1",
                id.as_str(),
                "SOURCE_REVISION_NOT_FOUND",
            ),
        };
        let (json, source_id, manifest_path): (String, String, String) = tx
            .query_row(sql, [value], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .optional()
            .map_err(database_error)?
            .ok_or_else(|| {
                AppError::new(
                    ErrorType::NotFound,
                    code,
                    "The source or revision is absent from the index.",
                )
                .with_hint(
                    "Run `knowmesh source list --include-removed` to locate available sources.",
                )
            })?;
        let source: SourceProjection =
            serde_json::from_str(&json).map_err(|_| invalid_projection())?;
        if source.manifest.id.as_str() != source_id
            || source.manifest_path != std::path::Path::new(&manifest_path)
        {
            return Err(invalid_projection());
        }
        Ok(SourceData {
            generation,
            snapshot_sha256,
            source,
        })
    }
}

fn state(tx: &Transaction<'_>) -> AppResult<(u64, String)> {
    tx.query_row(
        "SELECT indexed_generation,snapshot_sha256 FROM workspace_state WHERE singleton=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map_err(database_error)
}

fn invalid_projection() -> AppError {
    AppError::new(
        ErrorType::Validation,
        "INVALID_PROJECTION_PAYLOAD",
        "A source projection is invalid.",
    )
}
