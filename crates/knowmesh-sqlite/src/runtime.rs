use std::collections::BTreeMap;

use knowmesh_core::{
    error::{AppError, AppResult, ErrorType},
    ports::{IndexStore, RuntimeCopyReport},
};
use rusqlite::{OptionalExtension, TransactionBehavior, params_from_iter, types::Value};

use crate::{SqliteStore, database_error};

pub(crate) const RUNTIME_TABLES: [&str; 7] = [
    "operation_runs",
    "proposals",
    "proposal_revisions",
    "proposal_applications",
    "proposal_items",
    "idempotency_keys",
    "audit_events",
];

impl SqliteStore {
    pub fn copy_runtime_from(&mut self, source: &SqliteStore) -> AppResult<RuntimeCopyReport> {
        if self.path.canonicalize().ok() == source.path.canonicalize().ok() {
            return Err(AppError::new(
                ErrorType::Validation,
                "REBUILD_SOURCE_EQUALS_TARGET",
                "Runtime copying requires a separate candidate database.",
            ));
        }
        if self.projection_state()?.workspace_id != source.projection_state()?.workspace_id {
            return Err(AppError::new(
                ErrorType::Configuration,
                "WORKSPACE_ID_MISMATCH",
                "Runtime state belongs to another workspace.",
            ));
        }
        // One source read transaction gives every table the same snapshot. Deferred
        // foreign keys allow child runs to precede their parents during insertion.
        let source_tx = source
            .connection
            .unchecked_transaction()
            .map_err(database_error)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        tx.execute_batch(
            "PRAGMA defer_foreign_keys=ON;
            DELETE FROM audit_events; DELETE FROM idempotency_keys;
            DELETE FROM proposal_items; DELETE FROM proposal_applications; DELETE FROM proposal_revisions;
            DELETE FROM proposals; DELETE FROM operation_runs;",
        )
        .map_err(database_error)?;
        let mut table_counts = BTreeMap::new();
        for table in RUNTIME_TABLES {
            let mut read = source_tx
                .prepare(&format!("SELECT * FROM {table}"))
                .map_err(database_error)?;
            let columns = read.column_count();
            let placeholders = vec!["?"; columns].join(",");
            let mut insert = tx
                .prepare(&format!("INSERT INTO {table} VALUES ({placeholders})"))
                .map_err(database_error)?;
            let mut rows = read.query([]).map_err(database_error)?;
            let mut count = 0;
            while let Some(row) = rows.next().map_err(database_error)? {
                let values = (0..columns)
                    .map(|index| row.get::<_, Value>(index))
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(database_error)?;
                insert
                    .execute(params_from_iter(values))
                    .map_err(database_error)?;
                count += 1;
            }
            table_counts.insert(table.to_owned(), count);
        }
        let sequence: Option<i64> = source_tx
            .query_row(
                "SELECT seq FROM sqlite_sequence WHERE name='audit_events'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(database_error)?;
        tx.execute("DELETE FROM sqlite_sequence WHERE name='audit_events'", [])
            .map_err(database_error)?;
        if let Some(sequence) = sequence {
            tx.execute(
                "INSERT INTO sqlite_sequence(name,seq) VALUES('audit_events',?1)",
                [sequence],
            )
            .map_err(database_error)?;
        }
        let invalid: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check)",
                [],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        if invalid {
            return Err(AppError::new(ErrorType::Conflict, "RUNTIME_REFERENCE_MISSING", "Runtime state refers to objects absent from the rebuilt projection.").with_hint("Restore the referenced canonical objects or review runtime references before rebuilding."));
        }
        tx.commit().map_err(database_error)?;
        Ok(RuntimeCopyReport { table_counts })
    }
}
