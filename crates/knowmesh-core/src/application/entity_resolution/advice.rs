use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{EntityInput, ResolutionDecision, ResolutionReport, error, hash, validate_names};
use crate::{
    domain::{NodeId, sha256, valid_sha256},
    error::{AppError, AppResult, ErrorType},
    model::{self, Generation, GenerationOptions, OutputDiagnostic, UsageSummary},
    ports::ModelProvider,
};

const PROMPT: &str = include_str!("../../../prompts/entity-resolution-v1.md");

#[derive(Serialize, JsonSchema)]
struct AdviceInput<'a> {
    entity: &'a EntityInput,
    resolution: &'a ResolutionReport,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "decision", rename_all = "snake_case", deny_unknown_fields)]
enum AdviceOutput {
    Existing {
        node_id: NodeId,
        #[schemars(length(min = 1, max = 2048))]
        reason: String,
    },
    New {
        #[schemars(length(min = 1, max = 2048))]
        reason: String,
    },
    Ambiguous {
        #[schemars(length(min = 1, max = 2048))]
        reason: String,
    },
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct EntityAdvice {
    pub decision: ResolutionDecision,
    pub selected_node_id: Option<NodeId>,
    pub reason: String,
    pub requires_review: bool,
    pub report_sha256: String,
    pub prompt_sha256: String,
    pub usage: UsageSummary,
    pub diagnostics: Vec<OutputDiagnostic>,
    pub warnings: Vec<String>,
}

pub fn advise(
    input: &EntityInput,
    report: &ResolutionReport,
    provider: &dyn ModelProvider,
    options: &GenerationOptions,
) -> AppResult<EntityAdvice> {
    validate_context(input, report)?;
    if report.automatic {
        return Err(error(
            "ENTITY_ALREADY_RESOLVED",
            "A deterministic automatic match does not require model advice.",
        ));
    }
    let generation = model::generate::<_, AdviceOutput>(
        provider,
        PROMPT,
        &AdviceInput {
            entity: input,
            resolution: report,
        },
        options,
    )?;
    let (mut decision, mut selected_node_id, reason) = match &generation.data {
        AdviceOutput::Existing { node_id, reason } => {
            let candidate = report
                .candidates
                .iter()
                .find(|candidate| &candidate.node_id == node_id)
                .ok_or_else(|| rejected("ENTITY_ADVICE_TARGET_INVALID", &generation))?;
            if !candidate.warnings.is_empty() || candidate.node_type != input.node_type {
                return Err(rejected("ENTITY_ADVICE_TARGET_BLOCKED", &generation));
            }
            (
                ResolutionDecision::Existing,
                Some(node_id.clone()),
                reason.clone(),
            )
        }
        AdviceOutput::New { reason } => (ResolutionDecision::New, None, reason.clone()),
        AdviceOutput::Ambiguous { reason } => (ResolutionDecision::Ambiguous, None, reason.clone()),
    };
    if reason.trim().is_empty() {
        return Err(rejected("ENTITY_ADVICE_INVALID", &generation));
    }
    let mut warnings = vec![];
    if report.decision == ResolutionDecision::Ambiguous
        || report.candidates_truncated
        || report
            .warnings
            .iter()
            .any(|warning| warning == "ENTITY_RETRIEVAL_LIMIT_REACHED")
    {
        decision = ResolutionDecision::Ambiguous;
        selected_node_id = None;
        warnings.push("ENTITY_AMBIGUITY_REQUIRES_REVIEW".into());
    }
    Ok(EntityAdvice {
        decision,
        selected_node_id,
        reason,
        requires_review: true,
        report_sha256: hash(report)?,
        prompt_sha256: sha256(PROMPT.as_bytes()),
        usage: generation.usage,
        diagnostics: generation.diagnostics,
        warnings,
    })
}

fn validate_context(input: &EntityInput, report: &ResolutionReport) -> AppResult<()> {
    validate_names(&input.name, &input.aliases)?;
    if report.input_sha256 != hash(input)?
        || !valid_sha256(&report.catalog_sha256)
        || !valid_sha256(&report.options_sha256)
        || report
            .retrieval_sha256
            .as_ref()
            .is_some_and(|hash| !valid_sha256(hash))
        || (report.retrieval_available && report.retrieval_sha256.is_none())
        || report.candidates.len() > 100
        || report.total_candidates > 100_000
        || report.total_candidates < report.candidates.len()
        || report.candidates_truncated != (report.total_candidates > report.candidates.len())
        || report.warnings.len() > 128
    {
        return Err(context_error());
    }
    let mut ids = BTreeSet::new();
    for candidate in &report.candidates {
        validate_names(&candidate.name, &[])?;
        if !ids.insert(&candidate.node_id)
            || candidate.node_type.is_empty()
            || candidate.node_type.len() > 64
            || candidate.matched_by.len() > 256
            || candidate.warnings.len() > 128
            || candidate
                .matched_by
                .iter()
                .chain(&candidate.warnings)
                .any(|text| text.len() > 256)
            || candidate
                .retrieval_score
                .is_some_and(|score| !score.is_finite() || !(0.0..=1.0).contains(&score))
        {
            return Err(context_error());
        }
    }
    match report.decision {
        ResolutionDecision::Existing
            if report.selected_node_id.as_ref().is_some_and(|id| {
                report.candidates.iter().any(|candidate| {
                    &candidate.node_id == id
                        && candidate.warnings.is_empty()
                        && candidate.node_type == input.node_type
                })
            }) => {}
        ResolutionDecision::New | ResolutionDecision::Ambiguous
            if report.selected_node_id.is_none() && !report.automatic => {}
        _ => return Err(context_error()),
    }
    Ok(())
}

fn context_error() -> AppError {
    error(
        "ENTITY_CONTEXT_MISMATCH",
        "Model advice requires the matching bounded resolution report.",
    )
}

fn rejected(code: &str, generation: &Generation<AdviceOutput>) -> AppError {
    AppError::new(
        ErrorType::Model,
        code,
        "The model suggestion violates the supplied entity candidate constraints.",
    )
    .with_details(
        serde_json::json!({"usage": generation.usage, "diagnostics": generation.diagnostics}),
    )
}
