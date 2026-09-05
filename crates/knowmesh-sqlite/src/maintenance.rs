use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::Arc,
};

use fs2::FileExt;
use knowmesh_core::error::{AppError, AppResult, ErrorType};

#[derive(Debug)]
pub struct DatabaseAccess {
    file: Arc<File>,
    database: PathBuf,
    exclusive: bool,
}

impl DatabaseAccess {
    pub fn open_store(&self) -> AppResult<crate::SqliteStore> {
        let access = Self {
            file: Arc::clone(&self.file),
            database: self.database.clone(),
            exclusive: self.exclusive,
        };
        crate::SqliteStore::open_with_access(&self.database, access)
    }

    pub fn ensure_quiescent(&self) -> AppResult<()> {
        if !self.exclusive || Arc::strong_count(&self.file) != 1 {
            return Err(database_in_use());
        }
        Ok(())
    }

    pub(crate) fn acquire(path: &Path, exclusive: bool) -> AppResult<Self> {
        let database = match path.canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let parent = path
                    .parent()
                    .filter(|path| !path.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."));
                parent
                    .canonicalize()
                    .map_err(lease_error)?
                    .join(path.file_name().ok_or_else(invalid_path)?)
            }
            Err(error) => return Err(lease_error(error)),
        };
        let mut filename = database.file_name().ok_or_else(invalid_path)?.to_owned();
        filename.push(".lease");
        let lease = database.with_file_name(filename);
        match fs::symlink_metadata(&lease) {
            Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
                return Err(invalid_path());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(lease_error(error)),
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lease)
            .map_err(lease_error)?;
        let locked = if exclusive {
            FileExt::try_lock_exclusive(&file)
        } else {
            FileExt::try_lock_shared(&file)
        };
        locked.map_err(|_| database_in_use())?;
        Ok(Self {
            file: Arc::new(file),
            database,
            exclusive,
        })
    }
}

fn database_in_use() -> AppError {
    AppError::new(
        ErrorType::Conflict,
        "DATABASE_IN_USE",
        "Database maintenance conflicts with a live writable connection.",
    )
    .retryable(true)
    .with_hint("Retry after active database connections or maintenance finish.")
}

fn invalid_path() -> AppError {
    AppError::new(
        ErrorType::Configuration,
        "INVALID_DATABASE_LEASE_PATH",
        "A database lease requires a regular file beside the database.",
    )
}

fn lease_error(_: std::io::Error) -> AppError {
    AppError::new(
        ErrorType::Io,
        "DATABASE_LEASE_FAILED",
        "The database connection lease could not be opened.",
    )
}
