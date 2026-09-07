use knowmesh_core::{
    application::node_read::{
        MAX_DETAIL_RELATIONS, NodeData, NodeGetQuery, NodeListData, NodeListQuery,
        NodeRelationSummary, NodeSelector, NodeSummary,
    },
    canonical::snapshot::NodeProjection,
    domain::{NodeId, normalize_name},
    error::{AppError, AppResult, ErrorType},
    ports::NodeReadStore,
};
use rusqlite::{OptionalExtension, Transaction, params};

use crate::{SqliteStore, database_error};

const FILTER: &str = "(?1 IS NULL OR node_type=?1)
    AND (?2 IS NULL OR EXISTS(SELECT 1 FROM json_each(tags_json) WHERE value=?2))
    AND (?3 IS NULL OR lifecycle_status=?3)";

/// Upper bound on the nodes inspected when resolving a name or alias before
/// the read is declared ambiguous instead of unbounded.
const MAX_NAME_CANDIDATES: usize = 50;

/// Upper bound on the ambiguity candidates reported in error details.
const REPORTED_CANDIDATES: usize = 8;

impl NodeReadStore for SqliteStore {
    fn node_get(&self, query: &NodeGetQuery) -> AppResult<NodeData> {
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(database_error)?;
        let (generation, snapshot_sha256) = state(&tx)?;
        match &query.selector {
            NodeSelector::Id(id) => {
                let json: Option<String> = tx
                    .query_row(
                        "SELECT canonical_json FROM nodes WHERE id=?1",
                        [id.as_str()],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(database_error)?;
                let node = projection(json.ok_or_else(not_found)?)?;
                let (relations, relations_total) = relations(&tx, id)?;
                Ok(NodeData {
                    generation,
                    snapshot_sha256,
                    node,
                    relations,
                    relations_total,
                })
            }
            NodeSelector::Name(name) => {
                let normalized = normalize_name(name);
                let mut candidates = name_matches(&tx, "n.canonical_json", normalized.as_str())?;
                match candidates.len() {
                    0 => Err(not_found()),
                    1 => {
                        let node = projection(candidates.remove(0))?;
                        let (relations, relations_total) = relations(&tx, &node.metadata.id)?;
                        Ok(NodeData {
                            generation,
                            snapshot_sha256,
                            node,
                            relations,
                            relations_total,
                        })
                    }
                    _ => Err(ambiguous(&tx, &normalized)?),
                }
            }
        }
    }

    fn node_list(&self, query: &NodeListQuery) -> AppResult<NodeListData> {
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(database_error)?;
        let (generation, snapshot_sha256) = state(&tx)?;
        if let Some(position) = &query.position
            && (position.generation != generation || position.snapshot_sha256 != snapshot_sha256)
        {
            return Err(stale());
        }
        let total = tx
            .query_row(
                &format!("SELECT count(*) FROM nodes WHERE {FILTER}"),
                params![query.node_type, query.tag, query.status],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        let after = query
            .position
            .as_ref()
            .map(|position| position.after.as_str())
            .unwrap_or("");
        let mut statement = tx
            .prepare(&format!(
                "SELECT canonical_json FROM nodes WHERE {FILTER} AND id>?4 ORDER BY id LIMIT ?5"
            ))
            .map_err(database_error)?;
        let rows = statement
            .query_map(
                params![
                    query.node_type,
                    query.tag,
                    query.status,
                    after,
                    u64::from(query.limit) + 1
                ],
                |row| row.get::<_, String>(0),
            )
            .map_err(database_error)?;
        let mut items = Vec::new();
        for row in rows {
            let node = projection(row.map_err(database_error)?)?;
            items.push(NodeSummary {
                id: node.metadata.id,
                name: node.metadata.name,
                node_type: node.metadata.node_type,
                aliases: node.metadata.aliases,
                tags: node.metadata.tags,
                lifecycle_status: node.metadata.lifecycle_status,
                updated_at: node.metadata.updated_at,
            });
        }
        let has_more = items.len() > query.limit as usize;
        items.truncate(query.limit as usize);
        Ok(NodeListData {
            generation,
            snapshot_sha256,
            total,
            items,
            has_more,
        })
    }
}

/// Selects the distinct nodes whose canonical name or an alias normalizes to
/// the requested name; `column` picks the projection payload or candidate fields.
fn name_matches(tx: &Transaction<'_>, column: &str, normalized: &str) -> AppResult<Vec<String>> {
    let sql = format!(
        "SELECT {column} FROM nodes n WHERE n.normalized_name=?1
        UNION
        SELECT {column} FROM node_aliases a JOIN nodes n ON n.id=a.node_id
            WHERE a.normalized_alias=?1 ORDER BY 1 LIMIT ?2"
    );
    let mut statement = tx.prepare(&sql).map_err(database_error)?;
    let rows = statement
        .query_map(params![normalized, MAX_NAME_CANDIDATES as i64], |row| {
            row.get::<_, String>(0)
        })
        .map_err(database_error)?;
    let mut matches = Vec::new();
    for row in rows {
        matches.push(row.map_err(database_error)?);
    }
    Ok(matches)
}

fn relations(tx: &Transaction<'_>, id: &NodeId) -> AppResult<(Vec<NodeRelationSummary>, u64)> {
    let total: u64 = tx
        .query_row(
            "SELECT count(*) FROM relations WHERE source_node_id=?1",
            [id.as_str()],
            |row| row.get(0),
        )
        .map_err(database_error)?;
    let mut statement = tx
        .prepare(
            "SELECT id,predicate,target_node_id,directed,lifecycle_status,evidence_status,confidence
            FROM relations WHERE source_node_id=?1 ORDER BY id LIMIT ?2",
        )
        .map_err(database_error)?;
    let rows = statement
        .query_map(params![id.as_str(), MAX_DETAIL_RELATIONS as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, bool>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<f64>>(6)?,
            ))
        })
        .map_err(database_error)?;
    let mut relations = Vec::new();
    for row in rows {
        let (id, predicate, target, directed, lifecycle, evidence, confidence) =
            row.map_err(database_error)?;
        relations.push(NodeRelationSummary {
            id: id.parse().map_err(|_| invalid_projection())?,
            predicate,
            target_node_id: target.parse().map_err(|_| invalid_projection())?,
            directed,
            lifecycle_status: enum_value(&lifecycle)?,
            evidence_status: enum_value(&evidence)?,
            confidence,
        });
    }
    Ok((relations, total))
}

