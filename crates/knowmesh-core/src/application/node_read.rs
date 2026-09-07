mod cursor;

use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    canonical::{snapshot::NodeProjection, workspace::Workspace},
    domain::{EvidenceStatus, LifecycleStatus, NodeId, RelationId, Timestamp},
    error::{AppError, AppResult, ErrorType},
    ports::NodeReadStore,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeGetInput {
    pub node: String,
    #[serde(default)]
    pub no_sync: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct NodeListInput {
    pub node_type: Option<String>,
    pub tag: Option<String>,
    pub status: Option<String>,
    pub limit: u32,
    pub cursor: Option<String>,
    pub no_sync: bool,
}

impl Default for NodeListInput {
    fn default() -> Self {
        Self {
            node_type: None,
            tag: None,
            status: None,
            limit: 20,
            cursor: None,
            no_sync: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NodeSummary {
    pub id: NodeId,
    pub name: String,
    pub node_type: String,
    pub aliases: Vec<String>,
    pub tags: Vec<String>,
    pub lifecycle_status: LifecycleStatus,
    pub updated_at: Timestamp,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct NodeListReport {
    pub generation: u64,
    pub index_complete: bool,
    pub total: u64,
    pub items: Vec<NodeSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NodeRelationSummary {
    pub id: RelationId,
    pub predicate: String,
    pub target_node_id: NodeId,
    pub directed: bool,
    pub lifecycle_status: LifecycleStatus,
    pub evidence_status: EvidenceStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct NodeDetail {
    #[serde(flatten)]
    pub node: NodeProjection,
    pub relations: Vec<NodeRelationSummary>,
    pub relations_total: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct NodeReport {
    pub generation: u64,
    pub index_complete: bool,
    pub node: NodeDetail,
}

#[derive(Debug, Clone)]
pub enum NodeSelector {
    Id(NodeId),
    Name(String),
}

#[derive(Debug)]
pub struct NodeGetQuery {
    pub selector: NodeSelector,
}

#[derive(Debug)]
pub struct NodeData {
    pub generation: u64,
    pub snapshot_sha256: String,
    pub node: NodeProjection,
    pub relations: Vec<NodeRelationSummary>,
    pub relations_total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeListPosition {
    pub generation: u64,
    pub snapshot_sha256: String,
    pub after: NodeId,
}

#[derive(Debug)]
pub struct NodeListQuery {
    pub node_type: Option<String>,
    pub tag: Option<String>,
    pub status: Option<String>,
    pub limit: u32,
    pub position: Option<NodeListPosition>,
}

#[derive(Debug)]
pub struct NodeListData {
    pub generation: u64,
    pub snapshot_sha256: String,
    pub total: u64,
    pub items: Vec<NodeSummary>,
    pub has_more: bool,
}

/// The association bound reported by `node.get`; larger Node documents keep the
/// remainder available through `relations_total`.
pub const MAX_DETAIL_RELATIONS: usize = 100;

pub fn get(
    workspace: &Workspace,
    store: &mut dyn NodeReadStore,
    input: &NodeGetInput,
) -> AppResult<NodeReport> {
    let selector = resolve(&input.node)?;
    let status = super::status::get(
        workspace,
        store,
        &super::status::StatusInput {
            no_sync: input.no_sync,
        },
    )?;
    let data = store.node_get(&NodeGetQuery { selector })?;
    data.node.metadata.validate()?;
    let index_complete = complete(&status, data.generation, &data.snapshot_sha256);
    Ok(NodeReport {
        generation: data.generation,
        index_complete,
        node: NodeDetail {
            node: data.node,
            relations: data.relations,
            relations_total: data.relations_total,
        },
    })
}

pub fn list(
    workspace: &Workspace,
    store: &mut dyn NodeReadStore,
    input: &NodeListInput,
) -> AppResult<NodeListReport> {
    if !(1..=100).contains(&input.limit) {
        return Err(AppError::new(
            ErrorType::Validation,
            "INVALID_PAGE_LIMIT",
            "The page limit must be between 1 and 100.",
        )
        .with_param("limit"));
    }
    if let Some(status) = &input.status
        && !matches!(status.as_str(), "active" | "superseded" | "retracted")
    {
        return Err(AppError::new(
            ErrorType::Validation,
            "INVALID_NODE_STATUS",
            "The lifecycle status filter must be active, superseded, or retracted.",
        )
        .with_param("status"));
    }
    let fingerprint = cursor::fingerprint(workspace, input)?;
    let position = input
        .cursor
        .as_deref()
        .map(|value| cursor::decode(value, &fingerprint))
        .transpose()?;
    let status = super::status::get(
        workspace,
        store,
        &super::status::StatusInput {
            no_sync: input.no_sync,
        },
    )?;
    let data = store.node_list(&NodeListQuery {
        node_type: input.node_type.clone(),
        tag: input.tag.clone(),
        status: input.status.clone(),
        limit: input.limit,
        position,
    })?;
    let index_complete = complete(&status, data.generation, &data.snapshot_sha256);
    let next_cursor = if data.has_more {
        data.items
            .last()
            .map(|last| {
                cursor::encode(
                    &fingerprint,
                    NodeListPosition {
                        generation: data.generation,
                        snapshot_sha256: data.snapshot_sha256.clone(),
                        after: last.id.clone(),
                    },
                )
            })
            .transpose()?
    } else {
        None
    };
    Ok(NodeListReport {
        generation: data.generation,
        index_complete,
        total: data.total,
        items: data.items,
        next_cursor,
    })
}

fn resolve(value: &str) -> AppResult<NodeSelector> {
    let text = value.trim();
    if text.is_empty() {
        return Err(invalid_node_id("A Node reference is required."));
    }
    if let Some(rest) = text.strip_prefix("kn_") {
        if ulid_shape(rest)
            && let Ok(id) = NodeId::from_str(text)
        {
            return Ok(NodeSelector::Id(id));
        }
        return Err(invalid_node_id(
            "The Node ID is not a canonical kn_ identifier.",
        ));
    }
    if let Some((prefix, payload)) = text.split_once('_')
        && typed_id_prefix(prefix)
        && ulid_shape(payload)
    {
        return Err(invalid_node_id(
            "The reference is another object's ID, not a Node ID or name.",
        ));
    }
    if text.len() > 2048 {
        return Err(invalid_node_id(
            "The Node name exceeds its 2048 byte reference limit.",
        ));
    }
    Ok(NodeSelector::Name(text.to_owned()))
}

fn typed_id_prefix(prefix: &str) -> bool {
    (2..=5).contains(&prefix.len()) && prefix.bytes().all(|b| b.is_ascii_lowercase())
}

/// Canonical ULID text: 26 Crockford base32 characters whose leading digit is 0-7.
fn ulid_shape(payload: &str) -> bool {
    payload.len() == 26
        && matches!(payload.as_bytes()[0], b'0'..=b'7')
        && payload
            .bytes()
            .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase() && !b"ILOU".contains(&c))
}

fn invalid_node_id(message: &str) -> AppError {
    AppError::new(ErrorType::Validation, "INVALID_NODE_ID", message)
        .with_param("node")
        .with_hint(
            "Pass a kn_ Node ID or a Node name; `knowmesh node list` shows valid references.",
        )
}

fn complete(status: &super::status::StatusReport, generation: u64, hash: &str) -> bool {
    status.sync_skipped.is_none()
        && !status.recovery_required
        && status.projection.generation == generation
        && !hash.is_empty()
}
