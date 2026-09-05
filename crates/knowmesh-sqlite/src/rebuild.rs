mod files;

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use knowmesh_core::{
    canonical::{
        snapshot::{CanonicalSnapshot, SnapshotWarning},
        workspace::Workspace,
    },
    domain::{WorkspaceId, sha256},
    error::{AppError, AppResult, ErrorType},
    ports::{
        IndexStore, ProjectionStore, RebuildBackend, RebuildCandidate, RebuildInput, RebuildReport,
        ReconcileReport,
    },
};
use serde_json::json;

use crate::{DatabaseAccess, SqliteStore, database_error, runtime::RUNTIME_TABLES};

#[derive(Debug, Clone)]
pub struct SqliteRebuilder {
    index: PathBuf,
    next: PathBuf,
    backups: PathBuf,
    retained: PathBuf,
}

impl SqliteRebuilder {
    pub fn new(workspace: &Workspace) -> AppResult<Self> {
        Ok(Self {
            index: workspace.index_path()?,
            next: workspace.runtime_path(Path::new("index.next.sqlite3"))?,
            backups: workspace.runtime_path(Path::new("backups"))?,
            retained: workspace.runtime_path(Path::new("rebuilds/retained"))?,
        })
    }

    fn original(&self, workspace_id: &WorkspaceId) -> AppResult<(Original, Option<SqliteStore>)> {
        let fingerprint = files::fingerprint(&self.index)?;
        if fingerprint.iter().all(Option::is_none) {
            return Ok((Original::Missing, None));
        }
        let source = SqliteStore::open_read_only(&self.index).and_then(|store| {
            let state = store.projection_state()?;
            if &state.workspace_id != workspace_id {
                return Err(AppError::new(
                    ErrorType::Configuration,
                    "WORKSPACE_ID_MISMATCH",
                    "The previous database belongs to another workspace.",
                ));
            }
            Ok((store, state))
        });
        match source {
            Ok((store, state)) => Ok((
                Original::Readable {
                    generation: state.projection.generation,
                    snapshot_sha256: state.snapshot_sha256,
                },
                Some(store),
            )),
            Err(error) if error.code == "DATABASE_CORRUPT" => {
                Ok((Original::Unavailable { fingerprint, error }, None))
            }
            Err(error) => Err(error),
        }
    }
}

enum Original {
    Missing,
    Readable {
        generation: u64,
        snapshot_sha256: String,
    },
    Unavailable {
        fingerprint: Vec<Option<String>>,
        error: AppError,
    },
}

impl Original {
    fn projection(&self, snapshot: &CanonicalSnapshot) -> AppResult<ReconcileReport> {
        let (previous, changed) = match self {
            Self::Readable {
                generation,
                snapshot_sha256,
            } => (*generation, snapshot_sha256 != &snapshot.content_sha256),
            _ => (0, true),
        };
        let generation = previous
            .checked_add(u64::from(changed))
            .ok_or_else(invalid_candidate)?;
        Ok(ReconcileReport {
            generation,
            changed,
            source_count: snapshot.sources.len(),
            node_count: snapshot.nodes.len(),
            claim_count: snapshot.claims.len(),
            relation_count: snapshot.relations.len(),
            evidence_count: snapshot.evidence.len(),
            synthesis_count: snapshot.syntheses.len(),
        })
    }

    fn validate_runtime(&self, input: &RebuildInput, next: &Path) -> AppResult<()> {
        if let Self::Unavailable { error, .. } = self
            && !input.discard_runtime
        {
            return Err(AppError::new(ErrorType::Conflict, "RUNTIME_PRESERVATION_FAILED", "The existing runtime state could not be read; both databases were retained.")
                .with_hint("Inspect the database and candidate. Use --discard-runtime --yes only to explicitly discard runtime state.")
                .with_details(json!({"cause": error.code, "candidate_path": next})));
        }
        Ok(())
    }
}

impl RebuildBackend for SqliteRebuilder {
    fn preview(
        &self,
        snapshot: &CanonicalSnapshot,
        input: &RebuildInput,
    ) -> AppResult<RebuildReport> {
        let (original, source) = self.original(&snapshot.workspace_id)?;
        original.validate_runtime(input, &self.next)?;
        let mut candidate = memory_candidate(snapshot)?;
        let counts = if input.discard_runtime {
            BTreeMap::new()
        } else if let Some(source) = source.as_ref() {
            candidate.copy_runtime_from(source)?.table_counts
        } else {
            BTreeMap::new()
        };
        Ok(RebuildReport {
            dry_run: true,
            projection: original.projection(snapshot)?,
            logical_sha256: snapshot_logical_hash(snapshot)?,
            runtime_table_counts: counts,
            discarded_runtime_tables: discarded(input),
            backup_paths: files::planned_backup(&self.index, &self.backups)?,
            retained_candidate_paths: vec![],
            warnings: snapshot.warnings.clone(),
        })
    }

    fn prepare(
        &self,
        snapshot: &CanonicalSnapshot,
        input: &RebuildInput,
    ) -> AppResult<Box<dyn RebuildCandidate>> {
        snapshot.validate()?;
        let access = SqliteStore::exclusive_access(&self.next)?;
        let retained = files::retain_candidate(&self.next, &self.retained)?;
        let mut next = access.open_store()?;
        next.bind_workspace(&snapshot.workspace_id, &snapshot.schema_hash)?;
        next.reconcile(snapshot)?;
        checkpoint(&next)?;
        let (original, source) = self.original(&snapshot.workspace_id)?;
        original.validate_runtime(input, &self.next)?;
        let counts = if input.discard_runtime {
            BTreeMap::new()
        } else if let Some(source) = source.as_ref() {
            next.copy_runtime_from(source)?.table_counts
        } else {
            BTreeMap::new()
        };
        let report = RebuildReport {
            dry_run: false,
            projection: original.projection(snapshot)?,
            logical_sha256: snapshot_logical_hash(snapshot)?,
            runtime_table_counts: counts,
            discarded_runtime_tables: discarded(input),
            backup_paths: vec![],
            retained_candidate_paths: retained,
            warnings: snapshot.warnings.clone(),
        };
        Ok(Box::new(Candidate {
            paths: self.clone(),
            next: Some(next),
            access,
            original,
            input: input.clone(),
            snapshot_sha256: snapshot.content_sha256.clone(),
            report,
        }))
    }
}