fn ambiguous(tx: &Transaction<'_>, normalized: &str) -> AppResult<AppError> {
    let rows = name_matches(
        tx,
        "json_object('id',n.id,'type',n.node_type,'name',n.canonical_name) AS candidate",
        normalized,
    )?;
    let mut candidates = Vec::new();
    for row in rows.iter().take(REPORTED_CANDIDATES) {
        candidates.push(
            serde_json::from_str::<serde_json::Value>(row).map_err(|_| invalid_projection())?,
        );
    }
    Ok(AppError::new(
        ErrorType::Validation,
        "AMBIGUOUS_NODE_NAME",
        "The name matches more than one knowledge node.",
    )
    .with_param("node")
    .with_details(serde_json::json!({ "candidates": candidates }))
    .with_hint("Read one of the candidate IDs with `knowmesh node get <node-id>`."))
}

fn state(tx: &Transaction<'_>) -> AppResult<(u64, String)> {
    tx.query_row(
        "SELECT indexed_generation,snapshot_sha256 FROM workspace_state WHERE singleton=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map_err(database_error)
}

fn projection(json: String) -> AppResult<NodeProjection> {
    serde_json::from_str(&json).map_err(|_| invalid_projection())
}

fn enum_value<T: serde::de::DeserializeOwned>(value: &str) -> AppResult<T> {
    serde_json::from_value(serde_json::Value::String(value.to_owned()))
        .map_err(|_| invalid_projection())
}

fn not_found() -> AppError {
    AppError::new(
        ErrorType::NotFound,
        "NODE_NOT_FOUND",
        "No knowledge node matches this ID or name.",
    )
    .with_param("node")
    .with_hint("Run `knowmesh node list` to browse available nodes.")
}

fn stale() -> AppError {
    AppError::new(
        ErrorType::Conflict,
        "CURSOR_STALE",
        "The index changed after the previous node page.",
    )
    .with_hint("Restart the node list without a cursor.")
}

fn invalid_projection() -> AppError {
    AppError::new(
        ErrorType::Validation,
        "INVALID_PROJECTION_PAYLOAD",
        "A node projection is invalid.",
    )
}
