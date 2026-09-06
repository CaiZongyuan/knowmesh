#[path = "../../../tests/support/mod.rs"]
mod support;

use std::slice::from_ref;

use knowmesh_core::{
    application::assertion_dedup::deduplicate,
    canonical::snapshot::CanonicalSnapshot,
    domain::{
        Claim, ClaimId, EvidenceId, EvidenceStance, EvidenceStatus, ExtractionMethod,
        LifecycleStatus, NodeId, Relation, RelationId, SourceRevisionId,
    },
};

fn fixture() -> (tempfile::TempDir, Claim, Relation) {
    let (temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    (
        temp,
        snapshot.claims[0].claim.clone(),
        snapshot.relations[0].relation.clone(),
    )
}

#[test]
fn exact_claim_duplicates_append_evidence_and_preserve_existing_identity_and_metadata() {
    let (_temp, mut existing, _) = fixture();
    existing.assertion.confidence = Some(0.4);
    existing.assertion.evidence_status = EvidenceStatus::Uncertain;
    let mut incoming = existing.clone();
    incoming.assertion.id = ClaimId::new();
    incoming.assertion.statement =
        format!("  {}  ", incoming.assertion.statement.replace(' ', "  "));
    incoming.assertion.confidence = Some(0.95);
    incoming.assertion.evidence_status = EvidenceStatus::Supported;
    incoming.assertion.evidence[0].id = EvidenceId::new();
    incoming.assertion.evidence[0].source_revision_id = SourceRevisionId::new();
    let report = deduplicate(from_ref(&existing), &[], from_ref(&incoming), &[]).unwrap();
    assert_eq!(
        report.claim_aliases[&incoming.assertion.id],
        existing.assertion.id
    );
    assert_eq!(report.claim_changes.len(), 1);
    let change = &report.claim_changes[0];
    assert_eq!(
        change.before_semantic_sha256,
        Some(
            existing
                .assertion
                .semantic_hash(&existing.subject_node_id)
                .unwrap()
        )
    );
    assert_eq!(change.record.assertion.id, existing.assertion.id);
    assert_eq!(
        change.record.assertion.statement,
        existing.assertion.statement
    );
    assert_eq!(change.record.assertion.confidence, Some(0.4));
    assert_eq!(
        change.record.assertion.evidence_status,
        EvidenceStatus::Uncertain
    );
    assert_eq!(change.record.assertion.evidence.len(), 2);
}

#[test]
fn a_new_id_for_the_same_physical_evidence_reuses_the_canonical_record_without_a_write() {
    let (_temp, existing, _) = fixture();
    let mut incoming = existing.clone();
    incoming.assertion.id = ClaimId::new();
    incoming.assertion.evidence[0].id = EvidenceId::new();
    incoming.assertion.evidence[0].confidence = 0.6;
    incoming.assertion.evidence[0].extraction_method = ExtractionMethod::Model;
    let report = deduplicate(from_ref(&existing), &[], from_ref(&incoming), &[]).unwrap();
    assert!(report.claim_changes.is_empty());
    assert_eq!(
        report.evidence_aliases[&incoming.assertion.evidence[0].id],
        existing.assertion.evidence[0].id
    );
}

#[test]
fn source_revision_locator_and_stance_are_part_of_evidence_identity() {
    let (_temp, existing, _) = fixture();
    for field in 0..3 {
        let mut incoming = existing.clone();
        incoming.assertion.id = ClaimId::new();
        let evidence = &mut incoming.assertion.evidence[0];
        evidence.id = EvidenceId::new();
        match field {
            0 => evidence.source_revision_id = SourceRevisionId::new(),
            1 => evidence.locator.paragraph = Some(2),
            _ => evidence.stance = EvidenceStance::Contradicts,
        }
        let report = deduplicate(from_ref(&existing), &[], &[incoming], &[]).unwrap();
        assert_eq!(report.claim_changes[0].record.assertion.evidence.len(), 2);
        if field == 2 {
            assert_eq!(
                report.claim_changes[0].record.assertion.evidence_status,
                EvidenceStatus::Conflicting
            );
        }
    }
}

#[test]
fn case_and_compatibility_characters_in_scientific_statements_are_not_exact_duplicates() {
    let (_temp, mut existing, _) = fixture();
    existing.assertion.evidence.clear();
    existing.assertion.evidence_status = EvidenceStatus::Unreviewed;
    for (left, right) in [
        (
            "The model predicts Co transport.",
            "The model predicts CO transport.",
        ),
        ("Variant X² had an effect.", "Variant X2 had an effect."),
    ] {
        existing.assertion.statement = left.into();
        let mut incoming = existing.clone();
        incoming.assertion.id = ClaimId::new();
        incoming.assertion.statement = right.into();
        let report = deduplicate(from_ref(&existing), &[], from_ref(&incoming), &[]).unwrap();
        assert_eq!(report.claim_changes.len(), 1);
        assert_eq!(report.claim_changes[0].before_semantic_sha256, None);
        assert_eq!(
            report.claim_aliases[&incoming.assertion.id],
            incoming.assertion.id
        );
    }
}

#[test]
fn reusing_an_existing_evidence_or_assertion_id_with_different_content_is_an_error() {
    let (_temp, existing, _) = fixture();
    let mut incoming = existing.clone();
    incoming.assertion.id = ClaimId::new();
    incoming.assertion.evidence[0].confidence = 0.2;
    assert_eq!(
        deduplicate(from_ref(&existing), &[], &[incoming], &[])
            .unwrap_err()
            .code,
        "EVIDENCE_ID_CONFLICT"
    );
    let mut incoming = existing.clone();
    incoming.assertion.statement = "A different assertion.".into();
    assert_eq!(
        deduplicate(&[existing], &[], &[incoming], &[])
            .unwrap_err()
            .code,
        "ASSERTION_ID_CONFLICT"
    );
}

#[test]
fn qualifiers_subjects_and_inactive_history_are_not_merged_or_reactivated() {
    let (_temp, existing, _) = fixture();
    for field in 0..3 {
        let mut old = existing.clone();
        let mut incoming = existing.clone();
        incoming.assertion.id = ClaimId::new();
        match field {
            0 => {
                incoming
                    .assertion
                    .qualifiers
                    .insert("species".into(), serde_json::json!("other"));
            }
            1 => incoming.subject_node_id = NodeId::new(),
            _ => old.assertion.lifecycle_status = LifecycleStatus::Retracted,
        }
        let report = deduplicate(&[old], &[], from_ref(&incoming), &[]).unwrap();
        assert_eq!(report.claim_changes.len(), 1);
        assert_eq!(report.claim_changes[0].before_semantic_sha256, None);
        assert_eq!(
            report.claim_changes[0].record.assertion.id,
            incoming.assertion.id
        );
    }
    let mut inactive = existing.clone();
    inactive.assertion.lifecycle_status = LifecycleStatus::Retracted;
    assert_eq!(
        deduplicate(&[inactive], &[], &[existing], &[])
            .unwrap_err()
            .code,
        "ASSERTION_LIFECYCLE_CONFLICT"
    );
}

#[test]
fn undirected_relations_merge_reversed_endpoints_while_directed_relations_keep_orientation() {
    let (_temp, _, mut existing) = fixture();
    existing.assertion.predicate = "compared_with".into();
    for directed in [false, true] {
        existing.assertion.directed = directed;
        let mut incoming = existing.clone();
        incoming.assertion.id = RelationId::new();
        std::mem::swap(
            &mut incoming.source_node_id,
            &mut incoming.assertion.target_node_id,
        );
        let report = deduplicate(&[], from_ref(&existing), &[], from_ref(&incoming)).unwrap();
        if directed {
            assert_eq!(report.relation_changes.len(), 1);
            assert_eq!(
                report.relation_aliases[&incoming.assertion.id],
                incoming.assertion.id
            );
        } else {
            assert!(report.relation_changes.is_empty());
            assert_eq!(
                report.relation_aliases[&incoming.assertion.id],
                existing.assertion.id
            );
        }
    }
}

#[test]
fn duplicates_within_one_candidate_batch_are_order_independent() {
    let (_temp, mut first, _) = fixture();
    first.assertion.id = ClaimId::new();
    let mut second = first.clone();
    second.assertion.id = ClaimId::new();
    second.assertion.evidence[0].id = EvidenceId::new();
    second.assertion.evidence[0].source_revision_id = SourceRevisionId::new();
    let forward = deduplicate(&[], &[], &[first.clone(), second.clone()], &[]).unwrap();
    let reverse = deduplicate(&[], &[], &[second.clone(), first.clone()], &[]).unwrap();
    assert_eq!(
        serde_json::to_value(&forward).unwrap(),
        serde_json::to_value(&reverse).unwrap()
    );
    assert_eq!(forward.claim_changes.len(), 1);
    assert_eq!(forward.claim_changes[0].record.assertion.evidence.len(), 2);
    assert_eq!(
        forward.claim_aliases[&first.assertion.id],
        forward.claim_aliases[&second.assertion.id]
    );
}

fn conflicting_pair(first: &Claim) -> Vec<Claim> {
    use knowmesh_core::domain::{ConflictGroup, ConflictGroupId, ConflictGroupStatus};

    let mut first = first.clone();
    let mut second = first.clone();
    second.assertion.id = ClaimId::new();
    second.assertion.statement = "Model A was not evaluated on Dataset B.".into();
    let mut ids = vec![first.assertion.id.clone(), second.assertion.id.clone()];
    ids.sort();
    let group = ConflictGroup {
        id: ConflictGroupId::new(),
        claim_ids: ids,
        reason: "Opposite statements in the same scope.".into(),
        status: ConflictGroupStatus::Open,
        created_at: "2026-09-06T00:00:00Z".parse().unwrap(),
        resolved_at: None,
    };
    for claim in [&mut first, &mut second] {
        claim.assertion.evidence_status = EvidenceStatus::Conflicting;
        claim.assertion.conflict_groups = vec![group.clone()];
    }
    vec![first, second]
}

#[test]
fn deduplication_preserves_existing_conflict_records_and_rejects_candidate_predeclarations() {
    let (_temp, existing, _) = fixture();
    let pair = conflicting_pair(&existing);
    let mut incoming = pair[0].clone();
    incoming.assertion.id = ClaimId::new();
    incoming.assertion.conflict_groups.clear();
    incoming.assertion.evidence[0].id = EvidenceId::new();
    incoming.assertion.evidence[0].source_revision_id = SourceRevisionId::new();
    let report = deduplicate(&pair, &[], &[incoming], &[]).unwrap();
    assert_eq!(
        report.claim_changes[0].record.assertion.conflict_groups,
        pair[0].assertion.conflict_groups
    );
    assert_eq!(
        report.claim_changes[0].before_semantic_sha256,
        Some(
            pair[0]
                .assertion
                .semantic_hash(&pair[0].subject_node_id)
                .unwrap()
        )
    );
    assert_eq!(
        deduplicate(&pair, &[], &pair, &[]).unwrap_err().code,
        "INVALID_DEDUP_CANDIDATE"
    );
}

#[test]
fn newly_extracted_evidence_prefers_the_record_already_owned_by_the_duplicate_target() {
    let (_temp, mut existing, mut relation) = fixture();
    existing.assertion.evidence[0].id = "evd_01ARZ3NDEKTSV4RRFFQ69G5FAZ".parse().unwrap();
    relation.assertion.evidence[0].id = "evd_01ARZ3NDEKTSV4RRFFQ69G5FAA".parse().unwrap();
    let mut incoming = existing.clone();
    incoming.assertion.id = ClaimId::new();
    incoming.assertion.evidence[0].id = EvidenceId::new();
    let report = deduplicate(from_ref(&existing), &[relation], from_ref(&incoming), &[]).unwrap();
    assert!(report.claim_changes.is_empty());
    assert!(report.relation_changes.is_empty());
    assert_eq!(
        report.evidence_aliases[&incoming.assertion.evidence[0].id],
        existing.assertion.evidence[0].id
    );
}

#[test]
fn ambiguous_existing_assertions_and_reused_conflict_groups_are_invalid_contexts() {
    let (_temp, existing, relation) = fixture();
    let mut duplicate = relation.clone();
    duplicate.assertion.id = RelationId::new();
    assert_eq!(
        deduplicate(&[], &[relation, duplicate], &[], &[])
            .unwrap_err()
            .code,
        "AMBIGUOUS_ASSERTION_DUPLICATE"
    );
    let first_pair = conflicting_pair(&existing);
    let mut other = existing;
    other.subject_node_id = NodeId::new();
    other.assertion.id = ClaimId::new();
    let mut second_pair = conflicting_pair(&other);
    for claim in &mut second_pair {
        claim.assertion.conflict_groups[0].id =
            first_pair[0].assertion.conflict_groups[0].id.clone();
    }
    let all: Vec<_> = first_pair.into_iter().chain(second_pair).collect();
    assert_eq!(
        deduplicate(&all, &[], &[], &[]).unwrap_err().code,
        "CONFLICT_GROUP_ID_CONFLICT"
    );
}
