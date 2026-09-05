use std::{path::Path, process::Command};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    canonical::{snapshot::CanonicalSnapshot, workspace::Workspace},
    domain::WorkspaceId,
    error::{AppError, AppResult, ErrorType},
    ports::{DatabaseDiagnostics, IndexStore},
};

use super::sync::{self, RecoveryReport, SyncReport};

pub enum IndexAccess<'a> {
    Missing,
    Ready(&'a dyn IndexStore),
    Failed(AppError),
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RepairInput {
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub yes: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DiagnosticIssue {
    pub code: String,
    pub severity: String,
    pub message: String,
    pub hint: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GitStatus {
    pub repository: bool,
    pub runtime_ignored: Option<bool>,
    pub runtime_tracked: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DoctorReport {
    pub workspace_id: WorkspaceId,
    pub healthy: bool,
    pub dry_run: bool,
    pub database: Option<DatabaseDiagnostics>,
    pub generation: Option<u64>,
    pub sync_required: Option<bool>,
    pub recovery: Option<RecoveryReport>,
    pub sync: Option<SyncReport>,
    pub git: Option<GitStatus>,
    pub issues: Vec<DiagnosticIssue>,
}

pub fn inspect(workspace: &Workspace, access: IndexAccess<'_>) -> AppResult<DoctorReport> {
    let mut issues = Vec::new();
    let recovery = match sync::recovery_status(workspace) {
        Ok(status) => {
            if status.recovery_required {
                issues.push(issue(
                    "TRANSACTION_RECOVERY_REQUIRED",
                    "error",
                    "A file transaction requires recovery.",
                    "Review `doctor --repair --dry-run`, then use `doctor --repair --yes`.",
                ));
            }
            Some(status)
        }
        Err(error) => {
            issues.push(from_error(error));
            None
        }
    };
    let mut database = None;
    let mut state = None;
    match access {
        IndexAccess::Missing => issues.push(issue(
            "INDEX_MISSING",
            "warning",
            "The workspace has no index yet.",
            "Run `knowmesh sync`.",
        )),
        IndexAccess::Failed(error) => issues.push(from_error(error)),
        IndexAccess::Ready(store) => {
            match store.diagnostics() {
                Ok(diagnostics) => {
                    if diagnostics.integrity != "ok" || diagnostics.foreign_key_violations != 0 {
                        issues.push(issue(
                            "DATABASE_INTEGRITY_FAILED",
                            "error",
                            "Database integrity or foreign-key checks failed.",
                            "Preserve the database and rebuild its projections.",
                        ));
                    }
                    database = Some(diagnostics);
                }
                Err(error) => issues.push(from_error(error)),
            }
            match store.projection_state() {
                Ok(value) if value.workspace_id == workspace.config.workspace.id => {
                    state = Some(value)
                }
                Ok(_) => issues.push(issue(
                    "WORKSPACE_ID_MISMATCH",
                    "error",
                    "The index belongs to another workspace.",
                    "Preserve this database and select the matching workspace.",
                )),
                Err(error) => issues.push(from_error(error)),
            }
        }
    }
    let mut sync_required = None;
    if recovery
        .as_ref()
        .is_some_and(|status| !status.recovery_required)
    {
        match CanonicalSnapshot::scan(workspace) {
            Ok(snapshot) => {
                let changed = state
                    .as_ref()
                    .is_none_or(|state| state.snapshot_sha256 != snapshot.content_sha256);
                sync_required = Some(changed);
                if changed && state.is_some() {
                    issues.push(issue(
                        "INDEX_OUT_OF_DATE",
                        "warning",
                        "Canonical files differ from the complete index snapshot.",
                        "Run `knowmesh sync`.",
                    ));
                }
                issues.extend(
                    snapshot
                        .warnings
                        .into_iter()
                        .map(|warning| DiagnosticIssue {
                            code: warning.code,
                            severity: "warning".into(),
                            message: warning.message,
                            hint: Some(format!("Inspect {}.", warning.path.display())),
                        }),
                );
            }
            Err(error) => issues.push(from_error(error)),
        }
    }
    let git = match git_status(&workspace.root) {
        Ok(status) => {
            if !status.repository {
                issues.push(issue(
                    "GIT_NOT_INITIALIZED",
                    "warning",
                    "The workspace is not inside a Git repository.",
                    "Initialize Git when version history is desired.",
                ));
            } else {
                if status.runtime_ignored != Some(true) {
                    issues.push(issue(
                        "RUNTIME_NOT_IGNORED",
                        "warning",
                        "Git does not ignore the runtime directory.",
                        "Add .knowmesh/ to the workspace .gitignore.",
                    ));
                }
                if status.runtime_tracked {
                    issues.push(issue(
                        "RUNTIME_FILES_TRACKED",
                        "warning",
                        "Git already tracks files under .knowmesh/.",
                        "Review tracked runtime files before removing them from the Git index.",
                    ));
                }
            }
            Some(status)
        }
        Err(error) => {
            let mut warning = from_error(error);
            warning.severity = "warning".into();
            issues.push(warning);
            None
        }
    };
    Ok(DoctorReport {
        workspace_id: workspace.config.workspace.id.clone(),
        healthy: issues.is_empty(),
        dry_run: false,
        database,
        generation: state.map(|state| state.projection.generation),
        sync_required,
        recovery,
        sync: None,
        git,
        issues,
    })
}

pub fn preview_repair(workspace: &Workspace, access: IndexAccess<'_>) -> AppResult<DoctorReport> {
    let mut report = inspect(workspace, access)?;
    report.dry_run = true;
    Ok(report)
}

pub fn validate_repair(input: &RepairInput) -> AppResult<()> {
    if !input.dry_run && !input.yes {
        return Err(AppError::new(
            ErrorType::Confirmation,
            "CONFIRMATION_REQUIRED",
            "Transaction recovery requires explicit confirmation.",
        )
        .with_hint("Review `doctor --repair --dry-run`, then repeat with --yes."));
    }
    Ok(())
}

pub fn repair(
    workspace: &Workspace,
    store: &mut dyn IndexStore,
    input: &RepairInput,
) -> AppResult<DoctorReport> {
    validate_repair(input)?;
    if input.dry_run {
        return preview_repair(workspace, IndexAccess::Ready(store));
    }
    let recovery = sync::recover(workspace, store)?;
    let current = Workspace::load(&workspace.root)?;
    let sync = sync::synchronize(&current, store)?;
    let mut report = inspect(&current, IndexAccess::Ready(store))?;
    report.recovery = Some(recovery);
    report.sync = Some(sync);
    Ok(report)
}

fn git_status(root: &Path) -> AppResult<GitStatus> {
    let run = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_COMMON_DIR")
            .output()
            .map_err(|_| {
                AppError::new(
                    ErrorType::Configuration,
                    "GIT_UNAVAILABLE",
                    "Git checks could not run.",
                )
            })
    };
    let repository = run(&["rev-parse", "--is-inside-work-tree"])?;
    if !repository.status.success() || String::from_utf8_lossy(&repository.stdout).trim() != "true"
    {
        return Ok(GitStatus {
            repository: false,
            runtime_ignored: None,
            runtime_tracked: false,
        });
    }
    let ignored = run(&["check-ignore", "--quiet", "--no-index", "--", ".knowmesh/"])?;
    let tracked = run(&["ls-files", "--", ".knowmesh/"])?;
    if !matches!(ignored.status.code(), Some(0 | 1)) || !tracked.status.success() {
        return Err(AppError::new(
            ErrorType::Configuration,
            "GIT_CHECK_FAILED",
            "Git ignore or tracking checks failed.",
        ));
    }
    Ok(GitStatus {
        repository: true,
        runtime_ignored: Some(ignored.status.success()),
        runtime_tracked: !tracked.stdout.is_empty(),
    })
}

fn issue(code: &str, severity: &str, message: &str, hint: &str) -> DiagnosticIssue {
    DiagnosticIssue {
        code: code.into(),
        severity: severity.into(),
        message: message.into(),
        hint: Some(hint.into()),
    }
}

fn from_error(error: AppError) -> DiagnosticIssue {
    DiagnosticIssue {
        code: error.code,
        severity: "error".into(),
        message: error.message,
        hint: error.hint,
    }
}
