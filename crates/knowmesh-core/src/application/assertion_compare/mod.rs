mod plan;
mod selection;
mod types;

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use types::*;

use crate::{
    domain::{Claim, ClaimId, LifecycleStatus, claim_conflict_groups, sha256},
    error::{AppError, AppResult, ErrorType},
    model::{self, Generation, GenerationOptions, UsageSummary},
    ports::ModelProvider,
};

pub const COMPARISON_VERSION: &str = "1";
const PROMPT: &str = include_str!("../../../prompts/assertion-comparison-v1.md");

pub struct ClaimComparisonContext<'a> {
    claims: BTreeMap<ClaimId, &'a Claim>,
    scopes: BTreeMap<String, BTreeSet<ClaimId>>,
    scope_keys: BTreeMap<ClaimId, String>,
    normalized_keys: BTreeMap<ClaimId, String>,
    context_sha256: String,
}

#[derive(Serialize, JsonSchema)]
struct ModelPair<'a> {
    left: &'a Claim,
    right: &'a Claim,
}

#[derive(Serialize, JsonSchema)]
struct ComparisonInput<'a> {
    #[schemars(length(min = 1, max = 32))]
    pairs: Vec<ModelPair<'a>>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawComparison {
    left_id: ClaimId,
    right_id: ClaimId,
    verdict: ComparisonVerdict,
    #[schemars(length(min = 1, max = 2048))]
    reason: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ComparisonOutput {
    #[schemars(length(min = 1, max = 32))]
    comparisons: Vec<RawComparison>,
}

impl<'a> ClaimComparisonContext<'a> {
    pub fn new(claims: &'a [Claim]) -> AppResult<Self> {
        if claims.len() > 100_000 {
            return Err(error(
                "CLAIM_COMPARISON_LIMIT",
                "A comparison context supports at most 100000 Claims.",
            ));
        }
        let mut context = Self {
            claims: BTreeMap::new(),
            scopes: BTreeMap::new(),
            scope_keys: BTreeMap::new(),
            normalized_keys: BTreeMap::new(),
            context_sha256: String::new(),
        };
        let mut subjects = BTreeMap::<_, Vec<_>>::new();
        let mut evidence = BTreeMap::new();
        for claim in claims {
            claim.assertion.validate()?;
            for item in &claim.assertion.evidence {
                if let Some(previous) = evidence.insert(&item.id, item)
                    && previous != item
                {
                    return Err(error(
                        "EVIDENCE_ID_CONFLICT",
                        "A comparison context must retain identical content for each shared Evidence ID.",
                    ));
                }
            }
            if context
                .claims
                .insert(claim.assertion.id.clone(), claim)
                .is_some()
            {
                return Err(error(
                    "DUPLICATE_ASSERTION_ID",
                    "A comparison context cannot repeat a Claim ID.",
                ));
            }
            subjects
                .entry(&claim.subject_node_id)
                .or_default()
                .push(&claim.assertion);
            if claim.assertion.lifecycle_status == LifecycleStatus::Active {
                let scope = hash(&(&claim.subject_node_id, &claim.assertion.qualifiers))?;
                context
                    .scopes
                    .entry(scope.clone())
                    .or_default()
                    .insert(claim.assertion.id.clone());
                context.scope_keys.insert(claim.assertion.id.clone(), scope);
                context.normalized_keys.insert(
                    claim.assertion.id.clone(),
                    claim.assertion.normalized_hash()?,
                );
            }
        }
        let mut groups = BTreeSet::new();
        for claims in subjects.values() {
            for group in claim_conflict_groups(claims.iter().copied())? {
                if !groups.insert(&group.id) {
                    return Err(error(
                        "CONFLICT_GROUP_ID_CONFLICT",
                        "Conflict group IDs must belong to one subject Node.",
                    ));
                }
            }
        }
        context.context_sha256 = hash(&(COMPARISON_VERSION, &context.claims))?;
        Ok(context)
    }

