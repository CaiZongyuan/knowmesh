pub mod pagination;
pub mod ranking;
mod types;

pub use types::*;

use crate::{
    application::{
        lexical::{LexicalChannel, LexicalQuery, QuerySyntax, RecordType},
        status::{self, StatusInput},
    },
    canonical::workspace::Workspace,
    domain::{
        freshness::{assertion_freshness, synthesis_freshness},
        normalize_name, sha256,
    },
    error::{AppError, AppResult, ErrorType},
    ports::SearchStore,
};
use pagination::{PageContext, PageInput, paginate};
use ranking::{Channel, ChannelInput, RankingConfig};

pub fn execute(
    workspace: &Workspace,
    store: &mut dyn SearchStore,
    input: &SearchInput,
) -> AppResult<SearchReport> {
    let settings = &workspace.config.search;
    let limit = input.limit.unwrap_or(settings.default_limit as u32);
    if !(1..=100).contains(&limit) {
        return Err(AppError::new(
            ErrorType::Validation,
            "INVALID_PAGE_LIMIT",
            "The page limit must be between 1 and 100.",
        )
        .with_param("limit"));
    }
    let mut query = LexicalQuery {
        query: input.query.clone(),
        query_syntax: input.query_syntax,
        record_types: input.record_types.clone(),
        node_types: input.node_types.clone(),
        source_ids: input.source_ids.clone(),
        tags: input.tags.clone(),
        statuses: input.statuses.clone(),
        candidate_limit: settings.candidate_limit,
        timeout_ms: settings.lexical_timeout_ms,
    };
    query.validate()?;
    query.query = match input.query_syntax {
        QuerySyntax::Literal => input.query.split_whitespace().collect::<Vec<_>>().join(" "),
        QuerySyntax::Advanced => input.query.trim().to_owned(),
    };
    query.record_types = ordered(&query.record_types);
    query.node_types = ordered(&query.node_types);
    query.source_ids = ordered(&query.source_ids);
    query.tags = ordered(&query.tags);
    query.statuses = ordered(&query.statuses);
    let normalized = query.query.clone();
    let fingerprint = sha256(
        &serde_json::to_vec(&(
            &query.query,
            query.query_syntax,
            &query.record_types,
            &query.node_types,
            &query.source_ids,
            &query.tags,
            &query.statuses,
        ))
        .map_err(|_| invalid_data())?,
    );
    let status = status::get(
        workspace,
        store,
        &StatusInput {
            no_sync: input.no_sync,
        },
    )?;
    let mut data = store.search_data(&query)?;
    if data.workspace_id != workspace.config.workspace.id {
        return Err(AppError::new(
            ErrorType::Configuration,
            "WORKSPACE_ID_MISMATCH",
            "The search index belongs to another workspace.",
        ));
    }
    let index_complete = status.sync_skipped.is_none()
        && !status.recovery_required
        && status.projection.generation == data.lexical.generation
        && !data.lexical.snapshot_sha256.is_empty();
    data.context.index_complete = index_complete;
    let mut channels: Vec<_> = data
        .lexical
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
            if workspace.config.embedding.enabled {
                "VECTOR_UNAVAILABLE"
            } else {
                "VECTOR_DISABLED"
            }
            .into(),
        ),
    });
    let ranking = RankingConfig::from(settings);
    let ranked = ranking::fuse(&normalized, &ranking, &channels, &data.exact_candidates)?;
    let entities = resolve_entities(&normalized, &ranked)?;
    let page = paginate(
        &ranked,
        &PageContext {
            workspace_id: data.workspace_id,
            query_sha256: fingerprint,
            generation: data.lexical.generation,
            snapshot_sha256: data.lexical.snapshot_sha256,
            ranking,
            candidate_limit: settings.candidate_limit,
        },
        &PageInput {
            limit,
            cursor: input.cursor.clone(),
        },
    )?;
    let mut groups = SearchGroups::default();
    for hit in page.hits {
        let freshness = match data.dependencies.get(&hit.candidate.unit_id) {
            Some(KnowledgeDependencies::Assertion(evidence)) => {
                Some(assertion_freshness(evidence, &data.context))
            }
            Some(KnowledgeDependencies::Synthesis {
                evidence_ids,
                snapshot,
            }) => Some(synthesis_freshness(
                evidence_ids,
                snapshot.as_ref(),
                &data.context,
            )),
            None if matches!(
                hit.candidate.record_type,
                RecordType::Node | RecordType::Claim | RecordType::Synthesis
            ) =>
            {
                return Err(invalid_data());
            }
            None => None,
        };
        let item = SearchHit {
            unit_id: hit.candidate.unit_id,
            record_type: hit.candidate.record_type,
            record_id: hit.candidate.record_id,
            title: hit.candidate.title,
            aliases: hit.candidate.aliases,
            preview: hit.candidate.preview,
            score: hit.explain.final_score,
            exact_id_tier: hit.explain.exact_id_tier,
            explain: input.explain.then_some(hit.explain),
            freshness,
        };
        match item.record_type {
            RecordType::Node => groups.knowledge.push(item),
            RecordType::Claim => groups.claims.push(item),
            RecordType::Source => groups.sources.push(item),
            RecordType::Synthesis => groups.syntheses.push(item),
            RecordType::Chunk => groups.chunks.push(item),
        }
    }
    let mut warnings = Vec::new();
    if input.include_graph_paths {
        warnings.push("GRAPH_PATHS_UNAVAILABLE".into());
    }
    if workspace.config.embedding.enabled {
        warnings.push("VECTOR_UNAVAILABLE".into());
    }
    if channels
        .iter()
        .any(|part| part.channel != Channel::Vector && part.unavailable_reason.is_some())
    {
        warnings.push("LEXICAL_SEARCH_DEGRADED".into());
    }
    Ok(SearchReport {
        generation: data.lexical.generation,
        index_complete,
        query: SearchQueryText {
            original: input.query.clone(),
            normalized,
        },
        groups,
        resolved_entities: entities,
        capabilities: SearchCapabilities {
            word_fts: channels
                .iter()
                .any(|part| part.channel == Channel::Word && part.unavailable_reason.is_none()),
            trigram_fts: channels
                .iter()
                .any(|part| part.channel == Channel::Trigram && part.unavailable_reason.is_none()),
            short_text: channels.iter().any(|part| {
                part.channel == Channel::ShortText && part.unavailable_reason.is_none()
            }),
            vector: false,
            graph_paths: false,
        },
        channels: ranked.channels,
        warnings,
        next_cursor: page.next_cursor,
    })
}

fn ordered<T: Clone + Ord>(values: &[T]) -> Vec<T> {
    let mut values = values.to_vec();
    values.sort();
    values.dedup();
    values
}

fn resolve_entities(
    query: &str,
    ranked: &ranking::RankingResult,
) -> AppResult<Vec<ResolvedEntity>> {
    let normalized = normalize_name(query);
    let mut entities = Vec::new();
    for hit in &ranked.hits {
        let candidate = &hit.candidate;
        if candidate.record_type != RecordType::Node {
            continue;
        }
        let matched_by = if hit.explain.exact_id_tier {
            EntityMatch::ExactId
        } else if normalize_name(&candidate.title) == normalized {
            EntityMatch::CanonicalName
        } else if candidate
            .aliases
            .iter()
            .any(|alias| normalize_name(alias) == normalized)
        {
            EntityMatch::Alias
        } else {
            continue;
        };
        entities.push(ResolvedEntity {
            node_id: candidate.record_id.parse()?,
            matched_by,
        });
    }
    Ok(entities)
}

fn invalid_data() -> AppError {
    AppError::new(
        ErrorType::Internal,
        "INVALID_SEARCH_DATA",
        "The search snapshot is missing required knowledge data.",
    )
}
