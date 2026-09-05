use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use knowmesh_core::{
    domain::{RunId, Timestamp, WorkspaceId},
    error::{AppError, AppResult, ErrorType},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Serialize, Deserialize)]
struct BackupManifest {
    version: u32,
    workspace_id: WorkspaceId,
    created_at: Timestamp,
    files: Vec<BackupFile>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupFile {
    name: String,
    sha256: String,
}

pub(super) fn bundle(path: &Path) -> Vec<PathBuf> {
    ["", "-wal", "-shm"]
        .into_iter()
        .map(|suffix| {
            let mut name = path.as_os_str().to_owned();
            name.push(suffix);
            PathBuf::from(name)
        })
        .collect()
}

pub(super) fn fingerprint(path: &Path) -> AppResult<Vec<Option<String>>> {
    bundle(path)
        .into_iter()
        .take(2)
        .map(|file| hash(&file))
        .collect()
}

pub(super) fn hash(path: &Path) -> AppResult<Option<String>> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(error)),
    };
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

pub(super) fn retain_candidate(path: &Path, retained_root: &Path) -> AppResult<Vec<PathBuf>> {
    let present = bundle(path)
        .into_iter()
        .filter(|path| path.exists())
        .collect::<Vec<_>>();
    if present.is_empty() {
        return Ok(vec![]);
    }
    fs::create_dir_all(retained_root).map_err(io_error)?;
    let directory = retained_root.join(RunId::new().as_str());
    fs::create_dir(&directory).map_err(io_error)?;
    let mut retained = Vec::new();
    for file in present {
        let target = directory.join(file.file_name().ok_or_else(invalid_path)?);
        fs::rename(&file, &target).map_err(io_error)?;
        retained.push(target);
    }
    sync_directory(&directory)?;
    sync_directory(retained_root)?;
    sync_directory(path.parent().ok_or_else(invalid_path)?)?;
    Ok(retained)
}

pub(super) fn backup(
    path: &Path,
    backups: &Path,
    workspace_id: &WorkspaceId,
) -> AppResult<Vec<PathBuf>> {
    let present = bundle(path)
        .into_iter()
        .filter(|path| path.exists())
        .collect::<Vec<_>>();
    if present.is_empty() {
        return Ok(vec![]);
    }
    fs::create_dir_all(backups).map_err(io_error)?;
    let directory = backups.join(format!("rebuild-{}", RunId::new()));
    fs::create_dir(&directory).map_err(io_error)?;
    let mut manifest = BackupManifest {
        version: 1,
        workspace_id: workspace_id.clone(),
        created_at: Timestamp::now(),
        files: vec![],
    };
    let mut paths = Vec::new();
    for original in present {
        let name = original
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(invalid_path)?
            .to_owned();
        let target = directory.join(&name);
        let mut input = File::open(&original).map_err(io_error)?;
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&target)
            .map_err(io_error)?;
        std::io::copy(&mut input, &mut output).map_err(io_error)?;
        output.sync_all().map_err(io_error)?;
        let digest = hash(&target)?.ok_or_else(invalid_path)?;
        if hash(&original)?.as_ref() != Some(&digest) {
            return Err(AppError::new(
                ErrorType::Conflict,
                "REBUILD_BACKUP_CHANGED",
                "Database content changed while its backup was being verified.",
            ));
        }
        manifest.files.push(BackupFile {
            name,
            sha256: digest,
        });
        paths.push(target);
    }
    let bytes = serde_json::to_vec(&manifest).map_err(|_| invalid_path())?;
    let mut marker = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(directory.join("backup.json"))
        .map_err(io_error)?;
    marker.write_all(&bytes).map_err(io_error)?;
    marker.sync_all().map_err(io_error)?;
    sync_directory(&directory)?;
    sync_directory(backups)?;
    sync_directory(backups.parent().ok_or_else(invalid_path)?)?;
    Ok(paths)
}

pub(super) fn planned_backup(path: &Path, backups: &Path) -> AppResult<Vec<PathBuf>> {
    let directory = backups.join(format!("rebuild-{}", RunId::new()));
    bundle(path)
        .into_iter()
        .filter(|path| path.exists())
        .map(|path| Ok(directory.join(path.file_name().ok_or_else(invalid_path)?)))
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReplacementStage {
    CandidateSynced,
    BackupSynced,
    SidecarsRemoved,
    Replaced,
}

pub(super) fn replace(
    next: &Path,
    current: &Path,
    backups: &Path,
    workspace_id: &WorkspaceId,
) -> AppResult<Vec<PathBuf>> {
    replace_observed(next, current, backups, workspace_id, |_| Ok(()))
}

fn replace_observed(
    next: &Path,
    current: &Path,
    backups: &Path,
    workspace_id: &WorkspaceId,
    mut observe: impl FnMut(ReplacementStage) -> AppResult<()>,
) -> AppResult<Vec<PathBuf>> {
    sync_file(next)?;
    observe(ReplacementStage::CandidateSynced)?;
    let paths = backup(current, backups, workspace_id)?;
    observe(ReplacementStage::BackupSynced)?;
    remove_sidecars(current)?;
    observe(ReplacementStage::SidecarsRemoved)?;
    fs::rename(next, current).map_err(io_error)?;
    sync_directory(current.parent().ok_or_else(invalid_path)?)?;
    observe(ReplacementStage::Replaced)?;
    Ok(paths)
}

pub(super) fn remove_sidecars(path: &Path) -> AppResult<()> {
    for sidecar in bundle(path).into_iter().skip(1) {
        match fs::remove_file(sidecar) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(error)),
        }
    }
    Ok(())
}

