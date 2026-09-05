use knowmesh_core::{
    canonical::{schema::Schema, workspace::Workspace},
    error::AppResult,
};
use knowmesh_sqlite::SqliteStore;

pub fn open_store(workspace: &Workspace) -> AppResult<SqliteStore> {
    let schema = Schema::load(workspace)?;
    let store = SqliteStore::open(&workspace.index_path()?)?;
    store.bind_workspace(&workspace.config.workspace.id, &schema.hash)?;
    Ok(store)
}