    pub fn compare(
        &self,
        pairs: &[ClaimPair],
        provider: &dyn ModelProvider,
        options: &GenerationOptions,
    ) -> AppResult<ComparisonReport> {
        let pairs = self.checked_pairs(pairs)?;
        let input = self.model_input(&pairs);
        let input_sha256 = hash(&input)?;
        let mut report = ComparisonReport {
            version: 1,
            context_sha256: self.context_sha256.clone(),
            input_sha256,
            prompt_sha256: sha256(PROMPT.as_bytes()),
            comparisons: vec![],
            usage: UsageSummary::default(),
            diagnostics: vec![],
        };
        if pairs.is_empty() {
            return Ok(report);
        }
        let generated = model::generate::<_, ComparisonOutput>(provider, PROMPT, &input, options)?;
        let mut results = BTreeMap::new();
        for result in &generated.data.comparisons {
            let pair = ordered_pair(result.left_id.clone(), result.right_id.clone());
            if !pairs.contains(&pair)
                || result.reason.trim().is_empty()
                || results.insert(pair, result).is_some()
            {
                return Err(output_invalid(&generated));
            }
        }
        if results.len() != pairs.len() {
            return Err(output_invalid(&generated));
        }
        for (pair, result) in results {
            let left = self.claims[&pair.left_id];
            let right = self.claims[&pair.right_id];
            report.comparisons.push(ClaimComparison {
                left_semantic_sha256: left.assertion.semantic_hash(&left.subject_node_id)?,
                right_semantic_sha256: right.assertion.semantic_hash(&right.subject_node_id)?,
                pair,
                verdict: result.verdict,
                reason: result.reason.clone(),
            });
        }
        report.usage = generated.usage;
        report.diagnostics = generated.diagnostics;
        Ok(report)
    }

    fn checked_pairs(&self, pairs: &[ClaimPair]) -> AppResult<BTreeSet<ClaimPair>> {
        if pairs.len() > 32 {
            return Err(error(
                "CLAIM_COMPARISON_LIMIT",
                "A model comparison batch supports at most 32 pairs.",
            ));
        }
        let mut unique = BTreeSet::new();
        for pair in pairs {
            let pair = ordered_pair(pair.left_id.clone(), pair.right_id.clone());
            self.validate_pair(&pair)?;
            if !unique.insert(pair) {
                return Err(error(
                    "DUPLICATE_CLAIM_PAIR",
                    "A comparison batch cannot repeat a Claim pair.",
                ));
            }
        }
        Ok(unique)
    }

    fn validate_pair(&self, pair: &ClaimPair) -> AppResult<()> {
        let left = self.scope_keys.get(&pair.left_id);
        let right = self.scope_keys.get(&pair.right_id);
        if pair.left_id == pair.right_id || left.is_none() || left != right {
            return Err(error(
                "CLAIM_COMPARISON_SCOPE_MISMATCH",
                "Comparison requires distinct active Claims with the same subject and qualifiers.",
            ));
        }
        if self.normalized_keys[&pair.left_id] == self.normalized_keys[&pair.right_id] {
            return Err(error(
                "CLAIM_COMPARISON_EXACT_DUPLICATE",
                "Exact duplicates must be handled by deterministic deduplication.",
            ));
        }
        Ok(())
    }

    fn model_input(&self, pairs: &BTreeSet<ClaimPair>) -> ComparisonInput<'a> {
        ComparisonInput {
            pairs: pairs
                .iter()
                .map(|pair| ModelPair {
                    left: self.claims[&pair.left_id],
                    right: self.claims[&pair.right_id],
                })
                .collect(),
        }
    }
}

fn ordered_pair(left_id: ClaimId, right_id: ClaimId) -> ClaimPair {
    if left_id < right_id {
        ClaimPair { left_id, right_id }
    } else {
        ClaimPair {
            left_id: right_id,
            right_id: left_id,
        }
    }
}

fn hash(value: &impl Serialize) -> AppResult<String> {
    serde_json::to_vec(value)
        .map(|bytes| sha256(&bytes))
        .map_err(|_| {
            error(
                "CLAIM_COMPARISON_CONTEXT_INVALID",
                "Could not encode the comparison context.",
            )
        })
}

fn error(code: &str, message: &str) -> AppError {
    AppError::new(ErrorType::Validation, code, message)
}

fn output_invalid(generation: &Generation<ComparisonOutput>) -> AppError {
    AppError::new(ErrorType::Model, "CLAIM_COMPARISON_OUTPUT_INVALID", "The model must classify every supplied Claim pair exactly once using only its supplied IDs.")
        .with_details(serde_json::json!({"usage": generation.usage, "diagnostics": generation.diagnostics}))
}
