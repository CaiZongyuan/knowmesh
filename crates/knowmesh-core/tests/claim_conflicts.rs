#[path = "../../../tests/support/mod.rs"]
mod support;

use std::fs;

use knowmesh_core::{
    canonical::node::NodeDocument,
    domain::{ClaimId, ConflictGroup, ConflictGroupId, ConflictGroupStatus, EvidenceStatus},
};

fn document() -> (tempfile::TempDir, NodeDocument) {
    let (temp, _) = support::fixture();
    let path = temp.path().join("knowledge/nodes/model-a.md");
    let mut document = NodeDocument::parse(&fs::read_to_string(path).unwrap()).unwrap();
    let mut other = document.claims[0].clone();
    other.id = ClaimId::new();
    other.statement = "Model A was not evaluated on Dataset B.".into();
    document.claims.push(other);
    let mut claim_ids: Vec<_> = document
        .claims
        .iter()
        .map(|claim| claim.id.clone())
        .collect();
    claim_ids.sort();
    let group = ConflictGroup {
        id: ConflictGroupId::new(),
        claim_ids,
        reason: "The two statements disagree about the same evaluation.".into(),
        status: ConflictGroupStatus::Open,
        created_at: "2026-09-06T00:00:00Z".parse().unwrap(),
        resolved_at: None,
    };
    for claim in &mut document.claims {
        claim.evidence_status = EvidenceStatus::Conflicting;
        claim.conflict_groups = vec![group.clone()];
    }
    (temp, document)
}

#[test]
fn shared_conflict_records_round_trip_in_canonical_claims() {
    let (_temp, document) = document();
    let text = document.render().unwrap();
    let parsed = NodeDocument::parse(&text).unwrap();
    assert_eq!(parsed.claims.len(), 2);
    assert_eq!(
        parsed.claims[0].conflict_groups,
        parsed.claims[1].conflict_groups
    );
    assert_eq!(parsed.claims[0].conflict_groups[0].claim_ids.len(), 2);
    assert_eq!(parsed.render().unwrap(), text);
}

#[test]
fn changed_or_missing_shared_records_are_rejected_before_rendering() {
    let (_temp, mut document) = document();
    document.claims[0].conflict_groups[0].reason = "Different reason.".into();
    assert_eq!(
        document.render().unwrap_err().code,
        "CONFLICT_GROUP_ID_CONFLICT"
    );
    document.claims[0].conflict_groups = document.claims[1].conflict_groups.clone();
    document.claims[1].conflict_groups.clear();
    assert_eq!(
        document.render().unwrap_err().code,
        "CONFLICT_GROUP_INCOMPLETE"
    );
}

#[test]
fn conflict_members_must_exist_and_have_the_same_qualifier_scope() {
    let (_temp, mut document) = document();
    document.claims[1].qualifiers.insert(
        "cell_type".into(),
        serde_json::json!("Different population"),
    );
    assert_eq!(
        document.render().unwrap_err().code,
        "CONFLICT_SCOPE_MISMATCH"
    );
    document.claims[1].qualifiers.clear();
    document.claims.pop();
    assert_eq!(
        document.render().unwrap_err().code,
        "CONFLICT_CLAIM_MISSING"
    );
}

#[test]
fn open_groups_require_conflicting_status_and_closed_groups_require_a_resolution_time() {
    let (_temp, mut document) = document();
    document.claims[1].evidence_status = EvidenceStatus::Supported;
    assert_eq!(
        document.render().unwrap_err().code,
        "CONFLICT_STATUS_MISMATCH"
    );
    for claim in &mut document.claims {
        claim.conflict_groups[0].status = ConflictGroupStatus::Resolved;
    }
    assert_eq!(
        document.render().unwrap_err().code,
        "INVALID_CONFLICT_GROUP"
    );
    for claim in &mut document.claims {
        claim.conflict_groups[0].resolved_at = Some("2026-09-06T01:00:00Z".parse().unwrap());
    }
    document.render().unwrap();
}

#[test]
fn conflict_changes_affect_assertion_freshness_hashes_without_changing_dedup_identity() {
    let (_temp, mut document) = document();
    let before = document.claims[0]
        .semantic_hash(&document.metadata.id)
        .unwrap();
    let duplicate_key = document.claims[0].normalized_hash().unwrap();
    for claim in &mut document.claims {
        claim.conflict_groups[0].reason = "A reviewed conflict explanation.".into();
    }
    assert_ne!(
        before,
        document.claims[0]
            .semantic_hash(&document.metadata.id)
            .unwrap()
    );
    assert_eq!(duplicate_key, document.claims[0].normalized_hash().unwrap());
}

#[test]
fn invalid_membership_reason_and_resolution_times_are_rejected() {
    let (_temp, document) = document();
    let group = document.claims[0].conflict_groups[0].clone();
    let mut changed = group.clone();
    changed.claim_ids.reverse();
    assert_eq!(
        changed.validate().unwrap_err().code,
        "INVALID_CONFLICT_GROUP"
    );
    let mut changed = group.clone();
    changed.claim_ids[1] = changed.claim_ids[0].clone();
    assert!(changed.validate().is_err());
    let mut changed = group.clone();
    changed.reason = "x".repeat(2049);
    assert!(changed.validate().is_err());
    let mut changed = group;
    changed.status = ConflictGroupStatus::Dismissed;
    changed.resolved_at = Some("2026-09-05T00:00:00Z".parse().unwrap());
    assert!(changed.validate().is_err());
    let mut claim = document.claims[0].clone();
    claim.id = ClaimId::new();
    assert_eq!(
        claim.validate().unwrap_err().code,
        "INVALID_CONFLICT_MEMBERSHIP"
    );
}

#[test]
fn conflict_group_ids_cannot_be_reused_by_another_subject_node() {
    use knowmesh_core::canonical::{snapshot::CanonicalSnapshot, workspace::Workspace};

    let (temp, document) = document();
    fs::write(
        temp.path().join("knowledge/nodes/model-a.md"),
        document.render().unwrap(),
    )
    .unwrap();
    let path = temp.path().join("knowledge/nodes/dataset-b.md");
    let mut other = NodeDocument::parse(&fs::read_to_string(&path).unwrap()).unwrap();
    other.claims = document.claims.clone();
    for claim in &mut other.claims {
        claim.id = ClaimId::new();
    }
    let mut ids: Vec<_> = other.claims.iter().map(|claim| claim.id.clone()).collect();
    ids.sort();
    for claim in &mut other.claims {
        claim.conflict_groups[0].claim_ids = ids.clone();
    }
    fs::write(path, other.render().unwrap()).unwrap();
    let workspace = Workspace::load(temp.path()).unwrap();
    assert_eq!(
        CanonicalSnapshot::scan(&workspace).unwrap_err().code,
        "CONFLICT_GROUP_ID_CONFLICT"
    );
}