struct Candidate {
    paths: SqliteRebuilder,
    next: Option<SqliteStore>,
    access: DatabaseAccess,
    original: Original,
    input: RebuildInput,
    snapshot_sha256: String,
    report: RebuildReport,
}

impl RebuildCandidate for Candidate {
    fn publish(mut self: Box<Self>, current: &CanonicalSnapshot) -> AppResult<RebuildReport> {
        current.validate()?;
        if self.snapshot_sha256 != current.content_sha256 {
            return Err(invalid_candidate());
        }
        let access = SqliteStore::exclusive_access(&self.paths.index)?;
        let mut next = self.next.take().ok_or_else(invalid_candidate)?;
        match &self.original {
            Original::Missing => {
                if files::fingerprint(&self.paths.index)?
                    .iter()
                    .any(Option::is_some)
                {
                    return Err(generation_changed());
                }
            }
            Original::Unavailable { fingerprint, .. } => {
                if files::fingerprint(&self.paths.index)? != *fingerprint {
                    return Err(generation_changed());
                }
            }
            Original::Readable {
                generation,
                snapshot_sha256,
            } => {
                let source = access.open_store()?;
                let state = source.projection_state()?;
                if state.projection.generation != *generation
                    || state.snapshot_sha256 != *snapshot_sha256
                    || state.workspace_id != current.workspace_id
                {
                    return Err(generation_changed());
                }
                if !self.input.discard_runtime {
                    self.report.runtime_table_counts =
                        next.copy_runtime_from(&source)?.table_counts;
                }
                checkpoint(&source)?;
            }
        }
        next.connection.execute("UPDATE workspace_state SET canonical_generation=?1,indexed_generation=?1 WHERE singleton=1", [self.report.projection.generation]).map_err(database_error)?;
        let actual = sha256(
            &serde_json::to_vec(&next.logical_snapshot()?).map_err(|_| invalid_candidate())?,
        );
        let diagnostics = next.diagnostics()?;
        if actual != self.report.logical_sha256
            || diagnostics.integrity != "ok"
            || diagnostics.foreign_key_violations != 0
        {
            return Err(invalid_candidate());
        }
        checkpoint(&next)?;
        drop(next);
        access.ensure_quiescent()?;
        self.access.ensure_quiescent()?;
        self.report.backup_paths = files::replace(
            &self.paths.next,
            &self.paths.index,
            &self.paths.backups,
            &current.workspace_id,
        )?;
        if let Err(error) = files::retain_backups(
            &self.paths.backups,
            &current.workspace_id,
            self.input.keep_backups,
            self.report
                .backup_paths
                .first()
                .and_then(|path| path.parent()),
        ) {
            self.report.warnings.push(SnapshotWarning {
                code: "BACKUP_RETENTION_FAILED".into(),
                message: error.message,
                path: self.paths.backups.clone(),
            });
        }
        Ok(self.report)
    }
}

fn checkpoint(store: &SqliteStore) -> AppResult<()> {
    let busy: i64 = store
        .connection
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
        .map_err(database_error)?;
    if busy != 0 {
        return Err(AppError::new(
            ErrorType::Conflict,
            "DATABASE_IN_USE",
            "Active SQLite readers prevent a complete rebuild checkpoint.",
        )
        .retryable(true));
    }
    Ok(())
}

fn memory_candidate(snapshot: &CanonicalSnapshot) -> AppResult<SqliteStore> {
    let mut connection = rusqlite::Connection::open_in_memory().map_err(database_error)?;
    connection
        .execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(database_error)?;
    crate::migrations::apply(&mut connection)?;
    let mut candidate = SqliteStore {
        connection,
        path: PathBuf::new(),
        _access: None,
    };
    candidate.bind_workspace(&snapshot.workspace_id, &snapshot.schema_hash)?;
    candidate.reconcile(snapshot)?;
    Ok(candidate)
}

fn discarded(input: &RebuildInput) -> Vec<String> {
    if input.discard_runtime {
        RUNTIME_TABLES.into_iter().map(str::to_owned).collect()
    } else {
        vec![]
    }
}

fn snapshot_logical_hash(snapshot: &CanonicalSnapshot) -> AppResult<String> {
    let value = json!({"sources":snapshot.sources,"nodes":snapshot.nodes,"claims":snapshot.claims,"relations":snapshot.relations,"evidence":snapshot.evidence,"syntheses":snapshot.syntheses});
    Ok(sha256(
        &serde_json::to_vec(&value).map_err(|_| invalid_candidate())?,
    ))
}

fn invalid_candidate() -> AppError {
    AppError::new(
        ErrorType::Conflict,
        "INVALID_REBUILD_CANDIDATE",
        "The candidate does not match the validated canonical projection.",
    )
}

fn generation_changed() -> AppError {
    AppError::new(
        ErrorType::Conflict,
        "REBUILD_GENERATION_CHANGED",
        "The current database changed while the candidate was being built.",
    )
    .retryable(true)
    .with_hint("Both databases were retained; retry rebuilding from the current state.")
}
