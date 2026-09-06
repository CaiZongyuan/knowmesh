use std::path::PathBuf;

use knowmesh_core::{
    application::impact::{ImpactData, ImpactQuery},
    canonical::{snapshot::CanonicalSnapshot, workspace::Workspace},
    error::{AppError, AppResult, ErrorType},
    ports::{ImpactPreviewBackend, ImpactStore, IndexStore},
};

use crate::{SqliteStore, database_error};

pub struct SqliteImpactPreview {
    index: PathBuf,
}

impl SqliteImpactPreview {
    pub fn new(workspace: &Workspace) -> AppResult<Self> {
        Ok(Self {
            index: workspace.index_path()?,
        })
    }
}

impl ImpactPreviewBackend for SqliteImpactPreview {
    fn preview(&self, snapshot: &CanonicalSnapshot, query: &ImpactQuery) -> AppResult<ImpactData> {
        let exists = self.index.try_exists().map_err(|_| {
            AppError::new(
                ErrorType::Io,
                "INDEX_UNAVAILABLE",
                "The index path could not be inspected.",
            )
        })?;
        let generation = if exists {
            let state = SqliteStore::open_read_only(&self.index)?.projection_state()?;
            if state.workspace_id != snapshot.workspace_id {
                return Err(AppError::new(
                    ErrorType::Configuration,
                    "WORKSPACE_ID_MISMATCH",
                    "The existing index belongs to another workspace.",
                ));
            }
            state
                .projection
                .generation
                .checked_add(u64::from(state.snapshot_sha256 != snapshot.content_sha256))
                .ok_or_else(super::invalid_projection)?
        } else {
            1
        };
        let store = SqliteStore::from_snapshot_in_memory(snapshot)?;
        store.connection.execute("UPDATE workspace_state SET canonical_generation=?1,indexed_generation=?1 WHERE singleton=1", [generation]).map_err(database_error)?;
        store.source_impact(query)
    }
}
