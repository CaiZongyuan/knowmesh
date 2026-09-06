use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    domain::sha256,
    error::{AppError, AppResult, ErrorType},
};

#[derive(Debug)]
pub(crate) struct WorkspaceWriter {
    root: PathBuf,
    _lock: File,
}

#[derive(Debug)]
pub(crate) struct FileChange {
    pub path: PathBuf,
    pub before_sha256: Option<String>,
    pub content: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TransactionState {
    Prepared,
    CanonicalCommitted,
    Indexed,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransactionManifest {
    version: u32,
    pub id: String,
    pub state: TransactionState,
    pub changes: Vec<StagedChange>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StagedChange {
    pub path: PathBuf,
    operation: FileOperation,
    pub before_sha256: Option<String>,
    pub after_sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FileOperation {
    Create,
    Replace,
    Delete,
}

impl FileOperation {
    fn for_hashes(before: &Option<String>, after: &Option<String>) -> AppResult<Self> {
        match (before.is_some(), after.is_some()) {
            (false, true) => Ok(Self::Create),
            (true, true) => Ok(Self::Replace),
            (true, false) => Ok(Self::Delete),
            (false, false) => Err(invalid_journal()),
        }
    }
}

impl WorkspaceWriter {
    pub fn acquire(root: &Path) -> AppResult<Self> {
        let root = root.canonicalize().map_err(io_error)?;
        ensure_directory(&root, Path::new(".knowmesh/locks"))?;
        let lock_path = checked_path(&root, Path::new(".knowmesh/locks/workspace.lock"))?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)
            .map_err(io_error)?;
        FileExt::try_lock_exclusive(&lock).map_err(|_| {
            error(
                ErrorType::Conflict,
                "WORKSPACE_LOCKED",
                "Another workspace writer is active.",
            )
            .retryable(true)
        })?;
        Ok(Self { root, _lock: lock })
    }

    pub fn pending(&self) -> AppResult<Vec<TransactionManifest>> {
        pending(&self.root)
    }

    pub fn prepare(&self, changes: Vec<FileChange>) -> AppResult<String> {
        if !self.pending()?.is_empty() {
            return Err(recovery_required());
        }
        if changes.is_empty() || changes.len() > 10_000 {
            return Err(error(
                ErrorType::Validation,
                "INVALID_TRANSACTION_SIZE",
                "A transaction requires 1 to 10000 changed files.",
            ));
        }
        let mut paths = BTreeSet::new();
        for change in &changes {
            validate_canonical_path(&change.path)?;
            if !paths.insert(path_key(&change.path)) {
                return Err(error(
                    ErrorType::Validation,
                    "DUPLICATE_TRANSACTION_PATH",
                    "A file can occur only once in a transaction.",
                ));
            }
            if change
                .before_sha256
                .as_ref()
                .is_some_and(|hash| !valid_hash(hash))
                || (change.before_sha256.is_none() && change.content.is_none())
            {
                return Err(error(
                    ErrorType::Validation,
                    "INVALID_TRANSACTION_CHANGE",
                    "A change requires a valid precondition and result.",
                ));
            }
            if file_hash(&checked_path(&self.root, &change.path)?)? != change.before_sha256 {
                return Err(error(
                    ErrorType::Conflict,
                    "CANONICAL_FILE_CONFLICT",
                    "A canonical file changed before the transaction was prepared.",
                ));
            }
        }
        let id = ulid::Ulid::new().to_string();
        let staging = PathBuf::from(".knowmesh/staging").join(&id);
        ensure_directory(&self.root, &staging)?;
        let mut staged = Vec::new();
        for (index, change) in changes.into_iter().enumerate() {
            let after_sha256 = change.content.as_ref().map(|bytes| sha256(bytes));
            if let Some(content) = change.content {
                let path = checked_path(&self.root, &staging.join(format!("{index}.blob")))?;
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .map_err(io_error)?;
                file.write_all(&content).map_err(io_error)?;
                file.sync_all().map_err(io_error)?;
            }
            staged.push(StagedChange {
                path: change.path,
                operation: FileOperation::for_hashes(&change.before_sha256, &after_sha256)?,
                before_sha256: change.before_sha256,
                after_sha256,
            });
        }
        sync_directory(&self.root.join(&staging))?;
        let manifest = TransactionManifest {
            version: 1,
            id: id.clone(),
            state: TransactionState::Prepared,
            changes: staged,
        };
        self.save_manifest(&manifest)?;
        Ok(id)
    }

    pub fn apply(&self, id: &str) -> AppResult<TransactionManifest> {
        self.apply_observed(id, |_| Ok(()))
    }

    pub(super) fn apply_observed(
        &self,
        id: &str,
        mut after_replace: impl FnMut(usize) -> AppResult<()>,
    ) -> AppResult<TransactionManifest> {
        let mut manifest = load_manifest(&self.root, id)?;
        if manifest.state == TransactionState::Indexed {
            return Ok(manifest);
        }
        verify_recovery(&self.root, &manifest)?;
        let mut count = 0;
        for (index, change) in manifest.changes.iter().enumerate() {
            let target = checked_path(&self.root, &change.path)?;
            let actual = file_hash(&target)?;
            if actual == change.after_sha256 {
                continue;
            }
            if actual != change.before_sha256 {
                return Err(recovery_conflict());
            }
            let parent = change.path.parent().ok_or_else(invalid_path)?;
            ensure_directory(&self.root, parent)?;
            if change.after_sha256.is_some() {
                let mut temporary =
                    tempfile::NamedTempFile::new_in(target.parent().ok_or_else(invalid_path)?)
                        .map_err(io_error)?;
                let mut input = File::open(self.staged_path(id, index)?).map_err(io_error)?;
                std::io::copy(&mut input, &mut temporary).map_err(io_error)?;
                temporary.as_file().sync_all().map_err(io_error)?;
                if file_hash(temporary.path())? != change.after_sha256 {
                    return Err(error(
                        ErrorType::Conflict,
                        "TRANSACTION_STAGING_CORRUPT",
                        "Staged content changed during recovery; the target was not replaced.",
                    ));
                }
                if file_hash(&target)? != change.before_sha256 {
                    return Err(recovery_conflict());
                }
                if change.before_sha256.is_none() {
                    temporary
                        .persist_noclobber(&target)
                        .map_err(|_| recovery_conflict())?;
                } else {
                    temporary
                        .persist(&target)
                        .map_err(|err| io_error(err.error))?;
                }
            } else {
                fs::remove_file(&target).map_err(io_error)?;
            }
            sync_directory(target.parent().ok_or_else(invalid_path)?)?;
            count += 1;
            after_replace(count)?;
        }
        manifest.state = TransactionState::CanonicalCommitted;
        self.save_manifest(&manifest)?;
        Ok(manifest)
    }

    pub fn mark_indexed(&self, id: &str) -> AppResult<()> {
        let mut manifest = load_manifest(&self.root, id)?;
        if manifest.state != TransactionState::Indexed {
            if manifest.state != TransactionState::CanonicalCommitted {
                return Err(recovery_required());
            }
            for change in &manifest.changes {
                if file_hash(&checked_path(&self.root, &change.path)?)? != change.after_sha256 {
                    return Err(recovery_conflict());
                }
            }
            manifest.state = TransactionState::Indexed;
            self.save_manifest(&manifest)?;
        }
        let staging = checked_path(&self.root, &PathBuf::from(".knowmesh/staging").join(id))?;
        if staging.exists() {
            fs::remove_dir_all(&staging).map_err(io_error)?;
            sync_directory(staging.parent().ok_or_else(invalid_path)?)?;
        }
        Ok(())
    }

    fn staged_path(&self, id: &str, index: usize) -> AppResult<PathBuf> {
        staged_path(&self.root, id, index)
    }

    fn save_manifest(&self, manifest: &TransactionManifest) -> AppResult<()> {
        let relative = PathBuf::from(".knowmesh/transactions").join(&manifest.id);
        ensure_directory(&self.root, &relative)?;
        let directory = checked_path(&self.root, &relative)?;
        let path = checked_path(&self.root, &relative.join("manifest.json"))?;
        let mut file = tempfile::NamedTempFile::new_in(&directory).map_err(io_error)?;
        serde_json::to_writer(&mut file, manifest).map_err(|_| {
            error(
                ErrorType::Internal,
                "TRANSACTION_ENCODE_FAILED",
                "Could not encode the recovery journal.",
            )
        })?;
        file.as_file().sync_all().map_err(io_error)?;
        file.persist(path).map_err(|err| io_error(err.error))?;
        sync_directory(&directory)
    }
}

pub(crate) fn verify_recovery(root: &Path, manifest: &TransactionManifest) -> AppResult<()> {
    // Verify every target and staged hash before any remaining file is changed.
    for (index, change) in manifest.changes.iter().enumerate() {
        let actual = file_hash(&checked_path(root, &change.path)?)?;
        if actual != change.before_sha256 && actual != change.after_sha256 {
            return Err(recovery_conflict());
        }
        if let Some(expected) = &change.after_sha256
            && file_hash(&staged_path(root, &manifest.id, index)?)?.as_ref() != Some(expected)
        {
            return Err(error(
                ErrorType::Conflict,
                "TRANSACTION_STAGING_CORRUPT",
                "Staged content is missing or does not match the transaction hash.",
            ));
        }
    }
    Ok(())
}

pub(crate) fn recovery_content(
    root: &Path,
    manifest: &TransactionManifest,
    relative: &Path,
    max_bytes: u64,
) -> AppResult<Vec<u8>> {
    let path = match manifest
        .changes
        .iter()
        .enumerate()
        .find(|(_, change)| change.path == relative)
    {
        Some((index, change)) if change.after_sha256.is_some() => {
            staged_path(root, &manifest.id, index)?
        }
        Some(_) => return Err(recovery_conflict()),
        None => checked_path(root, relative)?,
    };
    super::workspace::read_bounded(&path, max_bytes)
}

fn staged_path(root: &Path, id: &str, index: usize) -> AppResult<PathBuf> {
    checked_path(
        root,
        &PathBuf::from(".knowmesh/staging")
            .join(id)
            .join(format!("{index}.blob")),
    )
}

pub(crate) fn pending(root: &Path) -> AppResult<Vec<TransactionManifest>> {
    let directory = checked_path(root, Path::new(".knowmesh/transactions"))?;
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(io_error(err)),
    };
    let mut manifests = Vec::new();
    for entry in entries {
        let entry = entry.map_err(io_error)?;
        let kind = entry.file_type().map_err(io_error)?;
        if !kind.is_dir() {
            return Err(error(
                ErrorType::Conflict,
                "INVALID_TRANSACTION_JOURNAL",
                "Transaction journal contains an unexpected entry.",
            ));
        }
        let id = entry.file_name().into_string().map_err(|_| invalid_id())?;
        let manifest_path = entry.path().join("manifest.json");
        if !manifest_path.exists() {
            continue;
        }
        let manifest = load_manifest(root, &id)?;
        if manifest.state != TransactionState::Indexed {
            manifests.push(manifest);
        }
    }
    manifests.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(manifests)
}

fn load_manifest(root: &Path, id: &str) -> AppResult<TransactionManifest> {
    if ulid::Ulid::from_string(id).is_err() || id.len() != 26 || id.to_ascii_uppercase() != id {
        return Err(invalid_id());
    }
    let path = checked_path(
        root,
        &PathBuf::from(".knowmesh/transactions")
            .join(id)
            .join("manifest.json"),
    )?;
    let bytes = super::workspace::read_bounded(&path, 8 * 1024 * 1024)?;
    let manifest: TransactionManifest =
        serde_json::from_slice(&bytes).map_err(|_| invalid_journal())?;
    if manifest.version != 1
        || manifest.id != id
        || manifest.changes.is_empty()
        || manifest.changes.len() > 10_000
    {
        return Err(invalid_journal());
    }
    let mut paths = BTreeSet::new();
    for change in &manifest.changes {
        validate_canonical_path(&change.path)?;
        if change.operation
            != FileOperation::for_hashes(&change.before_sha256, &change.after_sha256)?
        {
            return Err(invalid_journal());
        }
        if !paths.insert(path_key(&change.path))
            || change
                .before_sha256
                .as_ref()
                .is_some_and(|hash| !valid_hash(hash))
            || change
                .after_sha256
                .as_ref()
                .is_some_and(|hash| !valid_hash(hash))
            || (change.before_sha256.is_none() && change.after_sha256.is_none())
        {
            return Err(invalid_journal());
        }
    }
    Ok(manifest)
}

pub(crate) fn checked_path(root: &Path, relative: &Path) -> AppResult<PathBuf> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(invalid_path());
    }
    let mut target = root.to_owned();
    for part in relative.components() {
        target.push(part.as_os_str());
        match target.symlink_metadata() {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(error(
                    ErrorType::Policy,
                    "PATH_OUTSIDE_WORKSPACE",
                    "Workspace writes cannot follow symbolic links.",
                ));
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(io_error(err)),
        }
    }
    Ok(target)
}

