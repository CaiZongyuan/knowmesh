pub(crate) mod deadline;

use knowmesh_core::{
    application::lexical::{
        ChannelCandidates, LexicalCandidates, LexicalChannel, LexicalHit, LexicalQuery, QuerySyntax,
    },
    error::{AppError, AppResult, ErrorType},
    ports::LexicalSearchStore,
};
use rusqlite::{Transaction, params_from_iter, types::Value};

use crate::{SqliteStore, database_error};
use deadline::Deadline;

const FILTER: &str =
    "(json_array_length(?2)=0 OR u.record_type IN (SELECT value FROM json_each(?2)))
    AND (json_array_length(?3)=0 OR u.lifecycle_status IN (SELECT value FROM json_each(?3)))";
const OWNERS: &str = include_str!("lexical/owners.sql");
const ALIASES: &str = "COALESCE((SELECT json_extract(n.canonical_json,'$.aliases')
    FROM nodes n WHERE u.record_type='node' AND n.id=u.record_id),'[]')";
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
        let result = read_candidates(&tx, query)?;
        deadline.check()?;
        Ok(result)
    }
}

pub(crate) fn read_candidates(
    tx: &Transaction<'_>,
    query: &LexicalQuery,
) -> AppResult<LexicalCandidates> {
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
        tx,
        query,
        LexicalChannel::Word,
        &word_expression,
        &[],
    )?);
    match query.query_syntax {
        QuerySyntax::Advanced => {
            channels.push(candidates(
                tx,
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
                tx,
                query,
                channel,
                &literal_expression(&long),
                &patterns,
            )?);
        }
    }
    Ok(LexicalCandidates {
        generation,
        snapshot_sha256,
        channels,
    })
}

pub(crate) fn exact_matches(
    tx: &Transaction<'_>,
    query: &LexicalQuery,
) -> AppResult<Vec<LexicalHit>> {
    let (owners, filter) = filters(query);
    let sql = format!("{owners} SELECT u.unit_id,u.record_type,u.record_id,u.title,{ALIASES},NULL,substr(u.body,1,512)
        FROM search_units u WHERE u.record_id=?1 AND {filter} AND {SHORT_FILTER} ORDER BY u.unit_id LIMIT ?4");
    read_hits(tx, &sql, query, query.query.trim(), &[])
}

fn candidates(
    tx: &Transaction<'_>,
    query: &LexicalQuery,
    channel: LexicalChannel,
    expression: &str,
    short_patterns: &[String],
) -> AppResult<ChannelCandidates> {
    let (owners, filter) = filters(query);
    let sql = match channel {
        LexicalChannel::Word | LexicalChannel::Trigram => {
            let table = if channel == LexicalChannel::Word {
                "search_fts_word"
            } else {
                "search_fts_tri"
            };
            format!(
                "{owners} SELECT u.unit_id,u.record_type,u.record_id,u.title,{ALIASES},bm25({table}),substr(u.body,1,512)
                FROM {table} JOIN search_units u ON u.rowid={table}.rowid
                WHERE {table} MATCH ?1 AND {filter} AND {SHORT_FILTER}
                ORDER BY bm25({table}),u.unit_id LIMIT ?4"
            )
        }
        LexicalChannel::ShortText => format!(
            "{owners} SELECT u.unit_id,u.record_type,u.record_id,u.title,{ALIASES},NULL,substr(u.body,1,512)
            FROM search_units u WHERE ?1 IS NOT NULL AND {filter} AND {SHORT_FILTER}
            ORDER BY u.unit_id LIMIT ?4"
        ),
    };
    match read_hits(tx, &sql, query, expression, short_patterns) {
        Ok(hits) => Ok(ChannelCandidates {
            channel,
            hits,
            unavailable_reason: None,
        }),
        Err(error) if error.error_type == ErrorType::Io => Ok(ChannelCandidates {
            channel,
            hits: vec![],
            unavailable_reason: Some(error.code),
        }),
        Err(error) => Err(error),
    }
}

fn read_hits(
    tx: &Transaction<'_>,
    sql: &str,
    query: &LexicalQuery,
    expression: &str,
    short_patterns: &[String],
) -> AppResult<Vec<LexicalHit>> {
    let types = serde_json::to_string(&query.record_types).map_err(encoding_error)?;
    let statuses = serde_json::to_string(&query.statuses).map_err(encoding_error)?;
    let patterns = serde_json::to_string(short_patterns).map_err(encoding_error)?;
    let node_types = serde_json::to_string(&query.node_types).map_err(encoding_error)?;
    let source_ids = serde_json::to_string(&query.source_ids).map_err(encoding_error)?;
    let tags = serde_json::to_string(&query.tags).map_err(encoding_error)?;
    let mut statement = tx.prepare(sql).map_err(search_error)?;
    let values: [Value; 8] = [
        expression.to_owned().into(),
        types.into(),
        statuses.into(),
        i64::from(query.candidate_limit).into(),
        patterns.into(),
        node_types.into(),
        source_ids.into(),
        tags.into(),
    ];
    let parameter_count = statement.parameter_count();
    let mut rows = statement
        .query(params_from_iter(values.iter().take(parameter_count)))
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
            aliases: serde_json::from_str(&aliases).map_err(|_| {
                AppError::new(
                    ErrorType::Validation,
                    "INVALID_PROJECTION_PAYLOAD",
                    "A search unit has invalid canonical aliases.",
                )
            })?,
            preview: row.get(6).map_err(database_error)?,
            rank: hits.len() as u32 + 1,
            bm25: row.get(5).map_err(database_error)?,
        });
    }
    Ok(hits)
}

fn literal_expression(terms: &[&str]) -> String {
    terms
        .iter()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn filters(query: &LexicalQuery) -> (&'static str, String) {
    let mut filter = FILTER.to_owned();
    if !query.node_types.is_empty() {
        filter.push_str(" AND EXISTS(SELECT 1 FROM search_nodes sn JOIN nodes n ON n.id=sn.node_id WHERE sn.unit_id=u.unit_id AND n.node_type IN (SELECT value FROM json_each(?6)))");
    }
    if !query.source_ids.is_empty() {
        filter.push_str(" AND EXISTS(SELECT 1 FROM search_sources ss WHERE ss.unit_id=u.unit_id AND ss.source_id IN (SELECT value FROM json_each(?7)))");
    }
    if !query.tags.is_empty() {
        filter.push_str(" AND NOT EXISTS(SELECT 1 FROM json_each(?8) AS requested WHERE NOT EXISTS(SELECT 1 FROM search_tags st WHERE st.unit_id=u.unit_id AND st.tag=requested.value))");
    }
    let owners = if filter.len() == FILTER.len() {
        ""
    } else {
        OWNERS
    };
    (owners, filter)
}

fn like_pattern(term: &str) -> String {
    format!(
        "%{}%",
        term.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}

pub(crate) fn search_error(error: rusqlite::Error) -> AppError {
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
