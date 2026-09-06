use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Serialize;

use super::{
    EntityInput, EntityResolver, ResolutionCandidate, ResolutionOptions, ResolutionReport, error,
    validate_names,
};
use crate::{
    application::{
        lexical::{LexicalCandidates, LexicalChannel, LexicalQuery, QuerySyntax, RecordType},
        search::ranking::{self, Channel, ChannelInput, RankingConfig},
        status::{self, StatusInput},
    },
    canonical::{schema::Schema, workspace::Workspace},
    domain::{NodeId, NodeMetadata, WorkspaceId, valid_sha256},
    error::{AppError, AppResult, ErrorType},
    ports::EntityResolutionStore,
};

pub struct EntityBatchQuery {
    pub queries: Vec<LexicalQuery>,
    pub max_catalog_nodes: usize,
    pub timeout_ms: u64,
}

impl EntityBatchQuery {
    pub fn validate(&self) -> AppResult<()> {
        if !(1..=64).contains(&self.queries.len())
            || !(1..=100_000).contains(&self.max_catalog_nodes)
            || !(1..=5000).contains(&self.timeout_ms)
        {
            return Err(error(
                "INVALID_ENTITY_BATCH",
                "Entity batches require 1..=64 bounded literal queries and a complete bounded catalog.",
            ));
        }
        for query in &self.queries {
            query.validate()?;
            if query.query_syntax != QuerySyntax::Literal
                || query.record_types != [RecordType::Node]
                || query.node_types.len() != 1
                || query.statuses != ["active"]
                || !query.source_ids.is_empty()
                || !query.tags.is_empty()
            {
                return Err(error(
                    "INVALID_ENTITY_BATCH",
                    "Entity retrieval supports literal active-Node title/alias queries.",
                ));
            }
        }
        Ok(())
    }
}

