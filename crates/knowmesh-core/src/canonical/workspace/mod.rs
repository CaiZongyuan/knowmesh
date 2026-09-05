mod config;

pub use config::*;

use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::transaction::{self, FileChange, WorkspaceWriter};
use crate::{
    domain::{WorkspaceId, sha256},
    error::{AppError, AppResult, ErrorType},
};

const PURPOSE: &str = "---\nversion: 1\nkind: workspace_purpose\n---\n\n# Research Purpose\n\n## Scope\nVirtual cell models for perturbation prediction.\n\n## Key Questions\nHow do STATE and existing models generalize across cell types?\n\n## Comparison Dimensions\nTraining data, perturbation coverage, evaluation splits, and data leakage.\n";

#[derive(Debug)]
pub struct Workspace {
    pub root: PathBuf,
    pub config: WorkspaceConfig,
    pub purpose: Option<Purpose>,
}

#[derive(Debug, Clone)]
pub struct Purpose {
    pub text: String,
    pub sha256: String,
}

impl Workspace {
    pub fn index_path(&self) -> AppResult<PathBuf> {
        self.runtime_path(Path::new("index.sqlite3"))
    }

    pub fn runtime_path(&self, relative: &Path) -> AppResult<PathBuf> {
        runtime_path(&self.root, relative)
    }

    pub fn load(root: &Path) -> AppResult<Self> {
        let root = root.canonicalize().map_err(|_| workspace_not_found())?;
        let config =
            WorkspaceConfig::parse(&read_bounded(&root.join("knowmesh.yaml"), 1024 * 1024)?)?;
        let purpose = config
            .workspace
            .purpose
            .as_ref()
            .map(|path| -> AppResult<Purpose> {
                let path = confined_existing_path(&root, Path::new(path))?;
                let bytes = read_bounded(&path, 16 * 1024).map_err(|e| {
                    if e.code == "FILE_TOO_LARGE" {
                        AppError::new(
                            ErrorType::Validation,
                            "PURPOSE_TOO_LARGE",
                            "Purpose exceeds 16 KiB.",
                        )
                    } else {
                        e
                    }
                })?;
                let text = String::from_utf8(bytes).map_err(|_| {
                    config::config_error("INVALID_PURPOSE", "Purpose must be UTF-8 Markdown.")
                })?;
                let mut lines = text.lines();
                if lines.next() != Some("---") {
                    return Err(config::config_error(
                        "INVALID_PURPOSE",
                        "Purpose needs versioned YAML frontmatter.",
                    ));
                }
                let mut header = Vec::new();
                let mut closed = false;
                for line in lines.by_ref() {
                    if line == "---" {
                        closed = true;
                        break;
                    }
                    header.push(line);
                }
                let header: serde_yaml::Value =
                    serde_yaml::from_str(&header.join("\n")).map_err(|_| {
                        config::config_error("INVALID_PURPOSE", "Purpose frontmatter is invalid.")
                    })?;
                if !closed
                    || header["version"].as_u64() != Some(1)
                    || header["kind"].as_str() != Some("workspace_purpose")
                {
                    return Err(config::config_error(
                        "INVALID_PURPOSE",
                        "Expected workspace_purpose version 1.",
                    ));
                }
                Ok(Purpose {
                    sha256: sha256(text.as_bytes()),
                    text,
                })
            })
            .transpose()?;
        Ok(Self {
            root,
            config,
            purpose,
        })
    }
}

pub fn resolve_workspace(
    explicit: Option<&Path>,
    environment: Option<&Path>,
    cwd: &Path,
) -> AppResult<PathBuf> {
    resolve_workspace_inner(explicit, environment, cwd, false)
}

pub fn runtime_path(root: &Path, relative: &Path) -> AppResult<PathBuf> {
    transaction::checked_path(root, &Path::new(".knowmesh").join(relative))
}

pub(crate) fn resolve_workspace_inner(
    explicit: Option<&Path>,
    environment: Option<&Path>,
    cwd: &Path,
    allow_recovery: bool,
) -> AppResult<PathBuf> {
    let recognized = |path: &Path| {
        path.join("knowmesh.yaml").is_file()
            || (allow_recovery && path.join(".knowmesh/transactions").is_dir())
    };
    if let Some(path) = explicit.or(environment) {
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            cwd.join(path)
        };
        if !recognized(&path) {
            return Err(workspace_not_found());
        }
        return path.canonicalize().map_err(|_| workspace_not_found());
    }
    cwd.ancestors()
        .find(|path| recognized(path))
        .ok_or_else(workspace_not_found)?
        .canonicalize()
        .map_err(|_| workspace_not_found())
}

pub fn confined_existing_path(root: &Path, relative: &Path) -> AppResult<PathBuf> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return Err(outside_workspace());
    }
    let resolved = root.join(relative).canonicalize().map_err(|_| {
        config::config_error(
            "REFERENCED_FILE_MISSING",
            "A configured workspace file does not exist.",
        )
    })?;
    if !resolved.starts_with(root) {
        return Err(outside_workspace());
    }
    Ok(resolved)
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InitOptions {
    pub name: String,
    pub template: String,
    pub dry_run: bool,
}

