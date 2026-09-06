use std::time::Instant;

use knowmesh_core::{
    application::{
        entity_resolution::{EntityBatchData, EntityBatchQuery},
        lexical::{
            ChannelCandidates, LexicalCandidates, LexicalChannel, LexicalHit, LexicalQuery,
            RecordType,
        },
    },
    canonical::snapshot::NodeProjection,
    error::{AppError, AppResult, ErrorType},
    ports::EntityResolutionStore,
};
use rusqlite::{Transaction, params};

use crate::{
    SqliteStore, database_error,
    lexical::{deadline::Deadline, like_pattern, literal_expression},
};

impl EntityResolutionStore for SqliteStore {
    fn entity_resolution_data(&self, query: &EntityBatchQuery) -> AppResult<EntityBatchData> {
        query.validate()?;
        let started = Instant::now();
        let deadline = Deadline::new(&self.connection, query.timeout_ms)?;
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(database_error)?;
        let catalog = read_catalog(&tx, query.max_catalog_nodes);
        deadline.check()?;
        let mut data = catalog?;
        drop(deadline);
        for query_part in &query.queries {
            let remaining = query
                .timeout_ms
                .saturating_sub(started.elapsed().as_millis() as u64);
            if remaining == 0 {
                return Err(timeout());
            }
            let deadline = Deadline::new(&self.connection, query_part.timeout_ms.min(remaining))?;
            let channels = read_channels(&tx, query_part);
            deadline.check()?;
            data.lexical.push(LexicalCandidates {
                generation: data.generation,
                snapshot_sha256: data.snapshot_sha256.clone(),
                channels: channels?,
            });
        }
        if started.elapsed().as_millis() > u128::from(query.timeout_ms) {
            return Err(timeout());
        }
        Ok(data)
    }
}

fn read_catalog(tx: &Transaction<'_>, max_nodes: usize) -> AppResult<EntityBatchData> {
    let (workspace, schema_hash, generation, snapshot_sha256): (String, String, u64, String) = tx.query_row(
        "SELECT workspace_id,schema_hash,indexed_generation,snapshot_sha256 FROM workspace_state WHERE singleton=1", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).map_err(database_error)?;
    let count: usize = tx
        .query_row("SELECT count(*) FROM nodes", [], |row| row.get(0))
        .map_err(database_error)?;
    if count > max_nodes {
        return Err(AppError::new(
            ErrorType::Validation,
            "ENTITY_CATALOG_LIMIT",
            "The complete entity catalog exceeds its node budget.",
        ));
    }
    let mut statement = tx
        .prepare("SELECT canonical_json FROM nodes ORDER BY id")
        .map_err(database_error)?;
    let mut nodes = Vec::with_capacity(count);
    let mut total_bytes = 0usize;
    for row in statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(database_error)?
    {
        let text = row.map_err(database_error)?;
        total_bytes = total_bytes.saturating_add(text.len());
        if total_bytes > 64 * 1024 * 1024 {
            return Err(AppError::new(
                ErrorType::Validation,
                "ENTITY_CATALOG_LIMIT",
                "The serialized entity catalog exceeds 64 MiB.",
            ));
        }
        let node: NodeProjection = serde_json::from_str(&text).map_err(|_| invalid())?;
        nodes.push(node.metadata);
    }
    Ok(EntityBatchData {
        workspace_id: workspace.parse()?,
        schema_hash,
        generation,
        snapshot_sha256,
        nodes,
        lexical: vec![],
    })
}

fn read_channels(tx: &Transaction<'_>, query: &LexicalQuery) -> AppResult<Vec<ChannelCandidates>> {
    let terms: Vec<_> = query.query.split_whitespace().collect();
    let mut channels = vec![read_channel(tx, query, LexicalChannel::Word, &terms, &[])?];
    let (long, short): (Vec<_>, Vec<_>) = terms
        .into_iter()
        .partition(|term| term.chars().count() >= 3);
    let patterns: Vec<_> = short.into_iter().map(like_pattern).collect();
    channels.push(read_channel(
        tx,
        query,
        if long.is_empty() {
            LexicalChannel::ShortText
        } else {
            LexicalChannel::Trigram
        },
        &long,
        &patterns,
    )?);
    Ok(channels)
}

