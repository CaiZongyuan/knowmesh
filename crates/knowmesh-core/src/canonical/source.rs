use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    transaction::{FileChange, checked_path, io_error, pending, recovery_required},
    workspace::{Workspace, read_bounded},
};
use crate::{
    domain::{
        SourceId, SourceManifest, SourceRevision, SourceRevisionId, StorageMode, Timestamp, sha256,
        source_error, validate_source_url,
    },
    error::{AppError, AppResult, ErrorType},
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportInput {
    pub path: PathBuf,
    pub source_id: Option<SourceId>,
    pub storage: Option<StorageMode>,
    pub title: Option<String>,
    pub kind: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub dry_run: bool,
}

pub struct ImportedContent {
    pub bytes: Vec<u8>,
    pub mime_type: String,
    pub final_url: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ImportReport {
    pub source: SourceManifest,
    pub revision: SourceRevision,
    pub deduplicated: bool,
    pub dry_run: bool,
    pub changed_paths: Vec<PathBuf>,
}

#[derive(Debug)]
pub struct SourcePlan {
    pub source: SourceManifest,
    pub revision: SourceRevision,
    pub deduplicated: bool,
    pub(crate) changes: Vec<FileChange>,
}

impl SourcePlan {
    pub fn report(&self, dry_run: bool) -> ImportReport {
        ImportReport {
            source: self.source.clone(),
            revision: self.revision.clone(),
            deduplicated: self.deduplicated,
            dry_run,
            changed_paths: self.changes.iter().map(|c| c.path.clone()).collect(),
        }
    }
}

#[derive(Debug)]
pub struct SourceFile {
    pub path: PathBuf,
    pub manifest: SourceManifest,
    pub original: String,
    before: SourceManifest,
}

impl SourceFile {
    pub fn parse(path: PathBuf, bytes: &[u8]) -> AppResult<Self> {
        let original = std::str::from_utf8(bytes)
            .map_err(|_| {
                source_error(
                    "INVALID_SOURCE_MANIFEST",
                    "Source manifests must be UTF-8 YAML.",
                )
            })?
            .to_owned();
        if original.lines().any(|line| {
            line.starts_with("<<<<<<< ") || line.starts_with(">>>>>>> ") || line == "======="
        }) {
            return Err(AppError::new(
                ErrorType::Conflict,
                "CANONICAL_FILE_CONFLICT",
                "Resolve Git conflict markers before reading canonical files.",
            ));
        }
        let value: serde_yaml::Value = serde_yaml::from_str(&original).map_err(|_| {
            source_error(
                "INVALID_SOURCE_MANIFEST",
                "Source manifest is not valid YAML.",
            )
        })?;
        if value["version"].as_u64() != Some(1) {
            return Err(source_error(
                "UNSUPPORTED_SOURCE_VERSION",
                "Only source manifest version 1 is supported.",
            ));
        }
        let manifest: SourceManifest = serde_yaml::from_value(value).map_err(|_| {
            source_error(
                "INVALID_SOURCE_MANIFEST",
                "Source manifest fields are invalid or unknown.",
            )
        })?;
        manifest.validate()?;
        Ok(Self {
            path,
            before: manifest.clone(),
            manifest,
            original,
        })
    }

    pub fn render(&self) -> AppResult<String> {
        self.manifest.validate_update(&self.before)?;
        if self.manifest == self.before {
            return Ok(self.original.clone());
        }
        encode(&self.manifest)
    }
}

pub struct SourceLibrary<'a> {
    workspace: &'a Workspace,
}

impl<'a> SourceLibrary<'a> {
    pub fn new(workspace: &'a Workspace) -> Self {
        Self { workspace }
    }

    pub(crate) fn manifest_paths(&self) -> AppResult<Vec<PathBuf>> {
        let root = checked_path(&self.workspace.root, Path::new("sources"))?;
        let entries = fs::read_dir(root).map_err(io_error)?;
        let mut paths = Vec::new();
        for entry in entries {
            let entry = entry.map_err(io_error)?;
            if entry.file_type().map_err(io_error)?.is_file() {
                continue;
            }
            let relative = PathBuf::from("sources")
                .join(entry.file_name())
                .join("source.yaml");
            let path = checked_path(&self.workspace.root, &relative)?;
            if !path.exists() {
                continue;
            }
            paths.push(relative);
        }
        paths.sort();
        Ok(paths)
    }

    pub fn list(&self, include_removed: bool) -> AppResult<Vec<SourceFile>> {
        let mut sources = Vec::new();
        let mut ids = BTreeSet::new();
        for relative in self.manifest_paths()? {
            let path = checked_path(&self.workspace.root, &relative)?;
            let source = SourceFile::parse(relative, &read_bounded(&path, 16 * 1024 * 1024)?)?;
            if !ids.insert(source.manifest.id.clone()) {
                return Err(source_error(
                    "DUPLICATE_SOURCE_ID",
                    "Multiple source manifests have the same ID.",
                ));
            }
            if include_removed || source.manifest.removed_at.is_none() {
                sources.push(source);
            }
        }
        sources.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        Ok(sources)
    }

    pub fn get(&self, id: &SourceId) -> AppResult<SourceFile> {
        self.list(true)?
            .into_iter()
            .find(|source| &source.manifest.id == id)
            .ok_or_else(|| {
                AppError::new(
                    ErrorType::NotFound,
                    "SOURCE_NOT_FOUND",
                    "The source does not exist in this workspace.",
                )
            })
    }

    pub fn plan_add(
        &self,
        input: &ImportInput,
        imported: Option<ImportedContent>,
    ) -> AppResult<SourcePlan> {
        if !pending(&self.workspace.root)?.is_empty() {
            return Err(recovery_required());
        }
        let existing = input
            .source_id
            .as_ref()
            .map(|id| self.get(id))
            .transpose()?;
        if existing
            .as_ref()
            .is_some_and(|file| file.manifest.removed_at.is_some())
        {
            return Err(AppError::new(
                ErrorType::Conflict,
                "SOURCE_REMOVED",
                "Removed sources cannot receive new revisions.",
            ));
        }
        let is_remote = imported.is_some()
            || input
                .path
                .to_str()
                .is_some_and(|p| p.starts_with("https://") || p.starts_with("http://"));
        let storage = existing
            .as_ref()
            .map(|f| f.manifest.storage)
            .or(input.storage)
            .unwrap_or(if is_remote {
                StorageMode::SnapshotUrl
            } else {
                match self.workspace.config.sources.default_storage.as_str() {
                    "referenced" => StorageMode::Referenced,
                    "snapshot-url" => StorageMode::SnapshotUrl,
                    _ => StorageMode::Managed,
                }
            });
        if input.storage.is_some_and(|requested| requested != storage) {
            return Err(source_error(
                "SOURCE_STORAGE_MISMATCH",
                "Appending a revision must preserve the source storage mode.",
            ));
        }
        if is_remote != (storage == StorageMode::SnapshotUrl) {
            return Err(source_error(
                "SOURCE_STORAGE_MISMATCH",
                "URL inputs require snapshot-url; local files require managed or referenced storage.",
            ));
        }
        let limit = self.workspace.config.sources.max_file_mib * 1024 * 1024;
        let (bytes, mime, location, origin_url) = if is_remote {
            if !self.workspace.config.sources.allow_remote_urls {
                return Err(AppError::new(
                    ErrorType::Policy,
                    "REMOTE_URL_DISABLED",
                    "Remote URL ingestion is disabled for this workspace.",
                ));
            }
            let imported = imported.ok_or_else(|| {
                source_error(
                    "SOURCE_FETCH_REQUIRED",
                    "The URL must first be fetched by the configured network adapter.",
                )
            })?;
            validate_source_url(&imported.final_url)?;
            (
                imported.bytes,
                imported.mime_type,
                None,
                Some(imported.final_url),
            )
        } else {
            let location = input.path.canonicalize().map_err(io_error)?;
            let mime = mime_for_extension(&location)?;
            let bytes = read_bounded(&location, limit).map_err(|err| {
                if err.code == "FILE_TOO_LARGE" {
                    source_error(
                        "SOURCE_TOO_LARGE",
                        "Source exceeds the workspace file size limit.",
                    )
                } else {
                    err
                }
            })?;
            (bytes, mime.to_owned(), Some(location), None)
        };
        if bytes.len() as u64 > limit {
            return Err(source_error(
                "SOURCE_TOO_LARGE",
                "Source exceeds the workspace file size limit.",
            ));
        }
        let extension = validate_content(&mime, &bytes)?;
        let hash = sha256(&bytes);
        if let Some(file) = &existing
            && let Some(revision) = file
                .manifest
                .revisions
                .iter()
                .find(|revision| revision.sha256 == hash)
        {
            return Ok(SourcePlan {
                source: file.manifest.clone(),
                revision: revision.clone(),
                deduplicated: true,
                changes: Vec::new(),
            });
        }
        let now = Timestamp::now();
        let revision_id = SourceRevisionId::new();
        let revision = SourceRevision {
            path: if storage == StorageMode::Referenced {
                location
                    .as_ref()
                    .and_then(|path| path.to_str())
                    .ok_or_else(|| {
                        source_error(
                            "INVALID_REVISION_PATH",
                            "Referenced paths must be valid Unicode.",
                        )
                    })?
                    .to_owned()
            } else {
                format!("revisions/{revision_id}/original.{extension}")
            },
            id: revision_id,
            mime_type: mime,
            sha256: hash,
            byte_size: bytes.len() as u64,
            captured_at: now,
            url: origin_url,
        };
        let title = input.title.clone().unwrap_or_else(|| {
            input
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Source")
                .to_owned()
        });
        let (source, manifest_path, before_hash) = if let Some(mut file) = existing {
            file.manifest.revisions.push(revision.clone());
            file.manifest.current_revision_id = revision.id.clone();
            file.manifest.updated_at = now;
            if input.title.is_some() {
                file.manifest.title = title;
            }
            file.manifest.validate_update(&file.before)?;
            (
                file.manifest,
                file.path,
                Some(sha256(file.original.as_bytes())),
            )
        } else {
            let id = SourceId::new();
            let slug = slug(&title);
            let path = PathBuf::from("sources")
                .join(format!("{slug}--{id}"))
                .join("source.yaml");
            (
                SourceManifest {
                    version: 1,
                    id,
                    slug,
                    kind: input.kind.clone(),
                    title,
                    authors: vec![],
                    identifiers: Default::default(),
                    language: None,
                    tags: input.tags.clone(),
                    storage,
                    current_revision_id: revision.id.clone(),
                    represented_nodes: vec![],
                    created_at: now,
                    updated_at: now,
                    removed_at: None,
                    revisions: vec![revision.clone()],
                },
                path,
                None,
            )
        };
        source.validate()?;
        let mut changes = Vec::new();
        if storage != StorageMode::Referenced {
            changes.push(FileChange {
                path: manifest_path
                    .parent()
                    .ok_or_else(|| {
                        source_error("INVALID_REVISION_PATH", "Missing source directory.")
                    })?
                    .join(&revision.path),
                before_sha256: None,
                content: Some(bytes),
            });
        }
        changes.push(FileChange {
            path: manifest_path,
            before_sha256: before_hash,
            content: Some(encode(&source)?.into_bytes()),
        });
        Ok(SourcePlan {
            source,
            revision,
            deduplicated: false,
            changes,
        })
    }

    pub fn plan_remove(&self, id: &SourceId) -> AppResult<SourcePlan> {
        if !pending(&self.workspace.root)?.is_empty() {
            return Err(recovery_required());
        }
        let mut file = self.get(id)?;
        let revision = file
            .manifest
            .revisions
            .iter()
            .find(|revision| revision.id == file.manifest.current_revision_id)
            .cloned()
            .ok_or_else(|| {
                source_error("SOURCE_HEAD_MISSING", "The current revision is missing.")
            })?;
        let mut changes = Vec::new();
        let deduplicated = file.manifest.removed_at.is_some();
        if !deduplicated {
            file.manifest.removed_at = Some(Timestamp::now());
            file.manifest.updated_at = file.manifest.removed_at.unwrap_or(file.manifest.updated_at);
            changes.push(FileChange {
                path: file.path.clone(),
                before_sha256: Some(sha256(file.original.as_bytes())),
                content: Some(file.render()?.into_bytes()),
            });
        }
        Ok(SourcePlan {
            source: file.manifest,
            revision,
            deduplicated,
            changes,
        })
    }

    pub fn content(
        &self,
        source: &SourceManifest,
        revision_id: &SourceRevisionId,
    ) -> AppResult<Vec<u8>> {
        let file = self.get(&source.id)?;
        self.content_at(&file.path, &file.manifest, revision_id)
    }

    pub(crate) fn content_at(
        &self,
        manifest_path: &Path,
        manifest: &SourceManifest,
        revision_id: &SourceRevisionId,
    ) -> AppResult<Vec<u8>> {
        manifest.validate()?;
        let revision = manifest
            .revisions
            .iter()
            .find(|revision| &revision.id == revision_id)
            .ok_or_else(|| {
                AppError::new(
                    ErrorType::NotFound,
                    "SOURCE_REVISION_NOT_FOUND",
                    "The revision does not belong to this source.",
                )
            })?;
        let path = if manifest.storage == StorageMode::Referenced {
            PathBuf::from(&revision.path)
        } else {
            checked_path(
                &self.workspace.root,
                &manifest_path
                    .parent()
                    .ok_or_else(|| {
                        source_error("INVALID_REVISION_PATH", "Missing source directory.")
                    })?
                    .join(&revision.path),
            )?
        };
        let limit = self.workspace.config.sources.max_file_mib * 1024 * 1024;
        if revision.byte_size > limit {
            return Err(source_error(
                "SOURCE_TOO_LARGE",
                "Historical content exceeds the current file size policy.",
            ));
        }
        let bytes = read_bounded(&path, revision.byte_size).map_err(|err| {
            if err.code == "FILE_TOO_LARGE" {
                revision_changed()
            } else {
                err
            }
        })?;
        if bytes.len() as u64 != revision.byte_size || sha256(&bytes) != revision.sha256 {
            return Err(revision_changed());
        }
        Ok(bytes)
    }
}

fn mime_for_extension(path: &Path) -> AppResult<&'static str> {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("md" | "markdown") => Ok("text/markdown"),
        Some("txt") => Ok("text/plain"),
        Some("html" | "htm") => Ok("text/html"),
        Some("pdf") => Ok("application/pdf"),
        _ => Err(source_error(
            "SOURCE_TYPE_UNSUPPORTED",
            "Only Markdown, TXT, HTML, and PDF sources are supported.",
        )),
    }
}