pub(crate) fn ensure_directory(root: &Path, relative: &Path) -> AppResult<()> {
    checked_path(root, relative)?;
    let mut path = root.to_owned();
    for part in relative.components() {
        path.push(part.as_os_str());
        match fs::create_dir(&path) {
            Ok(()) => sync_directory(path.parent().ok_or_else(invalid_path)?)?,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists && path.is_dir() => {}
            Err(err) => return Err(io_error(err)),
        }
    }
    Ok(())
}

pub(crate) fn file_hash(path: &Path) -> AppResult<Option<String>> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(io_error(err)),
    };
    if !file.metadata().map_err(io_error)?.is_file() {
        return Err(io_error(std::io::Error::other("Expected a file")));
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(io_error)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(Some(format!("{:x}", hasher.finalize())))
}

pub(crate) fn validate_canonical_path(path: &Path) -> AppResult<()> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || path.components().any(|c| {
            c.as_os_str()
                .to_str()
                .is_none_or(|s| s.ends_with(['.', ' ']) || s.contains([':', '\\']))
        })
        || path.components().next().is_some_and(|c| {
            c.as_os_str().to_str().is_some_and(|s| {
                s.eq_ignore_ascii_case(".git") || s.eq_ignore_ascii_case(".knowmesh")
            })
        })
    {
        return Err(invalid_path());
    }
    Ok(())
}

