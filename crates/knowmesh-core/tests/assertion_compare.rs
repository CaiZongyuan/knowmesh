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
        let report: knowmesh_core::application::assertion_compare::ComparisonReport =
            serde_json::from_value(serde_json::to_value(&report).unwrap()).unwrap();
        assert_eq!(report.usage.total_tokens, 50);
        let plan = context
            .plan(&report, "2026-09-06T01:00:00Z".parse().unwrap())
            .unwrap();
        let plan: knowmesh_core::application::assertion_compare::ConflictPlan =
            serde_json::from_value(serde_json::to_value(&plan).unwrap()).unwrap();
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
            let original = claims
                .iter()
                .find(|claim| claim.assertion.id == change.record.assertion.id)
                .unwrap();
            assert_eq!(
                change.record.assertion.statement,
                original.assertion.statement
            );
            assert_eq!(
                change.record.assertion.evidence,
                original.assertion.evidence
            );
            assert_eq!(
                change.record.assertion.lifecycle_status,
                original.assertion.lifecycle_status
            );
            assert_eq!(
                change.record.assertion.confidence,
                original.assertion.confidence
            );
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

fn conflict_report(
    context: &ClaimComparisonContext<'_>,
    claims: &[Claim],
) -> knowmesh_core::application::assertion_compare::ComparisonReport {
    let page = context.select_pairs(&selection(claims, 32)).unwrap();
    let values: Vec<_> = page.pairs.iter().map(|pair| json!({"left_id":pair.left_id,"right_id":pair.right_id,"verdict":"conflicting","reason":"Opposite results in the supplied scope."})).collect();
    context
        .compare(
            &page.pairs,
            &Fake::new(json!({"comparisons": values})),
            &Default::default(),
        )
        .unwrap()
}

fn materialized(
    claims: &[Claim],
    plan: &knowmesh_core::application::assertion_compare::ConflictPlan,
) -> Vec<Claim> {
    claims
        .iter()
        .map(|claim| {
            plan.claim_changes
                .iter()
                .find(|change| change.record.assertion.id == claim.assertion.id)
                .map_or_else(|| claim.clone(), |change| change.record.clone())
        })
        .collect()
}

#[test]
fn open_groups_are_reused_and_closed_group_history_is_preserved_in_a_new_review_plan() {
    use knowmesh_core::domain::ConflictGroupStatus;

    let claims = claims(&[
        "Method X improved accuracy.",
        "Method X did not improve accuracy.",
    ]);
    let context = ClaimComparisonContext::new(&claims).unwrap();
    let initial = context
        .plan(
            &conflict_report(&context, &claims),
            "2026-09-06T01:00:00Z".parse().unwrap(),
        )
        .unwrap();
    let mut updated = materialized(&claims, &initial);
    let context = ClaimComparisonContext::new(&updated).unwrap();
    let repeated = context
        .plan(
            &conflict_report(&context, &updated),
            "2026-09-06T02:00:00Z".parse().unwrap(),
        )
        .unwrap();
    assert!(repeated.claim_changes.is_empty());
    assert!(repeated.groups.is_empty());
    assert_eq!(
        repeated.existing_group_ids,
        vec![initial.groups[0].id.clone()]
    );
    for claim in &mut updated {
        claim.assertion.conflict_groups[0].status = ConflictGroupStatus::Dismissed;
        claim.assertion.conflict_groups[0].resolved_at =
            Some("2026-09-06T02:00:00Z".parse().unwrap());
    }
    let context = ClaimComparisonContext::new(&updated).unwrap();
    let new_plan = context
        .plan(
            &conflict_report(&context, &updated),
            "2026-09-06T03:00:00Z".parse().unwrap(),
        )
        .unwrap();
    assert_ne!(new_plan.groups[0].id, initial.groups[0].id);
    for claim in materialized(&updated, &new_plan) {
        assert_eq!(claim.assertion.conflict_groups.len(), 2);
        assert!(
            claim
                .assertion
                .conflict_groups
                .iter()
                .any(|group| group.id == initial.groups[0].id
                    && group.status == ConflictGroupStatus::Dismissed)
        );
    }
    document(&materialized(&updated, &new_plan))
        .render()
        .unwrap();
}

#[test]
fn overlapping_conflicts_accumulate_on_each_claim_with_original_before_hashes() {
    let claims = claims(&["First result.", "Second result.", "Third result."]);
    let context = ClaimComparisonContext::new(&claims).unwrap();
    let plan = context
        .plan(
            &conflict_report(&context, &claims),
            "2026-09-06T01:00:00Z".parse().unwrap(),
        )
        .unwrap();
    assert_eq!(plan.groups.len(), 3);
    assert_eq!(plan.claim_changes.len(), 3);
    for change in &plan.claim_changes {
        assert_eq!(change.record.assertion.conflict_groups.len(), 2);
        let original = claims
            .iter()
            .find(|claim| claim.assertion.id == change.record.assertion.id)
            .unwrap();
        assert_eq!(
            change.before_semantic_sha256,
            Some(
                original
                    .assertion
                    .semantic_hash(&original.subject_node_id)
                    .unwrap()
            )
        );
    }
    document(&materialized(&claims, &plan)).render().unwrap();
}

