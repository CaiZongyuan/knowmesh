use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::ranking::{RankedHit, RankingConfig, RankingResult};
use crate::{
    domain::{WorkspaceId, sha256},
    error::{AppError, AppResult, ErrorType},
};

#[derive(Debug, Clone)]
pub struct PageContext {
    pub workspace_id: WorkspaceId,
    pub query_sha256: String,
    pub generation: u64,
    pub snapshot_sha256: String,
    pub ranking: RankingConfig,
    pub candidate_limit: u32,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageInput {
    pub limit: u32,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RankedPage {
    pub hits: Vec<RankedHit>,
    pub next_cursor: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u32,
    workspace_id: WorkspaceId,
    query_sha256: String,
    generation: u64,
    snapshot_sha256: String,
    ranking_sha256: String,
    results_sha256: String,
    position: Position,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Position {
    exact_id_tier: bool,
    score_bits: u64,
    unit_id: String,
}

impl Position {
    fn from_hit(hit: &RankedHit) -> Self {
        Self {
            exact_id_tier: hit.explain.exact_id_tier,
            score_bits: hit.explain.final_score.to_bits(),
            unit_id: hit.candidate.unit_id.clone(),
        }
    }

    fn matches(&self, hit: &RankedHit) -> bool {
        self.exact_id_tier == hit.explain.exact_id_tier
            && self.score_bits == hit.explain.final_score.to_bits()
            && self.unit_id == hit.candidate.unit_id
    }
}

pub fn paginate(
    ranked: &RankingResult,
    context: &PageContext,
    input: &PageInput,
) -> AppResult<RankedPage> {
    if !(1..=100).contains(&input.limit) {
        return Err(AppError::new(
            ErrorType::Validation,
            "INVALID_PAGE_LIMIT",
            "The page limit must be between 1 and 100.",
        )
        .with_param("limit"));
    }
    let cursor = input.cursor.as_deref().map(decode).transpose()?;
    let ranking_sha256 = digest(&(&context.ranking, context.candidate_limit))?;
    let results_sha256 = digest(ranked)?;
    let start = if let Some(cursor) = cursor {
        if cursor.workspace_id != context.workspace_id
            || cursor.query_sha256 != context.query_sha256
        {
            return Err(AppError::new(
                ErrorType::Validation,
                "CURSOR_QUERY_MISMATCH",
                "The cursor belongs to another workspace, query, or filter.",
            )
            .with_param("cursor"));
        }
        if cursor.generation != context.generation
            || cursor.snapshot_sha256 != context.snapshot_sha256
            || cursor.ranking_sha256 != ranking_sha256
            || cursor.results_sha256 != results_sha256
        {
            return Err(AppError::new(
                ErrorType::Conflict,
                "CURSOR_STALE",
                "The search index, ranking configuration, channels, or candidates changed.",
            )
            .with_param("cursor")
            .with_hint("Restart the search without a cursor."));
        }
        ranked
            .hits
            .iter()
            .position(|hit| cursor.position.matches(hit))
            .ok_or_else(invalid)?
            + 1
    } else {
        0
    };
    let end = (start + input.limit as usize).min(ranked.hits.len());
    let hits = ranked.hits[start..end].to_vec();
    let next_cursor = if end < ranked.hits.len() {
        Some(
            URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&Cursor {
                    version: 1,
                    workspace_id: context.workspace_id.clone(),
                    query_sha256: context.query_sha256.clone(),
                    generation: context.generation,
                    snapshot_sha256: context.snapshot_sha256.clone(),
                    ranking_sha256,
                    results_sha256,
                    position: Position::from_hit(hits.last().ok_or_else(invalid)?),
                })
                .map_err(|_| invalid())?,
            ),
        )
    } else {
        None
    };
    Ok(RankedPage { hits, next_cursor })
}

fn decode(value: &str) -> AppResult<Cursor> {
    if value.len() > 4096 {
        return Err(invalid());
    }
    let bytes = URL_SAFE_NO_PAD.decode(value).map_err(|_| invalid())?;
    let cursor: Cursor = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if cursor.version != 1 {
        return Err(invalid());
    }
    Ok(cursor)
}

fn digest(value: &impl Serialize) -> AppResult<String> {
    serde_json::to_vec(value)
        .map(|bytes| sha256(&bytes))
        .map_err(|_| {
            AppError::new(
                ErrorType::Internal,
                "SEARCH_ENCODING_FAILED",
                "Search state could not be fingerprinted.",
            )
        })
}

fn invalid() -> AppError {
    AppError::new(
        ErrorType::Validation,
        "INVALID_CURSOR",
        "The search cursor is invalid or unsupported.",
    )
    .with_param("cursor")
}
