use std::collections::BTreeMap;

use knowmesh_core::domain::{
    AssertionDependency, ClaimId, DependencySnapshot, EvidenceId, SourceHead, SourceId,
    SourceRevisionId,
    freshness::{
        AssertionState, Freshness, FreshnessContext, FreshnessReasonCode, SourceState,
        assertion_freshness, synthesis_freshness,
    },
    sha256,
};

fn context() -> (FreshnessContext, Vec<EvidenceId>) {
    let evidence = vec![EvidenceId::new(), EvidenceId::new()];
    let mut context = FreshnessContext {
        index_complete: true,
        ..Default::default()
    };
    for id in &evidence {
        let source = SourceId::new();
        let revision = SourceRevisionId::new();
        context.sources.insert(
            source.clone(),
            SourceState {
                current_revision_id: revision.clone(),
                removed: false,
            },
        );
        context.revisions.insert(revision.clone(), source);
        context.evidence.insert(id.clone(), revision);
    }
    (context, evidence)
}

#[test]
fn revised_and_removed_sources_require_review_without_dropping_independent_evidence() {
    let (mut context, evidence) = context();
    assert_eq!(
        assertion_freshness(&evidence, &context).freshness,
        Freshness::Current
    );
    let source = context.revisions[&context.evidence[&evidence[0]]].clone();
    context
        .sources
        .get_mut(&source)
        .unwrap()
        .current_revision_id = SourceRevisionId::new();
    let report = assertion_freshness(&evidence, &context);
    assert_eq!(report.freshness, Freshness::NeedsReview);
    assert_eq!(report.evidence_ids.len(), 2);
    assert_eq!(report.current_evidence_ids, vec![evidence[1].clone()]);
    assert_eq!(
        report.freshness_reasons[0].code,
        FreshnessReasonCode::SourceRevisionBehind
    );
    context.sources.get_mut(&source).unwrap().removed = true;
    let report = assertion_freshness(&evidence, &context);
    assert_eq!(report.freshness, Freshness::NeedsReview);
    assert_eq!(report.evidence_ids.len(), 2);
    assert!(
        report
            .freshness_reasons
            .iter()
            .any(|reason| reason.code == FreshnessReasonCode::SourceRemoved)
    );
    assert_eq!(report.current_evidence_ids, vec![evidence[1].clone()]);
}

#[test]
fn synthesis_compares_assertion_hashes_and_heads_and_inherits_assertion_evidence() {
    let (mut context, evidence) = context();
    let assertion = AssertionDependency::Claim {
        id: ClaimId::new(),
        semantic_sha256: sha256(b"accepted assertion"),
    };
    context.assertions.insert(
        assertion.id().to_owned(),
        AssertionState {
            dependency: assertion.clone(),
            evidence_ids: evidence.clone(),
        },
    );
    let revision = context.evidence[&evidence[0]].clone();
    let source = context.revisions[&revision].clone();
    let snapshot = DependencySnapshot {
        version: 1,
        assertions: vec![assertion.clone()],
        source_heads: vec![SourceHead {
            source_id: source.clone(),
            revision_id: revision,
        }],
    };
    let report = synthesis_freshness(&[], Some(&snapshot), &context);
    assert_eq!(report.freshness, Freshness::Current);
    assert_eq!(report.evidence_ids.len(), 2);
    let AssertionDependency::Claim {
        semantic_sha256, ..
    } = &mut context
        .assertions
        .get_mut(assertion.id())
        .unwrap()
        .dependency
    else {
        unreachable!()
    };
    *semantic_sha256 = sha256(b"retracted assertion");
    let report = synthesis_freshness(&[], Some(&snapshot), &context);
    assert_eq!(report.freshness, Freshness::NeedsReview);
    assert!(report.freshness_reasons.iter().any(|reason| reason.code
        == FreshnessReasonCode::DependencyChanged
        && reason.dependency_ids.contains(&assertion.id().to_owned())));
    context
        .sources
        .get_mut(&source)
        .unwrap()
        .current_revision_id = SourceRevisionId::new();
    let report = synthesis_freshness(&[], Some(&snapshot), &context);
    assert!(report.freshness_reasons.iter().any(|reason| reason.code
        == FreshnessReasonCode::DependencyChanged
        && reason.dependency_ids.contains(&source.to_string())));
    assert!(
        report
            .freshness_reasons
            .iter()
            .any(|reason| reason.code == FreshnessReasonCode::SourceRevisionBehind)
    );
}

#[test]
fn missing_snapshots_dependencies_or_incomplete_index_never_report_current() {
    let (mut context, evidence) = context();
    assert_eq!(
        synthesis_freshness(&evidence, None, &context).freshness,
        Freshness::Unknown
    );
    let assertion = AssertionDependency::Claim {
        id: ClaimId::new(),
        semantic_sha256: sha256(b"missing"),
    };
    let snapshot = DependencySnapshot {
        version: 1,
        assertions: vec![assertion],
        source_heads: vec![],
    };
    let report = synthesis_freshness(&evidence, Some(&snapshot), &context);
    assert_eq!(report.freshness, Freshness::Unknown);
    assert!(
        report
            .freshness_reasons
            .iter()
            .any(|reason| reason.code == FreshnessReasonCode::DependencyMissing)
    );
    context.index_complete = false;
    assert_eq!(
        assertion_freshness(&evidence, &context).freshness,
        Freshness::Unknown
    );
    context.index_complete = true;
    context.sources = BTreeMap::new();
    assert_eq!(
        assertion_freshness(&evidence, &context).freshness,
        Freshness::Unknown
    );
    context.evidence.clear();
    assert_eq!(
        assertion_freshness(&evidence, &context).freshness,
        Freshness::Unknown
    );
}

#[test]
fn freshness_reasons_are_deterministic_and_unknown_takes_precedence_over_changes() {
    let (mut context, mut evidence) = context();
    context
        .sources
        .values_mut()
        .for_each(|source| source.removed = true);
    context.index_complete = false;
    let expected = assertion_freshness(&evidence, &context);
    evidence.reverse();
    evidence.push(evidence[0].clone());
    assert_eq!(assertion_freshness(&evidence, &context), expected);
    assert_eq!(expected.freshness, Freshness::Unknown);
    assert!(
        expected
            .freshness_reasons
            .iter()
            .any(|reason| reason.code == FreshnessReasonCode::IndexIncomplete)
    );
    assert!(
        expected
            .freshness_reasons
            .iter()
            .any(|reason| reason.code == FreshnessReasonCode::SourceRemoved)
    );
}

#[test]
fn an_incomplete_index_cannot_identify_evidence_as_current() {
    let (mut context, evidence) = context();
    context.index_complete = false;
    let report = assertion_freshness(&evidence, &context);
    assert_eq!(report.evidence_ids.len(), evidence.len());
    assert!(report.current_evidence_ids.is_empty());
}
