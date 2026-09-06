use std::time::{Duration, Instant};

use knowmesh_core::error::{AppError, AppResult, ErrorType};
use rusqlite::Connection;

use crate::database_error;

pub(super) struct Deadline<'a> {
    connection: &'a Connection,
    expires: Instant,
}

impl<'a> Deadline<'a> {
    pub(super) fn new(connection: &'a Connection, timeout_ms: u64) -> AppResult<Self> {
        Self::at(
            connection,
            Instant::now() + Duration::from_millis(timeout_ms),
        )
    }

    fn at(connection: &'a Connection, expires: Instant) -> AppResult<Self> {
        connection
            .progress_handler(100, Some(move || Instant::now() >= expires))
            .map_err(database_error)?;
        Ok(Self {
            connection,
            expires,
        })
    }

    pub(super) fn check(&self) -> AppResult<()> {
        if Instant::now() >= self.expires {
            Err(timeout())
        } else {
            Ok(())
        }
    }
}

impl Drop for Deadline<'_> {
    fn drop(&mut self) {
        let _ = self.connection.progress_handler(0, None::<fn() -> bool>);
    }
}

pub(super) fn timeout() -> AppError {
    AppError::new(
        ErrorType::Policy,
        "SEARCH_TIMEOUT",
        "Lexical search exceeded its execution budget.",
    )
    .with_hint("Narrow the query or filters, or increase the bounded search timeout.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_expired_budget_interrupts_sql_and_dropping_it_restores_the_connection() {
        let db = Connection::open_in_memory().unwrap();
        let deadline = Deadline::at(&db, Instant::now()).unwrap();
        let error = db.query_row(
            "WITH RECURSIVE numbers(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM numbers WHERE x<10000) SELECT sum(x) FROM numbers",
            [], |row| row.get::<_, u64>(0),
        ).unwrap_err();
        assert_eq!(super::super::search_error(error).code, "SEARCH_TIMEOUT");
        assert_eq!(deadline.check().unwrap_err().code, "SEARCH_TIMEOUT");
        drop(deadline);
        assert_eq!(
            db.query_row("SELECT 1", [], |row| row.get::<_, u32>(0))
                .unwrap(),
            1
        );
    }
}
