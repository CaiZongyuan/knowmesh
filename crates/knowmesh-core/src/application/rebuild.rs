use crate::{
    canonical::{snapshot::CanonicalSnapshot, transaction::WorkspaceWriter, workspace::Workspace},
    error::{AppError, AppResult, ErrorType},
    ports::{RebuildBackend, RebuildReport},
};

pub use crate::ports::RebuildInput;

pub fn execute(
    workspace: &Workspace,
    backend: &dyn RebuildBackend,
    input: &RebuildInput,
) -> AppResult<RebuildReport> {
    if !(1..=20).contains(&input.keep_backups) {
        return Err(AppError::new(
            ErrorType::Validation,
            "INVALID_BACKUP_RETENTION",
            "Retain between 1 and 20 database backups.",
        )
        .with_param("keep_backups"));
    }
    if !input.dry_run && !input.yes {
        return Err(AppError::new(
            ErrorType::Confirmation,
            "CONFIRMATION_REQUIRED",
            "Rebuilding requires explicit confirmation.",
        )
        .with_hint("Review `rebuild --dry-run`, then repeat with --yes."));
    }
    let snapshot = CanonicalSnapshot::scan(workspace)?;
    if input.dry_run {
        return backend.preview(&snapshot, input);
    }
    let candidate = backend.prepare(&snapshot, input)?;
    let _writer = WorkspaceWriter::acquire(&workspace.root)?;
    let current = CanonicalSnapshot::scan(&Workspace::load(&workspace.root)?)?;
    if current.content_sha256 != snapshot.content_sha256 {
        return Err(AppError::new(
            ErrorType::Conflict,
            "REBUILD_CANONICAL_CHANGED",
            "Canonical files changed while the replacement index was being built.",
        )
        .retryable(true)
        .with_hint("The candidate was retained; retry rebuilding from the current files."));
    }
    candidate.publish(&current)
}