fn read_channel(
    tx: &Transaction<'_>,
    query: &LexicalQuery,
    channel: LexicalChannel,
    terms: &[&str],
    patterns: &[String],
) -> AppResult<ChannelCandidates> {
    let hits = read_hits(tx, query, channel, terms, patterns);
    match hits {
        Ok(hits) => Ok(ChannelCandidates {
            channel,
            hits,
            unavailable_reason: None,
        }),
        Err(error) if error.error_type == ErrorType::Io => Ok(ChannelCandidates {
            channel,
            hits: vec![],
            unavailable_reason: Some(
                match channel {
                    LexicalChannel::Word => "ENTITY_WORD_FTS_UNAVAILABLE",
                    LexicalChannel::Trigram => "ENTITY_TRIGRAM_FTS_UNAVAILABLE",
                    LexicalChannel::ShortText => "ENTITY_SHORT_TEXT_UNAVAILABLE",
                }
                .into(),
            ),
        }),
        Err(error) => Err(error),
    }
}

fn read_hits(
    tx: &Transaction<'_>,
    query: &LexicalQuery,
    channel: LexicalChannel,
    terms: &[&str],
    patterns: &[String],
) -> AppResult<Vec<LexicalHit>> {
    const FILTER: &str = r"n.node_type=?2 AND n.lifecycle_status='active'
        AND NOT EXISTS(SELECT 1 FROM json_each(?3) AS term
            WHERE NOT (n.canonical_name LIKE term.value ESCAPE '\'
                OR EXISTS(SELECT 1 FROM json_each(json_extract(n.canonical_json,'$.aliases')) AS alias
                    WHERE alias.value LIKE term.value ESCAPE '\')))";
    let sql = match channel {
        LexicalChannel::Word | LexicalChannel::Trigram => {
            let table = if channel == LexicalChannel::Word {
                "search_fts_word"
            } else {
                "search_fts_tri"
            };
            format!(
                "SELECT n.canonical_json,bm25({table}) FROM {table}
                JOIN search_units u ON u.rowid={table}.rowid JOIN nodes n ON n.id=u.record_id
                WHERE {table} MATCH ?1 AND u.record_type='node' AND {FILTER}
                ORDER BY bm25({table}),n.id LIMIT ?4"
            )
        }
        LexicalChannel::ShortText => format!(
            "SELECT n.canonical_json,NULL FROM nodes n WHERE ?1 IS NOT NULL AND {FILTER} ORDER BY n.id LIMIT ?4"
        ),
    };
    let expression = format!("{{title aliases}} : ({})", literal_expression(terms));
    let patterns = serde_json::to_string(patterns).map_err(|_| invalid())?;
    let mut statement = tx.prepare(&sql).map_err(database_error)?;
    let mut hits = vec![];
    for row in statement
        .query_map(
            params![
                expression,
                &query.node_types[0],
                patterns,
                query.candidate_limit
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<f64>>(1)?)),
        )
        .map_err(database_error)?
    {
        let (text, bm25) = row.map_err(database_error)?;
        let node: NodeProjection = serde_json::from_str(&text).map_err(|_| invalid())?;
        hits.push(LexicalHit {
            unit_id: format!("node:{}", node.metadata.id),
            record_type: RecordType::Node,
            record_id: node.metadata.id.to_string(),
            title: node.metadata.name,
            aliases: node.metadata.aliases,
            preview: String::new(),
            rank: hits.len() as u32 + 1,
            bm25,
        });
    }
    Ok(hits)
}

fn invalid() -> AppError {
    AppError::new(
        ErrorType::Validation,
        "INVALID_PROJECTION_PAYLOAD",
        "Entity projection metadata is invalid.",
    )
}

fn timeout() -> AppError {
    AppError::new(
        ErrorType::Validation,
        "SEARCH_TIMEOUT",
        "Entity retrieval exceeded its execution budget.",
    )
}