pub(crate) fn validate_content(mime: &str, bytes: &[u8]) -> AppResult<&'static str> {
    let extension = match mime {
        "text/markdown" => "md",
        "text/plain" => "txt",
        "text/html" => "html",
        "application/pdf" => "pdf",
        _ => {
            return Err(source_error(
                "SOURCE_TYPE_UNSUPPORTED",
                "The fetched MIME type is unsupported.",
            ));
        }
    };
    if mime == "application/pdf" {
        if !bytes.starts_with(b"%PDF-") {
            return Err(source_error(
                "SOURCE_MIME_MISMATCH",
                "The file does not contain a PDF header.",
            ));
        }
    } else if std::str::from_utf8(bytes).is_err() {
        return Err(source_error(
            "UNSUPPORTED_ENCODING",
            "Text sources must use UTF-8 encoding.",
        ));
    }
    Ok(extension)
}

fn encode(manifest: &SourceManifest) -> AppResult<String> {
    serde_yaml::to_string(manifest).map_err(|_| {
        source_error(
            "SOURCE_ENCODE_FAILED",
            "Could not encode the source manifest.",
        )
    })
}
fn revision_changed() -> AppError {
    AppError::new(
        ErrorType::Conflict,
        "SOURCE_REVISION_CHANGED",
        "Revision bytes no longer match the immutable content hash.",
    )
    .with_hint("Restore the original snapshot, or import changed content as a new revision.")
}
pub(crate) fn slug(title: &str) -> String {
    let value: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let value = value
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let value = value
        .chars()
        .take(80)
        .collect::<String>()
        .trim_matches('-')
        .to_owned();
    if value.is_empty() {
        "source".into()
    } else {
        value
    }
}
