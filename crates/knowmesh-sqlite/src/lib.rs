//! SQLite projection and runtime storage adapter for KnowMesh.

mod impact;
mod index;
mod lexical;
mod maintenance;
mod migrations;
mod rebuild;
mod reconcile;
mod runtime;
mod search;
mod source_read;

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub use impact::SqliteImpactPreview;
pub use knowmesh_core::ports::DatabaseDiagnostics;
use knowmesh_core::{
    domain::{Timestamp, WorkspaceId},
    error::{AppError, AppResult, ErrorType},
};
pub use maintenance::DatabaseAccess;
pub use rebuild::SqliteRebuilder;
use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, params};

#[derive(Debug)]
pub struct SqliteStore {
    pub(crate) connection: Connection,
    path: PathBuf,
    // Drop the connection before releasing its replacement barrier.
    _access: Option<DatabaseAccess>,
}

impl SqliteStore {
    pub(crate) fn from_snapshot_in_memory(
        snapshot: &knowmesh_core::canonical::snapshot::CanonicalSnapshot,
    ) -> AppResult<Self> {
        use knowmesh_core::ports::ProjectionStore;
        let mut connection = Connection::open_in_memory().map_err(database_error)?;
        connection
            .execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(database_error)?;
        migrations::apply(&mut connection)?;
        let mut candidate = Self {
            connection,
            path: PathBuf::new(),
            _access: None,
        };
        candidate.bind_workspace(&snapshot.workspace_id, &snapshot.schema_hash)?;
        candidate.reconcile(snapshot)?;
        Ok(candidate)
    }

    pub fn exclusive_access(path: &Path) -> AppResult<DatabaseAccess> {
        DatabaseAccess::acquire(path, true)
    }

    pub fn open_read_only(path: &Path) -> AppResult<Self> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(database_error)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(database_error)?;
        let version = migrations::validate_history(&connection)?;
        if version != migrations::current_version() {
            return Err(AppError::new(
                ErrorType::Configuration,
                "DATABASE_UPGRADE_REQUIRED",
                "The database requires a migration before it can be queried.",
            )
            .with_hint("Run `knowmesh sync` with the current binary."));
        }
        connection
            .execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(database_error)?;
        Ok(Self {
            connection,
            path: path.to_owned(),
            _access: None,
        })
    }

    pub fn open(path: &Path) -> AppResult<Self> {
        let access = DatabaseAccess::acquire(path, false)?;
        Self::open_with_access(path, access)
    }

    fn open_with_access(path: &Path, access: DatabaseAccess) -> AppResult<Self> {
        let mut connection = Connection::open(path).map_err(database_error)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(database_error)?;
        migrations::validate_history(&connection)?;
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA temp_store=MEMORY;").map_err(database_error)?;
        migrations::apply(&mut connection)?;
        Ok(Self {
            connection,
            path: path.to_owned(),
            _access: Some(access),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn bind_workspace(&self, id: &WorkspaceId, schema_hash: &str) -> AppResult<()> {
        let existing: Option<String> = self
            .connection
            .query_row(
                "SELECT workspace_id FROM workspace_state WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(database_error)?;
        if let Some(existing) = existing {
            return check_workspace_id(id, &existing);
        }
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(database_error)?;
        let now = Timestamp::now().to_string();
        tx.execute("INSERT OR IGNORE INTO workspace_state(singleton,workspace_id,schema_hash,created_at,updated_at) VALUES(1,?1,?2,?3,?3)", params![id.as_str(), schema_hash, now]).map_err(database_error)?;
        let existing: String = tx
            .query_row(
                "SELECT workspace_id FROM workspace_state WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        check_workspace_id(id, &existing)?;
        tx.commit().map_err(database_error)
    }

    pub fn diagnostics(&self) -> AppResult<DatabaseDiagnostics> {
        let mut violations = self
            .connection
            .prepare("PRAGMA foreign_key_check")
            .map_err(database_error)?;
        let foreign_key_violations = violations
            .query_map([], |_| Ok(()))
            .map_err(database_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(database_error)?
            .len();
        Ok(DatabaseDiagnostics {
            sqlite_version: self
                .connection
                .query_row("SELECT sqlite_version()", [], |r| r.get(0))
                .map_err(database_error)?,
            schema_version: self
                .connection
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .map_err(database_error)?,
            journal_mode: self
                .connection
                .pragma_query_value(None, "journal_mode", |r| r.get(0))
                .map_err(database_error)?,
            foreign_keys: self
                .connection
                .pragma_query_value(None, "foreign_keys", |r| r.get(0))
                .map_err(database_error)?,
            busy_timeout_ms: self
                .connection
                .pragma_query_value(None, "busy_timeout", |r| r.get(0))
                .map_err(database_error)?,
            integrity: self
                .connection
                .query_row("PRAGMA integrity_check", [], |r| r.get(0))
                .map_err(database_error)?,
            foreign_key_violations,
        })
    }

    pub fn generation(&self) -> AppResult<u64> {
        self.connection
            .query_row(
                "SELECT indexed_generation FROM workspace_state WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(database_error)?
            .ok_or_else(|| {
                AppError::new(
                    ErrorType::Configuration,
                    "WORKSPACE_NOT_BOUND",
                    "The database must be bound to a workspace before use.",
                )
            })
    }
}

fn check_workspace_id(id: &WorkspaceId, existing: &str) -> AppResult<()> {
    if existing != id.as_str() {
        return Err(AppError::new(
            ErrorType::Configuration,
            "WORKSPACE_ID_MISMATCH",
            "The database belongs to a different workspace.",
        )
        .with_hint("Use the correct workspace or rebuild its derived index."));
    }
    Ok(())
}

pub(crate) fn database_error(error: rusqlite::Error) -> AppError {
    if let rusqlite::Error::SqliteFailure(code, _) = &error {
        match code.code {
            ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => {
                return AppError::new(
                    ErrorType::Conflict,
                    "DATABASE_BUSY",
                    "The database is busy; retry after the active writer finishes.",
                )
                .retryable(true);
            }
            ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase => {
                return AppError::new(
                    ErrorType::Io,
                    "DATABASE_CORRUPT",
                    "The database is corrupt or has an unsupported file format.",
                )
                .with_hint("Run `knowmesh doctor` and preserve the database before recovery.");
            }
            ErrorCode::ConstraintViolation => {
                return AppError::new(
                    ErrorType::Validation,
                    "PROJECTION_CONSTRAINT_VIOLATION",
                    "Projected data violates a database constraint.",
                );
            }
            _ => {}
        }
    }
    AppError::new(
        ErrorType::Io,
        "DATABASE_OPERATION_FAILED",
        "A database operation failed.",
    )
}
