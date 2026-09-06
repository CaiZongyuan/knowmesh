use std::path::Path;

use knowmesh_core::{
    canonical::{schema::Schema, workspace::Workspace},
    error::{AppError, AppResult, ErrorType},
    ports::{ImpactPreviewBackend, ImpactStore, IndexStore, RebuildBackend},
};
use knowmesh_sqlite::SqliteStore;

pub fn open_store(workspace: &Workspace) -> AppResult<Box<dyn ImpactStore>> {
    let schema = Schema::load(workspace)?;
    let store = SqliteStore::open(&workspace.index_path()?)?;
    store.bind_workspace(&workspace.config.workspace.id, &schema.hash)?;
    Ok(Box::new(store))
}

pub fn rebuild_backend(workspace: &Workspace) -> AppResult<impl RebuildBackend> {
    knowmesh_sqlite::SqliteRebuilder::new(workspace)
}

pub fn impact_preview_backend(workspace: &Workspace) -> AppResult<impl ImpactPreviewBackend> {
    knowmesh_sqlite::SqliteImpactPreview::new(workspace)
}

pub fn inspect_store_at(root: &Path) -> AppResult<Option<Box<dyn IndexStore>>> {
    let path = knowmesh_core::canonical::workspace::runtime_path(root, Path::new("index.sqlite3"))?;
    if !path.try_exists().map_err(|_| {
        AppError::new(
            ErrorType::Io,
            "INDEX_UNAVAILABLE",
            "The index path could not be inspected.",
        )
    })? {
        return Ok(None);
    }
    SqliteStore::open_read_only(&path).map(|store| Some(Box::new(store) as Box<dyn IndexStore>))
}
