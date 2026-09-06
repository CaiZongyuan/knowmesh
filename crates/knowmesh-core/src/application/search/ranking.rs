use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    application::lexical::{LexicalHit, RecordType},
    canonical::workspace::SearchSettings,
    domain::normalize_name,
    error::{AppError, AppResult, ErrorType},
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RankingConfig {
    pub k: usize,
    pub word_weight: f64,
    pub trigram_weight: f64,
    pub vector_weight: f64,
    pub boosts_enabled: bool,
}

impl Default for RankingConfig {
    fn default() -> Self {
        Self::from(&SearchSettings::default())
    }
}

impl From<&SearchSettings> for RankingConfig {
    fn from(settings: &SearchSettings) -> Self {
        Self {
            k: settings.rrf_k,
            word_weight: settings.word_weight,
            trigram_weight: settings.trigram_weight,
            vector_weight: settings.vector_weight,
            boosts_enabled: settings.boosts_enabled,
        }
    }
}

impl RankingConfig {
    pub fn validate(&self) -> AppResult<()> {
        if self.k == 0
            || [self.word_weight, self.trigram_weight, self.vector_weight]
                .iter()
                .any(|weight| !weight.is_finite() || *weight <= 0.0)
        {
            return Err(AppError::new(
                ErrorType::Validation,
                "INVALID_RANKING_CONFIG",
                "RRF requires k >= 1 and finite positive channel weights.",
            ));
        }
        Ok(())
    }

