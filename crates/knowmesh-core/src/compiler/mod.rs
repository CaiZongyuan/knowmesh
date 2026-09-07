mod aggregate;
mod input;
mod progress;
mod types;
mod validate;

pub use types::*;

use serde::Serialize;

use input::{InputContract, ModelInput};

use crate::{
    domain::sha256,
    error::{AppError, AppResult, ErrorType},
    ingest::cache::{FileStageCache, chunk_cached, parse_cached},
    model::{self, UsageSummary},
    ports::{ModelProvider, SourceParser},
};

pub const PROMPT_ID: &str = "compiler-v1";
const PROMPT: &str = include_str!("../../prompts/compiler-v1.md");

pub fn extract(
    cache: &FileStageCache,
    parser: &dyn SourceParser,
    provider: &dyn ModelProvider,
    request: &ExtractionRequest<'_>,
    options: &ExtractionOptions,
) -> AppResult<ExtractionReport> {
    let started = std::time::Instant::now();
    let mut progress = progress::Progress::default();
    let result = (|| {
        validate::context(request, options)?;
        let parsed = parse_cached(cache, parser, request.revision, request.bytes)?;
        let chunked = chunk_cached(
            cache,
            request.revision,
            &parsed.value,
            &options.chunking,
            None,
        )?;
        if chunked.value.chunks.len() > 128 {
            return Err(validate::rejected("CANDIDATE_LIMIT_EXCEEDED"));
        }
        let validator = validate::CandidateValidator::new(request.schema, options.mode)?;
        let contract = InputContract::new(request, options)?;
        let mut candidates = Candidates::default();
        for chunk in &chunked.value.chunks {
            let input = ModelInput::new(request, options, chunk, &parsed.value.blocks);
            let identity = contract.identity(&input, request.model)?;
            progress.identity = Some(identity.clone());
            contract.validate(&input)?;
            let cached = cache.load(&identity.key, |artifact: &CandidateArtifact| {
                if artifact.version != 1 || hash(&artifact.identity)? != hash(&identity)? {
                    return Err(invalid());
                }
                validate::accounting(artifact)?;
                validator.local(&artifact.candidates, chunk, &input.blocks)
            })?;
            let (artifact, reference, cache_hit) = if let Some(hit) = cached {
                (hit.value, hit.reference, true)
            } else {
                let remaining = progress.remaining(&options.generation, started)?;
                let generation = model::generate::<_, types::LocalCandidates>(
                    provider,
                    &contract.instructions,
                    &input,
                    &remaining,
                )
                .map_err(|error| progress.generation_failed(error))?;
                let artifact = CandidateArtifact {
                    version: 1,
                    identity: identity.clone(),
                    candidates: generation.data.into(),
                    usage: generation.usage,
                    diagnostics: generation.diagnostics,
                };
                add_usage(&mut progress.usage, &artifact.usage);
                progress.generated_diagnostics(&artifact.diagnostics);
                if let Err(error) = validator.local(&artifact.candidates, chunk, &input.blocks) {
                    progress.diagnostics.push(ExtractionDiagnostic {
                        chunk_id: chunk.id.clone(),
                        attempt: artifact.usage.repairs + 1,
                        reason: error.code.clone(),
                        response_sha256: None,
                        candidate_sha256: Some(hash(&artifact.candidates)?),
                    });
                    return Err(error);
                }
                let reference = cache.store(&identity.key, &artifact)?;
                (artifact, reference, false)
            };
            progress.chunks.push(ChunkExtraction {
                identity,
                artifact: reference,
                cache_hit,
                usage: artifact.usage,
                diagnostics: artifact.diagnostics,
            });
            aggregate::append(&mut candidates, artifact.candidates, chunk.ordinal)?;
        }
        let mut warnings: Vec<_> = parsed
            .value
            .warnings
            .iter()
            .map(|warning| warning.code.clone())
            .collect();
        warnings.extend(chunked.value.warnings);
        Ok(ExtractionReport {
            candidates_sha256: hash(&candidates)?,
            candidates,
            requires_review: true,
            parsed: parsed.reference,
            chunked: chunked.reference,
            chunks: std::mem::take(&mut progress.chunks),
            usage: std::mem::take(&mut progress.usage),
            diagnostics: std::mem::take(&mut progress.diagnostics),
            warnings,
        })
    })();
    result.map_err(|error| progress.failed(error, request))
}

fn hash(value: &impl Serialize) -> AppResult<String> {
    serde_json::to_vec(value)
        .map(|bytes| sha256(&bytes))
        .map_err(|_| invalid())
}

fn invalid() -> AppError {
    AppError::new(
        ErrorType::Validation,
        "INVALID_CANDIDATE_ARTIFACT",
        "The candidate artifact does not match its extraction context.",
    )
}

fn add_usage(total: &mut UsageSummary, part: &UsageSummary) {
    total.requests = total.requests.saturating_add(part.requests);
    total.retries = total.retries.saturating_add(part.retries);
    total.repairs = total.repairs.saturating_add(part.repairs);
    total.input_tokens = total.input_tokens.saturating_add(part.input_tokens);
    total.output_tokens = total.output_tokens.saturating_add(part.output_tokens);
    total.total_tokens = total.input_tokens.saturating_add(total.output_tokens);
    total.estimated |= part.estimated;
}
