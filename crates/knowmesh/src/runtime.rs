use knowmesh_core::{
    canonical::{schema::Schema, workspace::Workspace},
    error::{AppError, AppResult, ErrorType},
};
use knowmesh_sqlite::SqliteStore;

pub fn open_store(workspace: &Workspace) -> AppResult<SqliteStore> {
    let schema = Schema::load(workspace)?;
    let store = SqliteStore::open(&workspace.index_path()?)?;
    store.bind_workspace(&workspace.config.workspace.id, &schema.hash)?;
    Ok(store)
}

pub fn inspect_store(workspace: &Workspace) -> AppResult<Option<SqliteStore>> {
    let path = workspace.index_path()?;
    if !path.try_exists().map_err(|_| {
        AppError::new(
            ErrorType::Io,
            "INDEX_UNAVAILABLE",
            "The index path could not be inspected.",
        )
    })? {
        return Ok(None);
    }
    SqliteStore::open_read_only(&path).map(Some)
}