pub(crate) fn path_key(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> AppResult<()> {
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(io_error)
}
#[cfg(not(unix))]
fn sync_directory(_: &Path) -> AppResult<()> {
    Ok(())
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn error(kind: ErrorType, code: &str, message: &str) -> AppError {
    AppError::new(kind, code, message)
}
fn invalid_path() -> AppError {
    error(
        ErrorType::Policy,
        "INVALID_CANONICAL_PATH",
        "Canonical transaction paths must be relative and cannot target internal directories.",
    )
}
fn invalid_id() -> AppError {
    error(
        ErrorType::Validation,
        "INVALID_TRANSACTION_ID",
        "Transaction IDs must be canonical ULIDs.",
    )
}
fn invalid_journal() -> AppError {
    error(
        ErrorType::Conflict,
        "INVALID_TRANSACTION_JOURNAL",
        "The transaction journal is malformed or unsupported.",
    )
}
fn recovery_conflict() -> AppError {
    error(
        ErrorType::Conflict,
        "TRANSACTION_RECOVERY_CONFLICT",
        "A target contains external changes; recovery materials have been preserved.",
    )
}
pub(crate) fn recovery_required() -> AppError {
    error(
        ErrorType::Conflict,
        "TRANSACTION_RECOVERY_REQUIRED",
        "An unfinished file transaction must be recovered before another write.",
    )
    .with_hint("Run `knowmesh doctor` to inspect the recovery journal.")
}
pub(crate) fn io_error(_: std::io::Error) -> AppError {
    error(
        ErrorType::Io,
        "CANONICAL_IO_FAILED",
        "A canonical file or recovery journal could not be accessed.",
    )
}