#[test]
fn incomplete_duplicate_and_out_of_schema_results_do_not_silently_skip_pairs() {
    let claims = claims(&["First result.", "Second result.", "Third result."]);
    let context = ClaimComparisonContext::new(&claims).unwrap();
    let page = context.select_pairs(&selection(&claims, 32)).unwrap();
    let first = &page.pairs[0];
    let item = json!({"left_id":first.left_id,"right_id":first.right_id,"verdict":"independent","reason":"Different propositions."});
    for results in [vec![item.clone()], vec![item.clone(), item.clone(), item]] {
        let provider = Fake::new(json!({"comparisons":results}));
        assert_eq!(
            context
                .compare(&page.pairs, &provider, &Default::default())
                .unwrap_err()
                .code,
            "CLAIM_COMPARISON_OUTPUT_INVALID"
        );
        assert_eq!(provider.requests.lock().unwrap().len(), 1);
    }
    let provider = Fake::new(
        json!({"comparisons":[{"left_id":first.left_id,"right_id":first.right_id,"verdict":"overwrite_claim","reason":"Not a valid operation."}]}),
    );
    let options = GenerationOptions {
        retry_backoff_ms: 0,
        ..Default::default()
    };
    assert_eq!(
        context
            .compare(&page.pairs, &provider, &options)
            .unwrap_err()
            .code,
        "STRUCTURED_OUTPUT_INVALID"
    );
    assert_eq!(provider.requests.lock().unwrap().len(), 3);
}

#[test]
fn selection_reorders_contexts_and_changes_page_size_without_losing_focused_pairs() {
    let claims = claims(&[
        "First result.",
        "Second result.",
        "Third result.",
        "Fourth result.",
    ]);
    let context = ClaimComparisonContext::new(&claims).unwrap();
    let mut input = PairSelection {
        focus_ids: vec![claims[0].assertion.id.clone()],
        limit: 1,
        cursor: None,
    };
    let first = context.select_pairs(&input).unwrap();
    let mut reversed = claims.clone();
    reversed.reverse();
    let second_context = ClaimComparisonContext::new(&reversed).unwrap();
    assert_eq!(
        first.next_cursor,
        second_context.select_pairs(&input).unwrap().next_cursor
    );
    input.cursor = first.next_cursor;
    input.limit = 32;
    let rest = second_context.select_pairs(&input).unwrap();
    assert_eq!(first.pairs.len() + rest.pairs.len(), 3);
    assert!(rest.next_cursor.is_none());
    assert!(rest.pairs.iter().all(|pair| pair != &first.pairs[0]));
    let empty = context.select_pairs(&PairSelection::default()).unwrap();
    assert!(empty.pairs.is_empty());
    let provider = Fake::new(json!({"comparisons":[]}));
    assert_eq!(
        context
            .compare(&[], &provider, &Default::default())
            .unwrap()
            .usage
            .requests,
        0
    );
    assert!(provider.requests.lock().unwrap().is_empty());
}

#[test]
fn a_claim_at_its_group_limit_blocks_only_that_conflict() {
    use knowmesh_core::{
        application::assertion_compare::ClaimPair,
        domain::{ConflictGroup, ConflictGroupId, ConflictGroupStatus},
    };

    let statements: Vec<_> = (0..130)
        .map(|index| format!("Recorded outcome {index}."))
        .collect();
    let refs: Vec<_> = statements.iter().map(String::as_str).collect();
    let mut claims = claims(&refs);
    for index in 2..130 {
        let mut ids = vec![
            claims[0].assertion.id.clone(),
            claims[index].assertion.id.clone(),
        ];
        ids.sort();
        let group = ConflictGroup {
            id: ConflictGroupId::new(),
            claim_ids: ids,
            reason: "Previously recorded conflict.".into(),
            status: ConflictGroupStatus::Open,
            created_at: "2026-09-06T00:00:00Z".parse().unwrap(),
            resolved_at: None,
        };
        for member in [0, index] {
            claims[member].assertion.evidence_status = EvidenceStatus::Conflicting;
            claims[member].assertion.conflict_groups.push(group.clone());
        }
    }
    let context = ClaimComparisonContext::new(&claims).unwrap();
    let pairs = vec![
        ClaimPair {
            left_id: claims[0].assertion.id.clone(),
            right_id: claims[1].assertion.id.clone(),
        },
        ClaimPair {
            left_id: claims[2].assertion.id.clone(),
            right_id: claims[3].assertion.id.clone(),
        },
    ];
    let output: Vec<_> = pairs.iter().map(|pair| json!({"left_id":pair.left_id,"right_id":pair.right_id,"verdict":"conflicting","reason":"Incompatible outcomes."})).collect();
    let report = context
        .compare(
            &pairs,
            &Fake::new(json!({"comparisons":output})),
            &Default::default(),
        )
        .unwrap();
    let plan = context
        .plan(&report, "2026-09-06T01:00:00Z".parse().unwrap())
        .unwrap();
    assert_eq!(plan.blocked_conflicts.len(), 1);
    assert_eq!(plan.blocked_conflicts[0].code, "CONFLICT_GROUP_LIMIT");
    assert_eq!(plan.groups.len(), 1);
    assert_eq!(plan.claim_changes.len(), 2);
    document(&materialized(&claims, &plan)).render().unwrap();
}

#[test]
fn a_shared_evidence_id_cannot_have_conflicting_payloads_in_comparison_context() {
    let mut claims = claims(&["A positive result.", "A negative result."]);
    claims[1].assertion.evidence[0].id = claims[0].assertion.evidence[0].id.clone();
    assert_eq!(
        ClaimComparisonContext::new(&claims).err().unwrap().code,
        "EVIDENCE_ID_CONFLICT"
    );
}