impl Default for InitOptions {
    fn default() -> Self {
        Self {
            name: "Knowledge Space".into(),
            template: "research".into(),
            dry_run: false,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct InitReport {
    pub workspace_id: WorkspaceId,
    pub root: PathBuf,
    pub created_paths: Vec<String>,
    pub dry_run: bool,
}

pub fn initialize(root: &Path, options: &InitOptions) -> AppResult<InitReport> {
    if options.name.trim().is_empty()
        || !["research", "general", "clinical"].contains(&options.template.as_str())
    {
        return Err(AppError::new(
            ErrorType::Validation,
            "INVALID_INIT_OPTIONS",
            "Choose a name and the research, general, or clinical preview template.",
        ));
    }
    let root = if root.is_absolute() {
        root.to_owned()
    } else {
        std::env::current_dir().map_err(io_error)?.join(root)
    };
    if !transaction::pending(&root)?.is_empty() {
        return Err(transaction::recovery_required());
    }
    if root.join("knowmesh.yaml").exists() {
        let existing = Workspace::load(&root)?;
        if existing.config.workspace.name != options.name
            || existing.config.workspace.template != options.template
        {
            return Err(initialization_conflict());
        }
        return Ok(InitReport {
            workspace_id: existing.config.workspace.id,
            root: existing.root,
            created_paths: vec![],
            dry_run: options.dry_run,
        });
    }
    let config = WorkspaceConfig::research(options.name.clone(), options.template.clone());
    let mut files = vec![(
        "schemas/base.yaml",
        include_str!("../../../../../schemas/base.yaml").to_owned(),
    )];
    if options.template == "research" {
        files.push(("purpose.md", PURPOSE.to_owned()));
        files.push((
            "schemas/research.yaml",
            include_str!("../../../../../schemas/research.yaml").to_owned(),
        ));
    }
    if options.template == "clinical" {
        files.push((
            "schemas/clinical.yaml",
            include_str!("../../../../../schemas/clinical.yaml").to_owned(),
        ));
    }
    files.push((
        "knowmesh.yaml",
        serde_yaml::to_string(&config).map_err(|_| {
            config::config_error("ENCODE_FAILED", "Could not render workspace configuration.")
        })?,
    ));
    for (relative, _) in &files {
        if root.join(relative).symlink_metadata().is_ok() {
            return Err(initialization_conflict());
        }
    }
    let directories = [
        ".knowmesh",
        ".knowmesh/locks",
        "schemas",
        "sources",
        "knowledge",
        "knowledge/nodes",
        "knowledge/syntheses",
        "evals",
    ];
    for relative in directories {
        let target = root.join(relative);
        if let Ok(meta) = target.symlink_metadata() {
            if meta.file_type().is_symlink() {
                return Err(outside_workspace());
            }
            if !meta.is_dir() {
                return Err(initialization_conflict());
            }
        }
    }
    let ignore_path = root.join(".gitignore");
    if ignore_path
        .symlink_metadata()
        .is_ok_and(|meta| meta.file_type().is_symlink())
    {
        return Err(outside_workspace());
    }
    let (mut ignore, ignore_before) = match fs::read_to_string(&ignore_path) {
        Ok(value) => {
            let hash = sha256(value.as_bytes());
            (value, Some(hash))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (String::new(), None),
        Err(error) => return Err(io_error(error)),
    };
    let update_ignore = !ignore.lines().any(|line| line == ".knowmesh/");
    if update_ignore {
        if !ignore.is_empty() && !ignore.ends_with('\n') {
            ignore.push('\n');
        }
        ignore.push_str(".knowmesh/\n");
    }
    let mut created_paths: Vec<_> = files.iter().map(|(p, _)| (*p).to_owned()).collect();
    if update_ignore {
        created_paths.push(".gitignore".into());
    }
    if !options.dry_run {
        fs::create_dir_all(&root).map_err(io_error)?;
        let writer = WorkspaceWriter::acquire(&root)?;
        for relative in directories {
            transaction::ensure_directory(&root, Path::new(relative))?;
        }
        let mut changes: Vec<_> = files
            .into_iter()
            .map(|(path, content)| FileChange {
                path: path.into(),
                before_sha256: None,
                content: Some(content.into_bytes()),
            })
            .collect();
        if update_ignore {
            changes.insert(
                0,
                FileChange {
                    path: ".gitignore".into(),
                    before_sha256: ignore_before,
                    content: Some(ignore.into_bytes()),
                },
            );
        }
        let id = writer.prepare(changes)?;
        writer.apply(&id)?;
        // Initialization creates no indexed knowledge objects.
        writer.mark_indexed(&id)?;
    }
    Ok(InitReport {
        workspace_id: config.workspace.id,
        root,
        created_paths,
        dry_run: options.dry_run,
    })
}

pub(crate) fn read_bounded(path: &Path, max: u64) -> AppResult<Vec<u8>> {
    let file = fs::File::open(path).map_err(io_error)?;
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() as u64 > max {
        return Err(AppError::new(
            ErrorType::Validation,
            "FILE_TOO_LARGE",
            "Configuration file exceeds its size limit.",
        ));
    }
    Ok(bytes)
}

fn outside_workspace() -> AppError {
    AppError::new(
        ErrorType::Policy,
        "PATH_OUTSIDE_WORKSPACE",
        "The path must remain inside the workspace.",
    )
}
fn initialization_conflict() -> AppError {
    AppError::new(
        ErrorType::Conflict,
        "INITIALIZATION_CONFLICT",
        "Initialization would replace existing workspace content.",
    )
    .with_hint("Choose an empty destination or use the existing workspace.")
}
fn workspace_not_found() -> AppError {
    config::config_error("WORKSPACE_NOT_FOUND", "No workspace was found.")
        .with_hint("Run `knowmesh init <path>` or pass --workspace <path>.")
}
fn io_error(_: std::io::Error) -> AppError {
    AppError::new(
        ErrorType::Io,
        "FILE_ACCESS_FAILED",
        "Could not access a workspace file.",
    )
    .with_hint("Check the path, permissions, and available disk space.")
}