    fn weight(&self, channel: Channel) -> f64 {
        match channel {
            Channel::Word => self.word_weight,
            Channel::Trigram | Channel::ShortText => self.trigram_weight,
            Channel::Vector => self.vector_weight,
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Word,
    Trigram,
    ShortText,
    Vector,
}

#[derive(Debug, Clone)]
pub struct ChannelInput {
    pub channel: Channel,
    pub hits: Vec<LexicalHit>,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Candidate {
    pub unit_id: String,
    pub record_type: RecordType,
    pub record_id: String,
    pub title: String,
    pub aliases: Vec<String>,
}

impl From<&LexicalHit> for Candidate {
    fn from(hit: &LexicalHit) -> Self {
        Self {
            unit_id: hit.unit_id.clone(),
            record_type: hit.record_type,
            record_id: hit.record_id.clone(),
            title: hit.title.clone(),
            aliases: hit.aliases.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ChannelSummary {
    pub channel: Channel,
    pub weight: f64,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Contribution {
    pub channel: Channel,
    pub rank: u32,
    pub weight: f64,
    pub contribution: f64,
}

#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct Boosts {
    pub exact_name: f64,
    pub exact_alias: f64,
    pub title_prefix: f64,
    pub total: f64,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Explanation {
    pub channels: Vec<Contribution>,
    pub degraded_channels: Vec<ChannelSummary>,
    pub raw_rrf: f64,
    pub normalization_bound: f64,
    pub normalized_score: f64,
    pub exact_id_tier: bool,
    pub boosts: Boosts,
    pub final_score: f64,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RankedHit {
    pub candidate: Candidate,
    pub explain: Explanation,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RankingResult {
    pub channels: Vec<ChannelSummary>,
    pub hits: Vec<RankedHit>,
}

pub fn fuse(
    query: &str,
    config: &RankingConfig,
    inputs: &[ChannelInput],
    exact_candidates: &[LexicalHit],
) -> AppResult<RankingResult> {
    config.validate()?;
    let mut ordered = BTreeMap::new();
    for input in inputs {
        if ordered.insert(input.channel, input).is_some()
            || (input.unavailable_reason.is_some() && !input.hits.is_empty())
        {
            return Err(invalid_candidates());
        }
    }
    if ordered.contains_key(&Channel::Trigram) && ordered.contains_key(&Channel::ShortText) {
        return Err(invalid_candidates());
    }
    let channels: Vec<_> = ordered
        .values()
        .map(|input| ChannelSummary {
            channel: input.channel,
            weight: config.weight(input.channel),
            unavailable_reason: input.unavailable_reason.clone(),
        })
        .collect();
    let bound: f64 = channels
        .iter()
        .filter(|channel| channel.unavailable_reason.is_none())
        .map(|channel| channel.weight / (config.k as f64 + 1.0))
        .sum();
    if !bound.is_finite() {
        return Err(AppError::new(
            ErrorType::Validation,
            "INVALID_RANKING_CONFIG",
            "The channel normalization bound is not finite.",
        ));
    }
    let mut candidates = BTreeMap::new();
    let mut contributions: BTreeMap<String, Vec<Contribution>> = BTreeMap::new();
    for input in ordered
        .values()
        .filter(|input| input.unavailable_reason.is_none())
    {
        let mut ranks = BTreeMap::new();
        for hit in &input.hits {
            if hit.rank == 0 {
                return Err(invalid_candidates());
            }
            insert(&mut candidates, hit)?;
            ranks
                .entry(hit.unit_id.clone())
                .and_modify(|rank: &mut u32| *rank = (*rank).min(hit.rank))
                .or_insert(hit.rank);
        }
        for (id, rank) in ranks {
            contributions.entry(id).or_default().push(Contribution {
                channel: input.channel,
                rank,
                weight: config.weight(input.channel),
                contribution: config.weight(input.channel) / (config.k as f64 + f64::from(rank)),
            });
        }
    }
    if bound == 0.0 {
        candidates.clear();
        contributions.clear();
    }
    for hit in exact_candidates {
        if hit.record_id != query.trim() {
            return Err(invalid_candidates());
        }
        insert(&mut candidates, hit)?;
    }
    let normalized_query = normalize_name(query);
    let degraded_channels: Vec<_> = channels
        .iter()
        .filter(|channel| channel.unavailable_reason.is_some())
        .cloned()
        .collect();
    let mut hits = Vec::new();
    for (id, candidate) in candidates {
        let channels = contributions.remove(&id).unwrap_or_default();
        let raw_rrf: f64 = channels.iter().map(|part| part.contribution).sum();
        let normalized_score = if bound > 0.0 {
            (raw_rrf / bound).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let boosts = if config.boosts_enabled {
            boosts(&candidate, &normalized_query)
        } else {
            Boosts::default()
        };
        let exact_id_tier = candidate.record_id == query.trim();
        let final_score = normalized_score + boosts.total;
        hits.push(RankedHit {
            candidate,
            explain: Explanation {
                channels,
                degraded_channels: degraded_channels.clone(),
                raw_rrf,
                normalization_bound: bound,
                normalized_score,
                exact_id_tier,
                boosts,
                final_score,
            },
        });
    }
    hits.sort_by(|left, right| {
        right
            .explain
            .exact_id_tier
            .cmp(&left.explain.exact_id_tier)
            .then_with(|| {
                right
                    .explain
                    .final_score
                    .total_cmp(&left.explain.final_score)
            })
            .then_with(|| left.candidate.unit_id.cmp(&right.candidate.unit_id))
    });
    Ok(RankingResult { channels, hits })
}

fn insert(candidates: &mut BTreeMap<String, Candidate>, hit: &LexicalHit) -> AppResult<()> {
    let candidate = Candidate::from(hit);
    if candidate.unit_id.is_empty() || candidate.record_id.is_empty() {
        return Err(invalid_candidates());
    }
    if let Some(previous) = candidates.insert(candidate.unit_id.clone(), candidate.clone())
        && previous != candidate
    {
        return Err(invalid_candidates());
    }
    Ok(())
}

fn boosts(candidate: &Candidate, query: &str) -> Boosts {
    if query.is_empty() {
        return Boosts::default();
    }
    let title = normalize_name(&candidate.title);
    let mut boosts = Boosts::default();
    if candidate.record_type == RecordType::Node && title == query {
        boosts.exact_name = 0.05;
    }
    if candidate
        .aliases
        .iter()
        .any(|alias| normalize_name(alias) == query)
    {
        boosts.exact_alias = 0.04;
    }
    if title.starts_with(query) {
        boosts.title_prefix = 0.02;
    }
    boosts.total = (boosts.exact_name + boosts.exact_alias + boosts.title_prefix).min(0.08);
    boosts
}

fn invalid_candidates() -> AppError {
    AppError::new(
        ErrorType::Internal,
        "INVALID_CHANNEL_CANDIDATES",
        "Search channels require unique identities, consistent metadata, and positive ranks.",
    )
}
