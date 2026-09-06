use std::sync::Mutex;

use knowmesh_core::{
    application::{
        assertion_compare::{ClaimComparisonContext, PairSelection},
        evidence_verify::{EvidenceInput, EvidenceVerifier},
    },
    canonical::node::NodeDocument,
    domain::{
        Claim, ClaimId, ClaimRecord, EvidenceStance, EvidenceStatus, ExtractionMethod,
        LifecycleStatus, Locator, NodeId, NodeKind, NodeMetadata, SourceRevision, SourceRevisionId,
        sha256,
    },
    error::AppResult,
    ingest::TextParser,
    model::{GenerationOptions, ModelRequest, ModelResponse, StopReason, TokenUsage},
    ports::{ModelProvider, SourceParser},
};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct Golden {
    name: String,
    left: String,
    right: String,
    verdict: String,
    reason: String,
    groups: usize,
    changes: usize,
    #[serde(default)]
    omit_right_evidence: bool,
    blocked: Option<String>,
}

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

fn claims(statements: &[&str]) -> Vec<Claim> {
    let text = format!("# Source\n\n{}\n", statements.join("\n\n"));
    let revision = SourceRevision {
        id: SourceRevisionId::new(),
        path: "fixture.md".into(),
        mime_type: "text/markdown".into(),
        encoding: None,
        sha256: sha256(text.as_bytes()),
        byte_size: text.len() as u64,
        captured_at: "2026-09-06T00:00:00Z".parse().unwrap(),
        url: None,
    };
    let parsed = TextParser::default()
        .parse(&revision, text.as_bytes())
        .unwrap();
    let verifier = EvidenceVerifier::new(&revision, &parsed, Default::default()).unwrap();
    let subject = NodeId::new();
    statements
        .iter()
        .enumerate()
        .map(|(index, statement)| {
            let block = &parsed.blocks[index + 1];
            let evidence = verifier
                .verify(&EvidenceInput {
                    source_revision_id: revision.id.clone(),
                    quote: (*statement).into(),
                    locator: Locator {
                        page: block.page,
                        section_path: block.section_path.clone(),
                        paragraph: block.paragraph,
                        char_start: Some(block.char_start),
                        char_end: Some(block.char_end),
                    },
                    stance: EvidenceStance::Supports,
                    extraction_method: ExtractionMethod::Parser,
                    confidence: 1.0,
                })
                .unwrap()
                .into_evidence();
            Claim {
                subject_node_id: subject.clone(),
                assertion: ClaimRecord {
                    id: ClaimId::new(),
                    statement: (*statement).into(),
                    lifecycle_status: LifecycleStatus::Active,
                    evidence_status: EvidenceStatus::Supported,
                    confidence: Some(0.9),
                    qualifiers: Default::default(),
                    evidence: vec![evidence],
                    conflict_groups: vec![],
                },
            }
        })
        .collect()
}

fn document(claims: &[Claim]) -> NodeDocument {
    let metadata = NodeMetadata {
        version: 1,
        id: claims[0].subject_node_id.clone(),
        kind: NodeKind::Node,
        schema: "research@1".into(),
        node_type: "Model".into(),
        name: "Method X".into(),
        aliases: vec![],
        tags: vec![],
        lifecycle_status: LifecycleStatus::Active,
        created_at: "2026-09-06T00:00:00Z".parse().unwrap(),
        updated_at: "2026-09-06T00:00:00Z".parse().unwrap(),
        properties: Default::default(),
        extra: Default::default(),
    };
    let mut doc =
        NodeDocument::create(metadata, "## Summary\n\nFictional comparison fixture.").unwrap();
    doc.claims = claims.iter().map(|claim| claim.assertion.clone()).collect();
    doc
}

fn selection(claims: &[Claim], limit: usize) -> PairSelection {
    PairSelection {
        focus_ids: claims
            .iter()
            .map(|claim| claim.assertion.id.clone())
            .collect(),
        limit,
        cursor: None,
    }
}

#[test]
fn golden_comparisons_keep_separate_claims_and_produce_valid_review_plans() {
    let cases: Vec<Golden> =
        serde_json::from_str(include_str!("fixtures/claim_comparisons.json")).unwrap();
    for case in cases {
        let mut claims = claims(&[&case.left, &case.right]);
        if case.omit_right_evidence {
            claims[1].assertion.evidence.clear();
            claims[1].assertion.evidence_status = EvidenceStatus::Unreviewed;
        }
        let context = ClaimComparisonContext::new(&claims).unwrap();
        let pairs = context.select_pairs(&selection(&claims, 32)).unwrap();
        assert_eq!(pairs.pairs.len(), 1, "{}", case.name);
        let pair = &pairs.pairs[0];
        let provider = Fake::new(
            json!({"comparisons":[{"left_id":pair.left_id,"right_id":pair.right_id,"verdict":case.verdict,"reason":case.reason}]}),
        );
        let report = context
            .compare(&pairs.pairs, &provider, &GenerationOptions::default())
            .unwrap();
        assert_eq!(report.usage.total_tokens, 50);
        let plan = context
            .plan(&report, "2026-09-06T01:00:00Z".parse().unwrap())
            .unwrap();
        assert!(plan.requires_review);
        assert_eq!(plan.groups.len(), case.groups, "{}", case.name);
        assert_eq!(plan.claim_changes.len(), case.changes, "{}", case.name);
        assert_eq!(
            plan.possible_duplicates.len(),
            usize::from(case.verdict == "possible_duplicate")
        );
        assert_eq!(
            plan.undetermined.len(),
            usize::from(case.verdict == "undetermined")
        );
        assert_eq!(
            plan.blocked_conflicts
                .first()
                .map(|blocked| blocked.code.as_str()),
            case.blocked.as_deref()
        );
        let mut doc = document(&claims);
        for change in &plan.claim_changes {
            let claim = doc
                .claims
                .iter_mut()
                .find(|claim| claim.id == change.record.assertion.id)
                .unwrap();
            *claim = change.record.assertion.clone();
        }
        let rendered = doc.render().unwrap();
        let reloaded = NodeDocument::parse(&rendered).unwrap();
        assert_eq!(reloaded.claims.len(), 2);
        for group in &plan.groups {
            for claim in &reloaded.claims {
                assert_eq!(claim.conflict_groups, vec![group.clone()]);
                assert_eq!(claim.evidence_status, EvidenceStatus::Conflicting);
            }
        }
        let requests = provider.requests.lock().unwrap();
        assert!(!requests[0].messages[0].content.contains(&case.left));
        assert!(requests[0].messages[1].content.contains(&case.left));
    }
}

