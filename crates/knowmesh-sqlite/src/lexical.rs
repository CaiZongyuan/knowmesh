mod deadline;

use knowmesh_core::{
    application::lexical::{
        ChannelCandidates, LexicalCandidates, LexicalChannel, LexicalHit, LexicalQuery, QuerySyntax,
    },
    error::{AppError, AppResult, ErrorType},
    ports::LexicalSearchStore,
};
use rusqlite::{Transaction, params};

use crate::{SqliteStore, database_error};
use deadline::Deadline;

const FILTER: &str =
    "(json_array_length(?2)=0 OR u.record_type IN (SELECT value FROM json_each(?2)))
    AND (json_array_length(?3)=0 OR u.lifecycle_status IN (SELECT value FROM json_each(?3)))";
const SHORT_FILTER: &str = r"NOT EXISTS(SELECT 1 FROM json_each(?5) AS term
    WHERE NOT (u.title LIKE term.value ESCAPE '\' OR u.aliases LIKE term.value ESCAPE '\'))";

impl LexicalSearchStore for SqliteStore {
    fn search_lexical(&self, query: &LexicalQuery) -> AppResult<LexicalCandidates> {
        query.validate()?;
        let deadline = Deadline::new(&self.connection, query.timeout_ms)?;
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(database_error)?;
        let (generation, snapshot_sha256) = tx
            .query_row(
                "SELECT indexed_generation,snapshot_sha256 FROM workspace_state WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(database_error)?;
        let mut channels = Vec::new();
        let terms: Vec<_> = query.query.split_whitespace().collect();
        let word_expression = match query.query_syntax {
            QuerySyntax::Literal => literal_expression(&terms),
            QuerySyntax::Advanced => query.query.clone(),
        };
        channels.push(candidates(
            &tx,
            query,
            LexicalChannel::Word,
            &word_expression,
            &[],
        )?);
        match query.query_syntax {
            QuerySyntax::Advanced => {
                channels.push(candidates(
                    &tx,
                    query,
                    LexicalChannel::Trigram,
                    &query.query,
                    &[],
                )?);
            }
            QuerySyntax::Literal => {
                let (long, short): (Vec<_>, Vec<_>) = terms
                    .into_iter()
                    .partition(|term| term.chars().count() >= 3);
                let patterns = short.into_iter().map(like_pattern).collect::<Vec<_>>();
                let channel = if long.is_empty() {
                    LexicalChannel::ShortText
                } else {
                    LexicalChannel::Trigram
                };
                channels.push(candidates(
                    &tx,
                    query,
                    channel,
                    &literal_expression(&long),
                    &patterns,
                )?);
            }
        }
        deadline.check()?;
        Ok(LexicalCandidates {
            generation,
            snapshot_sha256,
            channels,
        })
    }
}

fn candidates(
    tx: &Transaction<'_>,
    query: &LexicalQuery,
    channel: LexicalChannel,
    expression: &str,
    short_patterns: &[String],
) -> AppResult<ChannelCandidates> {
    let sql = match channel {
        LexicalChannel::Word | LexicalChannel::Trigram => {
            let table = if channel == LexicalChannel::Word {
                "search_fts_word"
            } else {
                "search_fts_tri"
            };
            format!(
                "SELECT u.unit_id,u.record_type,u.record_id,u.title,u.aliases,bm25({table})
                FROM {table} JOIN search_units u ON u.rowid={table}.rowid
                WHERE {table} MATCH ?1 AND {FILTER} AND {SHORT_FILTER}
                ORDER BY bm25({table}),u.unit_id LIMIT ?4"
            )
        }
        LexicalChannel::ShortText => format!(
            "SELECT u.unit_id,u.record_type,u.record_id,u.title,u.aliases,NULL
            FROM search_units u WHERE ?1 IS NOT NULL AND {FILTER} AND {SHORT_FILTER}
            ORDER BY u.unit_id LIMIT ?4"
        ),
    };
    let types = serde_json::to_string(&query.record_types).map_err(encoding_error)?;
    let statuses = serde_json::to_string(&query.statuses).map_err(encoding_error)?;
    let patterns = serde_json::to_string(short_patterns).map_err(encoding_error)?;
    let mut statement = tx.prepare(&sql).map_err(search_error)?;
    let mut rows = statement
        .query(params![
            expression,
            types,
            statuses,
            query.candidate_limit,
            patterns
        ])
        .map_err(search_error)?;
    let mut hits = Vec::new();
    while let Some(row) = rows.next().map_err(search_error)? {
        let record_type: String = row.get(1).map_err(database_error)?;
        let aliases: String = row.get(4).map_err(database_error)?;
        hits.push(LexicalHit {
            unit_id: row.get(0).map_err(database_error)?,
            record_type: serde_json::from_value(serde_json::Value::String(record_type)).map_err(
                |_| {
                    AppError::new(
                        ErrorType::Validation,
                        "INVALID_PROJECTION_PAYLOAD",
                        "A search unit has an invalid record type.",
                    )
                },
            )?,
            record_id: row.get(2).map_err(database_error)?,
            title: row.get(3).map_err(database_error)?,
            aliases: aliases.lines().map(str::to_owned).collect(),
            rank: hits.len() as u32 + 1,
            bm25: row.get(5).map_err(database_error)?,
        });
    }
    Ok(ChannelCandidates { channel, hits })
}

fn literal_expression(terms: &[&str]) -> String {
    terms
        .iter()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn like_pattern(term: &str) -> String {
    format!(
        "%{}%",
        term.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}

fn search_error(error: rusqlite::Error) -> AppError {
    if error.sqlite_error_code() == Some(rusqlite::ErrorCode::OperationInterrupted) {
        return deadline::timeout();
    }
    if let rusqlite::Error::SqliteFailure(_, Some(message)) = &error
        && (message.starts_with("fts5: syntax error")
            || message.starts_with("unterminated string")
            || message.starts_with("no such column:")
            || message.starts_with("fts5: parse error")
            || message.starts_with("malformed MATCH expression"))
    {
        return AppError::new(
            ErrorType::Validation,
            "INVALID_SEARCH_SYNTAX",
            "The advanced FTS query is invalid.",
        )
        .with_param("query")
        .with_hint("Correct the FTS5 syntax or use query_syntax=literal.");
    }
    database_error(error)
}

fn encoding_error(_: serde_json::Error) -> AppError {
    AppError::new(
        ErrorType::Internal,
        "SEARCH_ENCODING_FAILED",
        "Search filters could not be encoded.",
    )
}
