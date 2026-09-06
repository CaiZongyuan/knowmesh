use knowmesh_core::{
    domain::{Timestamp, sha256},
    error::{AppError, AppResult, ErrorType},
};
use rusqlite::{Connection, TransactionBehavior, params};

use crate::database_error;

const MIGRATIONS: &[(u32, &str, &str)] = &[
    (1, "initial", include_str!("../migrations/0001_initial.sql")),
    (
        2,
        "canonical_payloads",
        include_str!("../migrations/0002_canonical_payloads.sql"),
    ),
    (
        3,
        "snapshot_warnings",
        include_str!("../migrations/0003_snapshot_warnings.sql"),
    ),
    (
        4,
        "claim_normalization",
        include_str!("../migrations/0004_claim_normalization.sql"),
    ),
    (
        5,
        "node_summary_sections",
        include_str!("../migrations/0005_node_summary_sections.sql"),
    ),
    (
        6,
        "proposal_revisions",
        include_str!("../migrations/0006_proposal_revisions.sql"),
    ),
];

pub(crate) fn current_version() -> u32 {
    MIGRATIONS.last().map(|migration| migration.0).unwrap_or(0)
}

pub(crate) fn validate_history(connection: &Connection) -> AppResult<u32> {
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(database_error)?;
    if version > MIGRATIONS.last().map(|m| m.0).unwrap_or(0) {
        return Err(AppError::new(
            ErrorType::Configuration,
            "UNSUPPORTED_DATABASE_VERSION",
            "This database was created by a newer KnowMesh version.",
        ));
    }
    let has_ledger: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='schema_migrations')", [], |r| r.get(0)).map_err(database_error)?;
    if !has_ledger {
        let table_count: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get(0),
            )
            .map_err(database_error)?;
        if version != 0 || table_count != 0 {
            return Err(invalid_history());
        }
        return Ok(0);
    }
    let mut statement = connection
        .prepare("SELECT version,name,checksum FROM schema_migrations ORDER BY version")
        .map_err(database_error)?;
    let history = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(database_error)?;
    let mut count = 0;
    for row in history {
        let (applied, name, checksum) = row.map_err(database_error)?;
        let expected = MIGRATIONS.get(count).ok_or_else(invalid_history)?;
        if applied != expected.0 || name != expected.1 {
            return Err(invalid_history());
        }
        if checksum != sha256(expected.2.as_bytes()) {
            return Err(AppError::new(
                ErrorType::Configuration,
                "MIGRATION_CHECKSUM_MISMATCH",
                "An applied migration does not match this binary's migration history.",
            ));
        }
        count += 1;
    }
    if count != version as usize {
        return Err(invalid_history());
    }
    Ok(version)
}

pub(crate) fn apply(connection: &mut Connection) -> AppResult<()> {
    if validate_history(connection)? == MIGRATIONS.last().map(|m| m.0).unwrap_or(0) {
        return Ok(());
    }
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let version = validate_history(&transaction)?;
    for &(target, name, sql) in MIGRATIONS.iter().filter(|(target, _, _)| *target > version) {
        transaction.execute_batch(sql).map_err(database_error)?;
        transaction.execute("INSERT INTO schema_migrations(version,name,applied_at,checksum) VALUES(?1,?2,?3,?4)", params![target, name, Timestamp::now().to_string(), sha256(sql.as_bytes())]).map_err(database_error)?;
        transaction
            .pragma_update(None, "user_version", target)
            .map_err(database_error)?;
    }
    transaction.commit().map_err(database_error)
}

fn invalid_history() -> AppError {
    AppError::new(
        ErrorType::Configuration,
        "MIGRATION_HISTORY_INVALID",
        "The database migration ledger is missing, inconsistent, or not recognized.",
    )
}
