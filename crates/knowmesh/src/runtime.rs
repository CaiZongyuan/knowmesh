use std::path::Path;

use knowmesh_core::{
    canonical::{schema::Schema, workspace::Workspace},
    error::{AppError, AppResult, ErrorType},
    ports::{
        ImpactPreviewBackend, ImpactStore, IndexStore, ProposalStore, RebuildBackend, SearchStore,
        SourceReadStore,
    },
};
use knowmesh_sqlite::SqliteStore;

pub fn open_store(workspace: &Workspace) -> AppResult<Box<dyn ImpactStore>> {
    Ok(Box::new(configured_store(workspace)?))
}

pub fn open_source_store(workspace: &Workspace) -> AppResult<Box<dyn SourceReadStore>> {
    Ok(Box::new(configured_store(workspace)?))
}

pub fn open_search_store(workspace: &Workspace) -> AppResult<Box<dyn SearchStore>> {
    Ok(Box::new(configured_store(workspace)?))
}

pub fn open_proposal_store(
    workspace: &Workspace,
    writable: bool,
) -> AppResult<Box<dyn ProposalStore>> {
    let path = workspace.index_path()?;
    if !path.try_exists().map_err(|_| {
        AppError::new(
            ErrorType::Io,
            "INDEX_UNAVAILABLE",
            "The index path could not be inspected.",
        )
    })? {
        return Err(AppError::new(
            ErrorType::NotFound,
            "INDEX_REQUIRED",
            "Proposal operations require an existing index.",
        )
        .with_hint("Run `knowmesh sync` before creating a Proposal."));
    }
    let store = if writable {
        SqliteStore::open(&path)?
    } else {
        SqliteStore::open_read_only(&path)?
    };
    if store.projection_state()?.workspace_id != workspace.config.workspace.id {
        return Err(AppError::new(
            ErrorType::Configuration,
            "WORKSPACE_ID_MISMATCH",
            "The Proposal index belongs to another workspace.",
        ));
    }
    Ok(Box::new(store))
}

fn configured_store(workspace: &Workspace) -> AppResult<SqliteStore> {
    let schema = Schema::load(workspace)?;
    let store = SqliteStore::open(&workspace.index_path()?)?;
    store.bind_workspace(&workspace.config.workspace.id, &schema.hash)?;
    Ok(store)
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