#[test]
fn pair_pages_are_complete_stable_and_bound_to_the_context_and_focus() {
    let claims = claims(&[
        "One result.",
        "Another result.",
        "Third result.",
        "Fourth result.",
    ]);
    let context = ClaimComparisonContext::new(&claims).unwrap();
    let mut input = selection(&claims, 1);
    let mut seen = std::collections::BTreeSet::new();
    loop {
        let page = context.select_pairs(&input).unwrap();
        for pair in page.pairs {
            assert!(seen.insert((pair.left_id, pair.right_id)));
        }
        input.cursor = page.next_cursor;
        if input.cursor.is_none() {
            break;
        }
    }
    assert_eq!(seen.len(), 6);
    let first = context.select_pairs(&selection(&claims, 1)).unwrap();
    let mut different_focus = selection(&claims, 1);
    different_focus.focus_ids.truncate(1);
    different_focus.cursor = first.next_cursor.clone();
    assert_eq!(
        context.select_pairs(&different_focus).unwrap_err().code,
        "CLAIM_COMPARISON_CURSOR_MISMATCH"
    );
    let mut changed = claims.clone();
    changed[0].assertion.statement = "An updated result.".into();
    let changed_context = ClaimComparisonContext::new(&changed).unwrap();
    let mut stale = selection(&changed, 1);
    stale.cursor = first.next_cursor;
    assert_eq!(
        changed_context.select_pairs(&stale).unwrap_err().code,
        "CLAIM_COMPARISON_CURSOR_MISMATCH"
    );
}

#[test]
fn cross_scope_and_exact_pairs_are_excluded_or_rejected_before_model_calls() {
    let mut claims = claims(&[
        "One result.",
        "One result.",
        "Other result.",
        "Fourth result.",
    ]);
    claims[2].subject_node_id = NodeId::new();
    claims[3]
        .assertion
        .qualifiers
        .insert("population".into(), json!("other"));
    let context = ClaimComparisonContext::new(&claims).unwrap();
    assert!(
        context
            .select_pairs(&selection(&claims, 32))
            .unwrap()
            .pairs
            .is_empty()
    );
    let provider = Fake::new(json!({"comparisons":[]}));
    let pair = knowmesh_core::application::assertion_compare::ClaimPair {
        left_id: claims[0].assertion.id.clone(),
        right_id: claims[2].assertion.id.clone(),
    };
    assert_eq!(
        context
            .compare(&[pair], &provider, &Default::default())
            .unwrap_err()
            .code,
        "CLAIM_COMPARISON_SCOPE_MISMATCH"
    );
    assert!(provider.requests.lock().unwrap().is_empty());
}

#[test]
fn model_pair_identity_is_closed_and_invalid_results_retain_usage_without_raw_text() {
    let claims = claims(&["One result.", "An opposite result."]);
    let context = ClaimComparisonContext::new(&claims).unwrap();
    let pairs = context.select_pairs(&selection(&claims, 32)).unwrap();
    let provider = Fake::new(
        json!({"comparisons":[{"left_id":claims[0].assertion.id,"right_id":ClaimId::new(),"verdict":"conflicting","reason":"private invalid reason"}]}),
    );
    let error = context
        .compare(&pairs.pairs, &provider, &Default::default())
        .unwrap_err();
    assert_eq!(error.code, "CLAIM_COMPARISON_OUTPUT_INVALID");
    assert_eq!(error.details.as_ref().unwrap()["usage"]["requests"], 1);
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("private invalid reason")
    );
}

#[test]
fn changed_assertions_make_a_previous_comparison_report_stale() {
    let claims = claims(&["One result.", "An opposite result."]);
    let context = ClaimComparisonContext::new(&claims).unwrap();
    let pairs = context.select_pairs(&selection(&claims, 32)).unwrap();
    let pair = &pairs.pairs[0];
    let provider = Fake::new(
        json!({"comparisons":[{"left_id":pair.left_id,"right_id":pair.right_id,"verdict":"conflicting","reason":"Opposite results."}]}),
    );
    let report = context
        .compare(&pairs.pairs, &provider, &Default::default())
        .unwrap();
    let mut changed = claims.clone();
    changed[0].assertion.evidence[0].confidence = 0.7;
    let context = ClaimComparisonContext::new(&changed).unwrap();
    assert_eq!(
        context
            .plan(&report, "2026-09-06T01:00:00Z".parse().unwrap())
            .unwrap_err()
            .code,
        "CLAIM_COMPARISON_STALE"
    );
}