pub(super) fn retain_backups(
    backups: &Path,
    workspace_id: &WorkspaceId,
    keep: usize,
    current: Option<&Path>,
) -> AppResult<()> {
    if !backups.exists() {
        return Ok(());
    }
    let mut owned = Vec::new();
    for entry in fs::read_dir(backups).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        if !entry.file_type().map_err(io_error)?.is_dir()
            || !entry.file_name().to_string_lossy().starts_with("rebuild-")
        {
            continue;
        }
        let marker = entry.path().join("backup.json");
        if !marker.is_file() || fs::metadata(&marker).map_err(io_error)?.len() > 64 * 1024 {
            continue;
        }
        let Ok(manifest) =
            serde_json::from_slice::<BackupManifest>(&fs::read(marker).map_err(io_error)?)
        else {
            continue;
        };
        if manifest.version != 1 || &manifest.workspace_id != workspace_id {
            continue;
        }
        let mut recognized = true;
        for file in fs::read_dir(entry.path()).map_err(io_error)? {
            let file = file.map_err(io_error)?;
            let name = file.file_name();
            if !file.file_type().map_err(io_error)?.is_file()
                || ![
                    "backup.json",
                    "index.sqlite3",
                    "index.sqlite3-wal",
                    "index.sqlite3-shm",
                    "index.sqlite3.lease",
                ]
                .iter()
                .any(|known| name == *known)
            {
                recognized = false;
            }
        }
        if recognized {
            owned.push((
                current == Some(entry.path().as_path()),
                manifest.created_at,
                entry.path(),
            ));
        }
    }
    owned.sort();
    let remove = owned.len().saturating_sub(keep);
    for (_, _, directory) in owned.into_iter().take(remove) {
        fs::remove_dir_all(directory).map_err(io_error)?;
    }
    sync_directory(backups)
}

pub(super) fn sync_file(path: &Path) -> AppResult<()> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(io_error)?
        .sync_all()
        .map_err(io_error)
}

pub(super) fn sync_directory(path: &Path) -> AppResult<()> {
    #[cfg(unix)]
    File::open(path)
        .map_err(io_error)?
        .sync_all()
        .map_err(io_error)?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub(super) fn io_error(_: std::io::Error) -> AppError {
    AppError::new(
        ErrorType::Io,
        "REBUILD_IO_FAILED",
        "A rebuild file operation failed; existing recovery materials were retained.",
    )
}

fn invalid_path() -> AppError {
    AppError::new(
        ErrorType::Validation,
        "INVALID_REBUILD_PATH",
        "A rebuild artifact path is invalid.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn interrupted_replacement_always_keeps_a_complete_current_database() {
        for stop in [
            ReplacementStage::CandidateSynced,
            ReplacementStage::BackupSynced,
            ReplacementStage::SidecarsRemoved,
            ReplacementStage::Replaced,
        ] {
            let temp = tempfile::tempdir().unwrap();
            let current = temp.path().join("index.sqlite3");
            let next = temp.path().join("index.next.sqlite3");
            let backups = temp.path().join("backups");
            for (path, value) in [(&current, "old"), (&next, "new")] {
                let db = Connection::open(path).unwrap();
                db.execute("CREATE TABLE fixture (value TEXT NOT NULL)", [])
                    .unwrap();
                db.execute("INSERT INTO fixture VALUES(?1)", [value])
                    .unwrap();
            }
            let error = replace_observed(&next, &current, &backups, &WorkspaceId::new(), |stage| {
                if stage == stop {
                    Err(AppError::new(
                        ErrorType::Io,
                        "INJECTED_INTERRUPTION",
                        "Simulated interruption.",
                    ))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
            assert_eq!(error.code, "INJECTED_INTERRUPTION");
            assert!(current.is_file());
            let db = Connection::open(&current).unwrap();
            let expected = if stop == ReplacementStage::Replaced {
                "new"
            } else {
                "old"
            };
            assert_eq!(
                db.query_row("SELECT value FROM fixture", [], |row| row
                    .get::<_, String>(0))
                    .unwrap(),
                expected
            );
            assert_eq!(
                db.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
                    .unwrap(),
                "ok"
            );
            assert_eq!(next.exists(), stop != ReplacementStage::Replaced);
            if stop != ReplacementStage::CandidateSynced {
                let directory = fs::read_dir(&backups)
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path();
                let backup = Connection::open(directory.join("index.sqlite3")).unwrap();
                assert_eq!(
                    backup
                        .query_row("SELECT value FROM fixture", [], |row| row
                            .get::<_, String>(0))
                        .unwrap(),
                    "old"
                );
                assert!(directory.join("backup.json").is_file());
            }
        }
    }
}