pub struct EntityBatchData {
    pub workspace_id: WorkspaceId,
    pub schema_hash: String,
    pub generation: u64,
    pub snapshot_sha256: String,
    pub nodes: Vec<NodeMetadata>,
    pub lexical: Vec<LexicalCandidates>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct EntityBatchReport {
    pub workspace_id: WorkspaceId,
    pub generation: u64,
    pub snapshot_sha256: String,
    pub results: Vec<ResolutionReport>,
}

pub fn resolve_batch(
    workspace: &Workspace,
    store: &mut dyn EntityResolutionStore,
    entities: &[EntityInput],
    options: &ResolutionOptions,
) -> AppResult<EntityBatchReport> {
    let schema = Schema::load(workspace)?;
    if !(1..=64).contains(&entities.len()) {
        return Err(error(
            "INVALID_ENTITY_BATCH",
            "Entity batches require 1..=64 entities.",
        ));
    }
    EntityResolver::new(&schema, &[], options.clone())?;
    let mut queries = vec![];
    for entity in entities {
        validate_names(&entity.name, &entity.aliases)?;
        schema.entity_identifiers(&entity.node_type, &entity.properties)?;
        queries.push(LexicalQuery {
            query: entity.name.clone(),
            record_types: vec![RecordType::Node],
            node_types: vec![entity.node_type.clone()],
            candidate_limit: workspace.config.search.candidate_limit.max(2),
            timeout_ms: workspace.config.search.lexical_timeout_ms,
            ..Default::default()
        });
    }
    let query = EntityBatchQuery {
        queries,
        max_catalog_nodes: options.max_catalog_nodes,
        timeout_ms: 5000,
    };
    query.validate()?;
    let status = status::get(workspace, store, &StatusInput { no_sync: false })?;
    let data = store.entity_resolution_data(&query)?;
    if data.workspace_id != workspace.config.workspace.id || data.schema_hash != schema.hash {
        return Err(error(
            "ENTITY_CONTEXT_MISMATCH",
            "The entity catalog belongs to a different workspace or Schema.",
        ));
    }
    if status.sync_skipped.is_some()
        || status.recovery_required
        || status.projection.generation != data.generation
        || !valid_sha256(&data.snapshot_sha256)
    {
        return Err(AppError::new(
            ErrorType::Conflict,
            "ENTITY_INDEX_INCOMPLETE",
            "Entity resolution requires a complete synchronized index snapshot.",
        ));
    }
    if data.lexical.len() != entities.len()
        || data.lexical.iter().any(|lexical| {
            lexical.generation != data.generation || lexical.snapshot_sha256 != data.snapshot_sha256
        })
    {
        return Err(error(
            "ENTITY_CONTEXT_MISMATCH",
            "Entity retrieval results do not belong to the catalog snapshot.",
        ));
    }
    let resolver = EntityResolver::new(&schema, &data.nodes, options.clone())?;
    let mut ranking = RankingConfig::from(&workspace.config.search);
    ranking.boosts_enabled = false;
    let mut results = vec![];
    for ((entity, lexical), query_part) in entities.iter().zip(data.lexical).zip(&query.queries) {
        results.push(resolver.with_lexical(
            entity,
            lexical,
            &ranking,
            workspace.config.embedding.enabled,
            query_part.candidate_limit,
        )?);
    }
    Ok(EntityBatchReport {
        workspace_id: data.workspace_id,
        generation: data.generation,
        snapshot_sha256: data.snapshot_sha256,
        results,
    })
}

impl EntityResolver<'_> {
    fn with_lexical(
        &self,
        entity: &EntityInput,
        lexical: LexicalCandidates,
        ranking: &RankingConfig,
        vector_enabled: bool,
        candidate_limit: u32,
    ) -> AppResult<ResolutionReport> {
        let limit_reached = lexical
            .channels
            .iter()
            .any(|channel| channel.hits.len() >= candidate_limit as usize);
        let mut candidates: BTreeMap<_, _> = self
            .matching_candidates(entity)?
            .into_iter()
            .map(|candidate| (candidate.node_id.clone(), candidate))
            .collect();
        let mut channels: Vec<_> = lexical
            .channels
            .into_iter()
            .map(|part| ChannelInput {
                channel: match part.channel {
                    LexicalChannel::Word => Channel::Word,
                    LexicalChannel::Trigram => Channel::Trigram,
                    LexicalChannel::ShortText => Channel::ShortText,
                },
                hits: part.hits,
                unavailable_reason: part.unavailable_reason,
            })
            .collect();
        channels.push(ChannelInput {
            channel: Channel::Vector,
            hits: vec![],
            unavailable_reason: Some(
                if vector_enabled {
                    "VECTOR_UNAVAILABLE"
                } else {
                    "VECTOR_DISABLED"
                }
                .into(),
            ),
        });
        let ranked = ranking::fuse(&entity.name, ranking, &channels, &[])?;
        let retrieval_sha256 = super::hash(&(
            super::RESOLVER_VERSION,
            lexical.generation,
            &lexical.snapshot_sha256,
            ranking,
            candidate_limit,
            &ranked,
        ))?;
        let identifiers = self
            .schema
            .entity_identifiers(&entity.node_type, &entity.properties)?;
        for hit in ranked.hits {
            let id: NodeId = hit.candidate.record_id.parse()?;
            let index = self
                .nodes
                .binary_search_by(|node| node.metadata.id.cmp(&id))
                .map_err(|_| {
                    error(
                        "ENTITY_CONTEXT_MISMATCH",
                        "A retrieved Node is absent from the active catalog.",
                    )
                })?;
            let indexed = &self.nodes[index];
            let node = indexed.metadata;
            if hit.candidate.record_type != RecordType::Node
                || hit.candidate.title != node.name
                || hit.candidate.aliases != node.aliases
            {
                return Err(error(
                    "ENTITY_CONTEXT_MISMATCH",
                    "Retrieved Node metadata differs from the entity catalog.",
                ));
            }
            let candidate = candidates.entry(id).or_insert_with(|| {
                let mut warnings = vec![];
                if identifiers.iter().any(|(key, value)| {
                    indexed
                        .identifiers
                        .get(key)
                        .is_some_and(|other| other != value)
                }) {
                    warnings.push("ENTITY_IDENTIFIER_CONFLICT".into());
                }
                if node.node_type != entity.node_type {
                    warnings.push("ENTITY_TYPE_MISMATCH".into());
                }
                ResolutionCandidate {
                    node_id: node.id.clone(),
                    name: node.name.clone(),
                    node_type: node.node_type.clone(),
                    matched_by: vec![],
                    retrieval_score: None,
                    warnings,
                }
            });
            candidate.matched_by.push("fts".into());
            candidate.matched_by.sort();
            candidate.retrieval_score = Some(hit.explain.normalized_score);
        }
        let mut report = self.report(entity, candidates.into_values().collect())?;
        report.retrieval_available = ranked.channels.iter().any(|channel| {
            channel.channel != Channel::Vector && channel.unavailable_reason.is_none()
        });
        report.retrieval_sha256 = Some(retrieval_sha256);
        if limit_reached {
            report
                .warnings
                .push("ENTITY_RETRIEVAL_LIMIT_REACHED".into());
            if report
                .candidates
                .iter()
                .all(|candidate| super::strength(candidate) == 3)
            {
                report.decision = super::ResolutionDecision::Ambiguous;
                report.selected_node_id = None;
            }
        }
        report.warnings.extend(
            ranked
                .channels
                .into_iter()
                .filter_map(|channel| channel.unavailable_reason),
        );
        report.warnings.sort();
        report.warnings.dedup();
        Ok(report)
    }
}
