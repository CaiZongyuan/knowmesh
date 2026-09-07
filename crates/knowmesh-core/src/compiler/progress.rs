use std::time::{Duration, Instant};

use serde_json::json;

use super::{
    CandidateIdentity, ChunkExtraction, ExtractionDiagnostic, ExtractionRequest, add_usage,
};
use crate::{
    error::{AppError, AppResult, ErrorType},
    model::{GenerationOptions, OutputDiagnostic, UsageSummary},
};

#[derive(Default)]
pub(super) struct Progress {
    pub identity: Option<CandidateIdentity>,
    pub chunks: Vec<ChunkExtraction>,
    pub usage: UsageSummary,
    pub diagnostics: Vec<ExtractionDiagnostic>,
}

impl Progress {
    pub fn generated_diagnostics(&mut self, diagnostics: &[OutputDiagnostic]) {
        if let Some(identity) = &self.identity {
            self.diagnostics
                .extend(diagnostics.iter().map(|diagnostic| ExtractionDiagnostic {
                    chunk_id: identity.chunk_id.clone(),
                    attempt: diagnostic.attempt,
                    reason: diagnostic.reason.clone(),
                    response_sha256: Some(diagnostic.response_sha256.clone()),
                    candidate_sha256: None,
                }));
        }
    }

    pub fn remaining(
        &self,
        options: &GenerationOptions,
        started: Instant,
    ) -> AppResult<GenerationOptions> {
        let mut remaining = options.clone();
        remaining.max_calls = options.max_calls.saturating_sub(self.usage.requests);
        remaining.max_total_tokens = options
            .max_total_tokens
            .saturating_sub(self.usage.total_tokens);
        if remaining.max_calls == 0 || remaining.max_total_tokens == 0 {
            return Err(AppError::new(
                ErrorType::Policy,
                "MODEL_BUDGET_EXHAUSTED",
                "The extraction model budget is exhausted.",
            ));
        }
        remaining.timeout_ms = Duration::from_millis(options.timeout_ms)
            .saturating_sub(started.elapsed())
            .as_millis() as u64;
        if remaining.timeout_ms == 0 {
            return Err(AppError::new(
                ErrorType::Network,
                "MODEL_TIMEOUT",
                "The extraction deadline has elapsed.",
            )
            .retryable(true));
        }
        Ok(remaining)
    }

    pub fn generation_failed(&mut self, error: AppError) -> AppError {
        if let Some(details) = &error.details {
            if let Some(usage) = details
                .get("usage")
                .and_then(|value| serde_json::from_value(value.clone()).ok())
            {
                add_usage(&mut self.usage, &usage);
            }
            if let Some(diagnostics) = details.get("diagnostics").and_then(|value| {
                serde_json::from_value::<Vec<OutputDiagnostic>>(value.clone()).ok()
            }) {
                self.generated_diagnostics(&diagnostics);
            }
        }
        error
    }

    pub fn failed(&self, error: AppError, request: &ExtractionRequest<'_>) -> AppError {
        AppError::new(
            error.error_type,
            error.code,
            "Candidate extraction failed; inspect the error code and diagnostics.",
        )
        .retryable(error.retryable)
        .with_details(json!({
            "source_revision_id":request.revision.id,
            "source_sha256":request.revision.sha256,
            "identity":self.identity,
            "completed_chunks":self.chunks,
            "usage":self.usage,
            "diagnostics":self.diagnostics,
        }))
    }
}
