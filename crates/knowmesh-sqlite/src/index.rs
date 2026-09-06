use knowmesh_core::{
    canonical::snapshot::FileManifest,
    error::{AppError, AppResult, ErrorType},
    ports::{DatabaseDiagnostics, IndexStore, ProjectionState, ReconcileReport},
};

use crate::{SqliteStore, database_error};

impl IndexStore for SqliteStore {
    fn apply_proposal(
        &mut self,
        context: &knowmesh_core::application::proposal::apply::ApplyContext,
        canonical: &mut dyn FnMut() -> AppResult<
            knowmesh_core::application::proposal::apply::CanonicalApplication,
        >,
    ) -> AppResult<knowmesh_core::application::proposal::apply::ApplyReport> {
        crate::proposal::apply::commit(self, context, canonical)
    }
    fn projection_state(&self) -> AppResult<ProjectionState> {
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(database_error)?;
        let (workspace_id, schema_hash, snapshot_sha256, generation, warnings): (String, String, String, u64, Option<String>) = tx.query_row("SELECT workspace_id,schema_hash,snapshot_sha256,indexed_generation,snapshot_warnings_json FROM workspace_state WHERE singleton=1", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).map_err(database_error)?;
        let mut statement = tx.prepare("SELECT path,kind,public_id,byte_size,mtime_ns,sha256,format_version FROM file_manifest ORDER BY path").map_err(database_error)?;
        let files = statement
            .query_map([], |row| {
                Ok(FileManifest {
                    path: row.get::<_, String>(0)?.into(),
                    kind: row.get(1)?,
                    public_id: row.get(2)?,
                    byte_size: row.get(3)?,
                    mtime_ns: row.get(4)?,
                    sha256: row.get(5)?,
                    format_version: row.get(6)?,
                })
            })
            .map_err(database_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(database_error)?;
        let count = |table: &str| -> AppResult<usize> {
            tx.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .map_err(database_error)
        };
        Ok(ProjectionState {
            workspace_id: workspace_id.parse()?,
            schema_hash,
            snapshot_sha256,
            projection: ReconcileReport {
                generation,
                changed: false,
                source_count: count("sources")?,
                node_count: count("nodes")?,
                claim_count: count("claims")?,
                relation_count: count("relations")?,
                evidence_count: count("evidence")?,
                synthesis_count: count("syntheses")?,
            },
            files,
            warnings: warnings
                .map(|value| {
                    serde_json::from_str(&value).map_err(|_| {
                        AppError::new(
                            ErrorType::Validation,
                            "INVALID_INDEX_STATE",
                            "Stored snapshot warnings are invalid.",
                        )
                    })
                })
                .transpose()?,
        })
    }

    fn diagnostics(&self) -> AppResult<DatabaseDiagnostics> {
        SqliteStore::diagnostics(self)
    }
}
