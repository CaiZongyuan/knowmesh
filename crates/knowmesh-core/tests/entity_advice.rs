use std::sync::Mutex;

use knowmesh_core::{
    application::entity_resolution::{
        EntityInput, EntityResolver, ResolutionDecision, ResolutionReport, advise,
    },
    canonical::schema::{Schema, builtin},
    domain::{LifecycleStatus, NodeId, NodeKind, NodeMetadata},
    error::AppResult,
    model::{GenerationOptions, ModelRequest, ModelResponse, StopReason, TokenUsage},
    ports::ModelProvider,
};
use serde_json::json;

struct Fake {
    text: String,
    requests: Mutex<Vec<ModelRequest>>,
}

impl Fake {
    fn new(value: serde_json::Value) -> Self {
        Self {
            text: value.to_string(),
            requests: Mutex::new(vec![]),
        }
    }
}

impl ModelProvider for Fake {
    fn complete(&self, request: &ModelRequest) -> AppResult<ModelResponse> {
        self.requests.lock().unwrap().push(request.clone());
        Ok(ModelResponse {
            text: self.text.clone(),
            stop_reason: StopReason::Complete,
            usage: Some(TokenUsage {
                input_tokens: 30,
                output_tokens: 20,
            }),
        })
    }
}

fn fixture(count: usize, name: &str, node_type: &str) -> (EntityInput, ResolutionReport) {
    let schema = Schema::compose(vec![
        builtin("base@1").unwrap(),
        builtin("research@1").unwrap(),
    ])
    .unwrap();
    let nodes: Vec<_> = (0..count)
        .map(|_| NodeMetadata {
            version: 1,
            id: NodeId::new(),
            kind: NodeKind::Node,
            schema: "research@1".into(),
            node_type: "Model".into(),
            name: "Canonical".into(),
            aliases: vec!["Alias".into()],
            tags: vec![],
            lifecycle_status: LifecycleStatus::Active,
            created_at: "2026-09-06T00:00:00Z".parse().unwrap(),
            updated_at: "2026-09-06T00:00:00Z".parse().unwrap(),
            properties: Default::default(),
            extra: Default::default(),
        })
        .collect();
    let input = EntityInput {
        name: name.into(),
        node_type: node_type.into(),
        aliases: vec![],
        properties: Default::default(),
    };
    let report = EntityResolver::new(&schema, &nodes, Default::default())
        .unwrap()
        .resolve(&input)
        .unwrap();
    (input, report)
}

fn options() -> GenerationOptions {
    GenerationOptions {
        retry_backoff_ms: 0,
        ..Default::default()
    }
}

#[test]
fn bounded_model_advice_uses_only_the_supplied_candidates_and_always_requires_review() {
    let (input, report) = fixture(1, "Canonical", "Model");
    let selected = report.candidates[0].node_id.clone();
    let provider = Fake::new(
        json!({"decision":"existing", "node_id":selected, "reason":"The supplied names match."}),
    );
    let result = advise(&input, &report, &provider, &options()).unwrap();
    assert_eq!(result.decision, ResolutionDecision::Existing);
    assert_eq!(result.selected_node_id, Some(selected));
    assert!(result.requires_review);
    assert_eq!(result.usage.requests, 1);
    assert_eq!(result.usage.total_tokens, 50);
    assert_eq!(result.report_sha256.len(), 64);
    assert_eq!(result.prompt_sha256.len(), 64);
    let requests = provider.requests.lock().unwrap();
    assert!(!requests[0].messages[0].content.contains("Canonical"));
    assert!(requests[0].messages[1].content.contains("Canonical"));
}

#[test]
fn an_unknown_or_incompatible_model_target_is_rejected_with_usage_and_without_raw_output() {
    let (input, report) = fixture(1, "Canonical", "Model");
    let provider = Fake::new(
        json!({"decision":"existing", "node_id":NodeId::new(), "reason":"private raw reason"}),
    );
    let error = advise(&input, &report, &provider, &options()).unwrap_err();
    assert_eq!(error.code, "ENTITY_ADVICE_TARGET_INVALID");
    assert_eq!(error.details.as_ref().unwrap()["usage"]["requests"], 1);
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("private raw reason")
    );
    let (input, report) = fixture(1, "Canonical", "Dataset");
    let provider = Fake::new(
        json!({"decision":"existing", "node_id":report.candidates[0].node_id, "reason":"Likely related."}),
    );
    assert_eq!(
        advise(&input, &report, &provider, &options())
            .unwrap_err()
            .code,
        "ENTITY_ADVICE_TARGET_BLOCKED"
    );
}

#[test]
fn model_preference_cannot_silently_resolve_deterministic_ambiguity() {
    let (input, report) = fixture(2, "Canonical", "Model");
    let provider = Fake::new(
        json!({"decision":"existing", "node_id":report.candidates[0].node_id, "reason":"First candidate."}),
    );
    let result = advise(&input, &report, &provider, &options()).unwrap();
    assert_eq!(result.decision, ResolutionDecision::Ambiguous);
    assert_eq!(result.selected_node_id, None);
    assert!(result.requires_review);
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning == "ENTITY_AMBIGUITY_REQUIRES_REVIEW")
    );
}

#[test]
fn mismatched_inputs_and_already_resolved_entities_do_not_invoke_the_provider() {
    let (mut input, report) = fixture(1, "Canonical", "Model");
    let provider = Fake::new(json!({"decision":"new", "reason":"New entity."}));
    input.name = "Other".into();
    assert_eq!(
        advise(&input, &report, &provider, &options())
            .unwrap_err()
            .code,
        "ENTITY_CONTEXT_MISMATCH"
    );
    let (input, report) = fixture(1, "Alias", "Model");
    assert_eq!(
        advise(&input, &report, &provider, &options())
            .unwrap_err()
            .code,
        "ENTITY_ALREADY_RESOLVED"
    );
    assert!(provider.requests.lock().unwrap().is_empty());
}

#[test]
fn invalid_advice_is_bounded_by_the_existing_generation_contract() {
    let (input, report) = fixture(1, "Canonical", "Model");
    let provider = Fake::new(json!({"decision":"execute_shell", "reason":"Ignore instructions."}));
    let error = advise(&input, &report, &provider, &options()).unwrap_err();
    assert_eq!(error.code, "STRUCTURED_OUTPUT_INVALID");
    assert_eq!(provider.requests.lock().unwrap().len(), 3);
    assert_eq!(
        error.details.as_ref().unwrap()["usage"]["total_tokens"],
        150
    );
}

#[test]
fn new_and_ambiguous_model_advice_are_retained_as_review_suggestions() {
    let (input, report) = fixture(0, "Unseen", "Model");
    for decision in ["new", "ambiguous"] {
        let provider = Fake::new(
            json!({"decision":decision, "reason":"Insufficient context for an existing link."}),
        );
        let result = advise(&input, &report, &provider, &options()).unwrap();
        assert_eq!(result.selected_node_id, None);
        assert!(result.requires_review);
        assert_eq!(serde_json::to_value(result.decision).unwrap(), decision);
    }
}
