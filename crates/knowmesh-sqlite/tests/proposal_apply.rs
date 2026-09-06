#[path = "../../../tests/support/mod.rs"]
mod support;

use std::fs;

use knowmesh_core::{
    application::{
        proposal::{
            ProposalRecord,
            apply::{self, ApplyInput},
            prepare,
        },
        sync,
    },
    canonical::{node::NodeDocument, snapshot::CanonicalSnapshot, workspace::Workspace},
    domain::{
        Timestamp,
        proposal::{
            PatchOp, ProposalInput, ProposalItem, ProposalKind, ProposalState, ReviewInput,
            ReviewPolicy,
        },
    },
    ports::{ProjectionStore, ProposalStore},
};
use knowmesh_sqlite::SqliteStore;
use rusqlite::Connection;
use serde_json::json;

fn now() -> Timestamp {
    "2026-09-06T02:00:00Z".parse().unwrap()
}

fn fixture() -> (
    tempfile::TempDir,
    Workspace,
    SqliteStore,
    ProposalRecord,
    ApplyInput,
) {
    let (temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let mut store = SqliteStore::open(&workspace.index_path().unwrap()).unwrap();
    store
        .bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    let generation = store.reconcile(&snapshot).unwrap().generation;
    let prepared = prepare(
        &workspace,
        &ProposalInput {
            kind: ProposalKind::Manual,
            base_generation: generation,
            schema_hash: snapshot.schema_hash,
            source_revision_id: None,
            compiler_run_id: None,
            summary: "Add two reviewed aliases.".into(),
            items: snapshot
                .nodes
                .iter()
                .map(|node| {
                    ProposalItem::new(
                        PatchOp::AddAlias,
                        node.metadata.id.to_string(),
                        json!({"alias":format!("Reviewed {}",node.metadata.name)}),
                    )
                    .unwrap()
                })
                .collect(),
        },
        "author",
        now(),
    )
    .unwrap();
    let mut record = ProposalRecord {
        proposal: prepared.proposal,
        base_snapshot_sha256: prepared.base_snapshot_sha256,
    };
    store.proposal_create(&record).unwrap();
    record.proposal = record
        .proposal
        .review(
            &ReviewInput {
                expected_revision: 1,
                accept_all: true,
                decisions: vec![],
            },
            &ReviewPolicy::default(),
            "reviewer",
            now(),
        )
        .unwrap();
    store.proposal_save(1, &record).unwrap();
    let input = ApplyInput {
        proposal_id: record.proposal.id.clone(),
        expected_revision: 2,
        dry_run: false,
        yes: true,
    };
    (temp, workspace, store, record, input)
}

#[test]
fn apply_commits_canonical_index_review_history_and_a_repeatable_receipt() {
    let (_temp, workspace, mut store, approved, input) = fixture();
    let report = apply::execute(&workspace, &mut store, &input, "author", now()).unwrap();
    assert!(!report.dry_run);
    assert_eq!(report.changed_paths.len(), 2);
    assert!(report.transaction_id.is_some());
    assert_eq!(report.projection.as_ref().unwrap().generation, 2);
    assert_eq!(report.applied_revision, Some(3));
    let current = store.proposal_get(&input.proposal_id, None).unwrap();
    assert_eq!(current.proposal.state, ProposalState::Applied);
    assert_eq!(current.proposal.applied_generation, Some(2));
    assert_eq!(
        store.proposal_get(&input.proposal_id, Some(2)).unwrap(),
        approved
    );
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    assert!(snapshot.nodes.iter().all(|node| {
        node.metadata
            .aliases
            .contains(&format!("Reviewed {}", node.metadata.name))
    }));
    assert!(!sync::recovery_status(&workspace).unwrap().recovery_required);
    let repeated = apply::execute(&workspace, &mut store, &input, "retry", now()).unwrap();
    assert_eq!(
        serde_json::to_value(&repeated).unwrap(),
        serde_json::to_value(&report).unwrap()
    );
    let db = Connection::open(workspace.index_path().unwrap()).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM proposal_revisions", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM proposal_applications", [], |row| row
            .get::<_, u64>(
            0
        ))
        .unwrap(),
        1
    );
}

#[test]
fn apply_preview_and_missing_confirmation_do_not_mutate_runtime_or_canonical() {
    let (_temp, workspace, mut store, approved, mut input) = fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    input.yes = false;
    assert_eq!(
        apply::execute(&workspace, &mut store, &input, "author", now())
            .unwrap_err()
            .code,
        "CONFIRMATION_REQUIRED"
    );
    input.dry_run = true;
    let report = apply::execute(&workspace, &mut store, &input, "author", now()).unwrap();
    assert!(report.dry_run);
    assert_eq!(report.changed_paths.len(), 2);
    assert!(report.transaction_id.is_none());
    assert!(report.applied_revision.is_none());
    assert_eq!(
        store.proposal_get(&input.proposal_id, None).unwrap(),
        approved
    );
    assert_eq!(
        CanonicalSnapshot::scan(&workspace).unwrap().content_sha256,
        before.content_sha256
    );
    assert!(
        store
            .proposal_application(&input.proposal_id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn stale_content_and_wrong_revisions_fail_before_any_file_journal() {
    let (_temp, workspace, mut store, _approved, mut input) = fixture();
    input.expected_revision = 1;
    assert_eq!(
        apply::execute(&workspace, &mut store, &input, "author", now())
            .unwrap_err()
            .code,
        "PROPOSAL_REVISION_MISMATCH"
    );
    input.expected_revision = 2;
    let path = workspace.root.join("knowledge/nodes/model-a.md");
    let mut doc = NodeDocument::parse(&fs::read_to_string(&path).unwrap()).unwrap();
    doc.metadata.aliases.push("External edit".into());
    fs::write(path, doc.render().unwrap()).unwrap();
    assert_eq!(
        apply::execute(&workspace, &mut store, &input, "author", now())
            .unwrap_err()
            .code,
        "STALE_PROPOSAL"
    );
    assert!(!sync::recovery_status(&workspace).unwrap().recovery_required);
    assert!(
        store
            .proposal_application(&input.proposal_id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn failure_after_file_changes_rolls_back_sql_and_recovers_the_reviewed_revision() {
    let (_temp, workspace, mut store, approved, input) = fixture();
    let db = Connection::open(workspace.index_path().unwrap()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_apply_history BEFORE INSERT ON proposal_revisions WHEN NEW.revision=3 BEGIN SELECT RAISE(ABORT,'injected apply failure'); END;").unwrap();
    assert!(apply::execute(&workspace, &mut store, &input, "author", now()).is_err());
    assert!(sync::recovery_status(&workspace).unwrap().recovery_required);
    assert_eq!(
        store.proposal_get(&input.proposal_id, None).unwrap(),
        approved
    );
    assert_eq!(
        db.query_row(
            "SELECT indexed_generation FROM workspace_state",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert!(
        store
            .proposal_application(&input.proposal_id)
            .unwrap()
            .is_none()
    );
    for filename in ["model-a.md", "dataset-b.md"] {
        assert!(
            fs::read_to_string(workspace.root.join("knowledge/nodes").join(filename))
                .unwrap()
                .contains("Reviewed ")
        );
    }
    db.execute_batch("DROP TRIGGER fail_apply_history;")
        .unwrap();
    let pending = sync::recovery_status(&workspace).unwrap();
    let snapshot =
        CanonicalSnapshot::scan_committed(&workspace, &pending.transactions[0].id).unwrap();
    assert_eq!(
        store.reconcile(&snapshot).unwrap_err().code,
        "PROPOSAL_APPLY_COORDINATOR_REQUIRED"
    );
    let recovered = sync::recover(&workspace, &mut store).unwrap();
    assert_eq!(recovered.recovered_transaction_ids.len(), 1);
    assert_eq!(recovered.projection.unwrap().generation, 2);
    assert_eq!(
        store
            .proposal_get(&input.proposal_id, None)
            .unwrap()
            .proposal
            .state,
        ProposalState::Applied
    );
    let repeated = apply::execute(&workspace, &mut store, &input, "retry", now()).unwrap();
    assert_eq!(repeated.projection.unwrap().generation, 2);
}

#[test]
fn recovery_refuses_to_apply_a_journal_after_its_reviewed_revision_changes() {
    let (_temp, workspace, mut store, approved, input) = fixture();
    let db = Connection::open(workspace.index_path().unwrap()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_apply_history BEFORE INSERT ON proposal_revisions WHEN NEW.revision=3 BEGIN SELECT RAISE(ABORT,'injected apply failure'); END;").unwrap();
    assert!(apply::execute(&workspace, &mut store, &input, "author", now()).is_err());
    db.execute_batch("DROP TRIGGER fail_apply_history;")
        .unwrap();
    let rejected = ProposalRecord {
        proposal: approved
            .proposal
            .reject(2, "Review changed during interruption", "reviewer", now())
            .unwrap(),
        ..approved
    };
    store.proposal_save(2, &rejected).unwrap();
    assert_eq!(
        sync::recover(&workspace, &mut store).unwrap_err().code,
        "PROPOSAL_REVISION_MISMATCH"
    );
    assert!(sync::recovery_status(&workspace).unwrap().recovery_required);
    assert_eq!(
        store.proposal_get(&input.proposal_id, None).unwrap(),
        rejected
    );
    assert_eq!(
        db.query_row(
            "SELECT indexed_generation FROM workspace_state",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn recovery_rechecks_referenced_evidence_bytes_after_an_interruption() {
    use knowmesh_core::{
        canonical::source::SourceFile,
        domain::{ClaimId, StorageMode},
    };
    let (temp, workspace) = support::fixture();
    let source_path = workspace.root.join("sources/fixture/source.yaml");
    let mut source = SourceFile::parse(
        "sources/fixture/source.yaml".into(),
        &fs::read(&source_path).unwrap(),
    )
    .unwrap()
    .manifest;
    let original = fs::read(
        source_path
            .parent()
            .unwrap()
            .join(&source.revisions[0].path),
    )
    .unwrap();
    let referenced_path = temp.path().join("referenced.md");
    fs::write(&referenced_path, original).unwrap();
    source.storage = StorageMode::Referenced;
    source.revisions[0].path = referenced_path.to_str().unwrap().into();
    fs::write(&source_path, serde_yaml::to_string(&source).unwrap()).unwrap();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let mut store = SqliteStore::open(&workspace.index_path().unwrap()).unwrap();
    store
        .bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash)
        .unwrap();
    store.reconcile(&snapshot).unwrap();
    let mut claim = snapshot.claims[0].claim.assertion.clone();
    claim.id = ClaimId::new();
    claim.statement = "A newly reviewed assertion backed by the external source.".into();
    let prepared = prepare(
        &workspace,
        &ProposalInput {
            kind: ProposalKind::Compile,
            base_generation: 1,
            schema_hash: snapshot.schema_hash,
            source_revision_id: Some(source.current_revision_id),
            compiler_run_id: None,
            summary: "Reviewed source assertion.".into(),
            items: vec![
                ProposalItem::new(
                    PatchOp::AddClaim,
                    snapshot.claims[0].claim.subject_node_id.to_string(),
                    json!({"claim":claim}),
                )
                .unwrap(),
            ],
        },
        "author",
        now(),
    )
    .unwrap();
    let mut record = ProposalRecord {
        proposal: prepared.proposal,
        base_snapshot_sha256: prepared.base_snapshot_sha256,
    };
    store.proposal_create(&record).unwrap();
    record.proposal = record
        .proposal
        .review(
            &ReviewInput {
                expected_revision: 1,
                accept_all: true,
                decisions: vec![],
            },
            &ReviewPolicy::default(),
            "reviewer",
            now(),
        )
        .unwrap();
    store.proposal_save(1, &record).unwrap();
    let input = ApplyInput {
        proposal_id: record.proposal.id.clone(),
        expected_revision: 2,
        dry_run: false,
        yes: true,
    };
    let db = Connection::open(workspace.index_path().unwrap()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_apply_history BEFORE INSERT ON proposal_revisions WHEN NEW.revision=3 BEGIN SELECT RAISE(ABORT,'injected apply failure'); END;").unwrap();
    assert!(apply::execute(&workspace, &mut store, &input, "author", now()).is_err());
    db.execute_batch("DROP TRIGGER fail_apply_history;")
        .unwrap();
    fs::write(&referenced_path, "The referenced source was changed.").unwrap();
    assert_eq!(
        sync::recover(&workspace, &mut store).unwrap_err().code,
        "SOURCE_REVISION_CHANGED"
    );
    assert_eq!(
        store.proposal_get(&input.proposal_id, None).unwrap(),
        record
    );
    assert_eq!(store.generation().unwrap(), 1);
    assert!(sync::recovery_status(&workspace).unwrap().recovery_required);
}

#[test]
fn every_partial_file_state_recovers_with_one_completed_application() {
    for already_replaced in 0..=2 {
        let (_temp, workspace, mut store, _approved, input) = fixture();
        let before = CanonicalSnapshot::scan(&workspace).unwrap();
        let original: std::collections::BTreeMap<_, _> = before
            .nodes
            .iter()
            .map(|node| {
                (
                    node.canonical_path.clone(),
                    fs::read(workspace.root.join(&node.canonical_path)).unwrap(),
                )
            })
            .collect();
        let db = Connection::open(workspace.index_path().unwrap()).unwrap();
        db.execute_batch("CREATE TRIGGER fail_apply_history BEFORE INSERT ON proposal_revisions WHEN NEW.revision=3 BEGIN SELECT RAISE(ABORT,'injected apply failure'); END;").unwrap();
        apply::execute(&workspace, &mut store, &input, "author", now()).unwrap_err();
        db.execute_batch("DROP TRIGGER fail_apply_history;")
            .unwrap();
        let pending = sync::recovery_status(&workspace).unwrap();
        for path in pending.transactions[0].paths.iter().skip(already_replaced) {
            fs::write(workspace.root.join(path), &original[path]).unwrap();
        }
        let manifest_path = workspace
            .root
            .join(".knowmesh/transactions")
            .join(&pending.transactions[0].id)
            .join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["state"] = json!("prepared");
        fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let recovered = sync::recover(&workspace, &mut store).unwrap();
        assert_eq!(recovered.projection.unwrap().generation, 2);
        assert_eq!(
            store
                .proposal_get(&input.proposal_id, None)
                .unwrap()
                .proposal
                .state,
            ProposalState::Applied
        );
        assert!(!sync::recovery_status(&workspace).unwrap().recovery_required);
    }
}

struct InterruptedAfterCommit<'a>(&'a mut SqliteStore);

impl ProjectionStore for InterruptedAfterCommit<'_> {
    fn reconcile(
        &mut self,
        snapshot: &CanonicalSnapshot,
    ) -> knowmesh_core::error::AppResult<knowmesh_core::ports::ReconcileReport> {
        self.0.reconcile(snapshot)
    }
}
impl knowmesh_core::ports::IndexStore for InterruptedAfterCommit<'_> {
    fn projection_state(
        &self,
    ) -> knowmesh_core::error::AppResult<knowmesh_core::ports::ProjectionState> {
        self.0.projection_state()
    }
    fn diagnostics(
        &self,
    ) -> knowmesh_core::error::AppResult<knowmesh_core::ports::DatabaseDiagnostics> {
        self.0.diagnostics()
    }
    fn apply_proposal(
        &mut self,
        context: &apply::ApplyContext,
        canonical: &mut dyn FnMut() -> knowmesh_core::error::AppResult<apply::CanonicalApplication>,
    ) -> knowmesh_core::error::AppResult<apply::ApplyReport> {
        self.0.apply_proposal(context, canonical)?;
        Err(knowmesh_core::error::AppError::new(
            knowmesh_core::error::ErrorType::Io,
            "INJECTED_AFTER_COMMIT",
            "Simulated interruption after database commit.",
        ))
    }
}
impl ProposalStore for InterruptedAfterCommit<'_> {
    fn proposal_create(&mut self, record: &ProposalRecord) -> knowmesh_core::error::AppResult<()> {
        self.0.proposal_create(record)
    }
    fn proposal_get(
        &self,
        id: &knowmesh_core::domain::ProposalId,
        revision: Option<u32>,
    ) -> knowmesh_core::error::AppResult<ProposalRecord> {
        self.0.proposal_get(id, revision)
    }
    fn proposal_save(
        &mut self,
        expected: u32,
        record: &ProposalRecord,
    ) -> knowmesh_core::error::AppResult<()> {
        self.0.proposal_save(expected, record)
    }
    fn proposal_application(
        &self,
        id: &knowmesh_core::domain::ProposalId,
    ) -> knowmesh_core::error::AppResult<Option<apply::ApplyReceipt>> {
        self.0.proposal_application(id)
    }
}

#[test]
fn doctor_and_apply_retry_finish_a_committed_receipt_without_new_history() {
    use knowmesh_core::application::doctor;
    for doctor_recovery in [false, true] {
        let (_temp, workspace, mut store, _approved, input) = fixture();
        let mut interrupted = InterruptedAfterCommit(&mut store);
        assert_eq!(
            apply::execute(&workspace, &mut interrupted, &input, "author", now())
                .unwrap_err()
                .code,
            "INJECTED_AFTER_COMMIT"
        );
        let receipt = store
            .proposal_application(&input.proposal_id)
            .unwrap()
            .unwrap();
        assert_eq!(store.generation().unwrap(), 2);
        assert!(sync::recovery_status(&workspace).unwrap().recovery_required);
        if doctor_recovery {
            let repaired = doctor::repair_root(
                &workspace.root,
                doctor::IndexAccess::Ready(&store),
                &doctor::RepairInput {
                    dry_run: false,
                    yes: true,
                },
                |workspace| Ok(Box::new(SqliteStore::open(&workspace.index_path()?)?)),
            )
            .unwrap();
            assert!(!repaired.recovery.unwrap().recovery_required);
        }
        let repeated = apply::execute(&workspace, &mut store, &input, "retry", now()).unwrap();
        assert_eq!(
            serde_json::to_value(repeated).unwrap(),
            serde_json::to_value(receipt.report).unwrap()
        );
        assert_eq!(store.generation().unwrap(), 2);
        let db = Connection::open(workspace.index_path().unwrap()).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM proposal_revisions", [], |row| row
                .get::<_, u64>(0))
                .unwrap(),
            3
        );
        assert!(!sync::recovery_status(&workspace).unwrap().recovery_required);
    }
}

fn persist_approved(store: &mut SqliteStore, mut record: ProposalRecord) -> ApplyInput {
    store.proposal_create(&record).unwrap();
    record.proposal = record
        .proposal
        .review(
            &ReviewInput {
                expected_revision: 1,
                accept_all: true,
                decisions: vec![],
            },
            &ReviewPolicy::default(),
            "reviewer",
            now(),
        )
        .unwrap();
    store.proposal_save(1, &record).unwrap();
    ApplyInput {
        proposal_id: record.proposal.id,
        expected_revision: 2,
        dry_run: false,
        yes: true,
    }
}

#[test]
fn real_apply_rejects_unverifiable_claim_relation_and_synthesis_references_even_after_raw_approval()
{
    use knowmesh_core::domain::{
        ClaimId, EvidenceId, RelationId, SynthesisId, proposal::Proposal, sha256,
    };
    for (operation, invalid_locator) in [
        (PatchOp::AddClaim, false),
        (PatchOp::AddRelation, false),
        (PatchOp::AddClaim, true),
        (PatchOp::AddRelation, true),
        (PatchOp::CreateSynthesis, false),
    ] {
        let (_temp, workspace, mut store, _approved, _) = fixture();
        let before = CanonicalSnapshot::scan(&workspace).unwrap();
        let mut evidence = before.evidence[0].evidence.clone();
        evidence.id = EvidenceId::new();
        if invalid_locator {
            evidence.locator.page = Some(999);
        } else {
            evidence.quote = "This quote does not occur in the source.".into();
        }
        evidence.quote_sha256 = sha256(evidence.quote.as_bytes());
        let mut item = match operation {
            PatchOp::AddClaim => {
                let mut claim = before.claims[0].claim.assertion.clone();
                claim.id = ClaimId::new();
                claim.statement = "A proposed assertion.".into();
                claim.evidence = vec![evidence.clone()];
                ProposalItem::new(
                    operation,
                    before.claims[0].claim.subject_node_id.to_string(),
                    json!({"claim":claim}),
                )
                .unwrap()
            }
            PatchOp::AddRelation => {
                let mut relation = before.relations[0].relation.assertion.clone();
                relation.id = RelationId::new();
                relation.evidence = vec![evidence.clone()];
                ProposalItem::new(
                    operation,
                    before.relations[0].relation.source_node_id.to_string(),
                    json!({"relation":relation}),
                )
                .unwrap()
            }
            _ => {
                let mut metadata = before.syntheses[0].metadata.clone();
                metadata.id = SynthesisId::new();
                metadata.evidence_ids = vec![evidence.id.clone()];
                ProposalItem::new(operation, metadata.id.to_string(), json!({"metadata":metadata,"body":format!("A missing reference. [@{}]",evidence.id)})).unwrap()
            }
        };
        item.evidence_ids = vec![evidence.id];
        let proposal = Proposal::new(
            ProposalInput {
                kind: ProposalKind::Manual,
                base_generation: 1,
                schema_hash: before.schema_hash,
                source_revision_id: None,
                compiler_run_id: None,
                summary: "Unverified proposed content.".into(),
                items: vec![item],
            },
            "author",
            now(),
        )
        .unwrap();
        let input = persist_approved(
            &mut store,
            ProposalRecord {
                proposal,
                base_snapshot_sha256: before.content_sha256.clone(),
            },
        );
        assert_eq!(
            apply::execute(&workspace, &mut store, &input, "author", now())
                .unwrap_err()
                .code,
            "PROPOSAL_ACCEPTED_ITEMS_INVALID"
        );
        assert_eq!(
            CanonicalSnapshot::scan(&workspace).unwrap().content_sha256,
            before.content_sha256
        );
        assert_eq!(store.generation().unwrap(), 1);
        assert!(!sync::recovery_status(&workspace).unwrap().recovery_required);
    }
}

#[test]
fn verified_compiler_assertions_are_applied_with_immutable_source_bindings() {
    let (_temp, workspace, mut store, _approved, _) = fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let mut claim = before.claims[0].claim.assertion.clone();
    claim.id = knowmesh_core::domain::ClaimId::new();
    claim.statement = "The evaluation is documented in the cited source.".into();
    let mut relation = before.relations[0].relation.assertion.clone();
    relation.id = knowmesh_core::domain::RelationId::new();
    relation
        .qualifiers
        .insert("scope".into(), json!("documented evaluation"));
    let prepared = prepare(
        &workspace,
        &ProposalInput {
            kind: ProposalKind::Compile,
            base_generation: 1,
            schema_hash: before.schema_hash,
            source_revision_id: Some(before.sources[0].manifest.current_revision_id.clone()),
            compiler_run_id: None,
            summary: "Verified compiler assertions.".into(),
            items: vec![
                ProposalItem::new(
                    PatchOp::AddClaim,
                    before.claims[0].claim.subject_node_id.to_string(),
                    json!({"claim":claim}),
                )
                .unwrap(),
                ProposalItem::new(
                    PatchOp::AddRelation,
                    before.relations[0].relation.source_node_id.to_string(),
                    json!({"relation":relation}),
                )
                .unwrap(),
            ],
        },
        "author",
        now(),
    )
    .unwrap();
    let input = persist_approved(
        &mut store,
        ProposalRecord {
            proposal: prepared.proposal,
            base_snapshot_sha256: prepared.base_snapshot_sha256,
        },
    );
    let report = apply::execute(&workspace, &mut store, &input, "author", now()).unwrap();
    assert_eq!(report.projection.as_ref().unwrap().claim_count, 2);
    assert_eq!(report.projection.unwrap().relation_count, 2);
    let receipt = store
        .proposal_application(&input.proposal_id)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.context.sources.len(), 1);
    assert_eq!(
        receipt.context.sources[0].revision,
        before.sources[0].manifest.revisions[0]
    );
    let after = CanonicalSnapshot::scan(&workspace).unwrap();
    assert!(
        after
            .claims
            .iter()
            .any(|entry| entry.claim.assertion == claim)
    );
    assert!(
        after
            .relations
            .iter()
            .any(|entry| entry.relation.assertion == relation)
    );
}

#[test]
fn accepted_noop_has_a_durable_receipt_without_a_file_transaction_or_generation_bump() {
    let (_temp, workspace, mut store, _approved, _) = fixture();
    let before = CanonicalSnapshot::scan(&workspace).unwrap();
    let prepared = prepare(
        &workspace,
        &ProposalInput {
            kind: ProposalKind::Manual,
            base_generation: 1,
            schema_hash: before.schema_hash,
            source_revision_id: None,
            compiler_run_id: None,
            summary: "Retain the existing alias.".into(),
            items: vec![
                ProposalItem::new(
                    PatchOp::AddAlias,
                    before.nodes[0].metadata.id.to_string(),
                    json!({"alias":before.nodes[0].metadata.aliases[0]}),
                )
                .unwrap(),
            ],
        },
        "author",
        now(),
    )
    .unwrap();
    assert!(prepared.documents().is_empty());
    let record = ProposalRecord {
        proposal: prepared.proposal,
        base_snapshot_sha256: prepared.base_snapshot_sha256,
    };
    store.proposal_create(&record).unwrap();
    let mut input = ApplyInput {
        proposal_id: record.proposal.id.clone(),
        expected_revision: 1,
        dry_run: false,
        yes: true,
    };
    assert_eq!(
        apply::execute(&workspace, &mut store, &input, "author", now())
            .unwrap_err()
            .code,
        "PROPOSAL_REVIEW_REQUIRED"
    );
    let approved = ProposalRecord {
        proposal: record
            .proposal
            .review(
                &ReviewInput {
                    expected_revision: 1,
                    accept_all: true,
                    decisions: vec![],
                },
                &ReviewPolicy::default(),
                "reviewer",
                now(),
            )
            .unwrap(),
        ..record
    };
    store.proposal_save(1, &approved).unwrap();
    input.expected_revision = 2;
    let report = apply::execute(&workspace, &mut store, &input, "author", now()).unwrap();
    assert_eq!(report.projection.as_ref().unwrap().generation, 1);
    assert!(!report.projection.unwrap().changed);
    assert!(report.transaction_id.is_none());
    assert!(report.changed_paths.is_empty());
    assert!(!sync::recovery_status(&workspace).unwrap().recovery_required);
    assert_eq!(
        store
            .proposal_get(&input.proposal_id, None)
            .unwrap()
            .proposal
            .state,
        ProposalState::Applied
    );
    assert!(
        store
            .proposal_application(&input.proposal_id)
            .unwrap()
            .is_some()
    );
}

#[test]
fn applied_receipts_survive_atomic_rebuild_and_replay_after_later_canonical_edits() {
    use knowmesh_core::application::rebuild::{self, RebuildInput};
    let (_temp, workspace, mut store, _approved, input) = fixture();
    let applied = apply::execute(&workspace, &mut store, &input, "author", now()).unwrap();
    let expected = serde_json::to_value(&applied).unwrap();
    drop(store);
    let backend = knowmesh_sqlite::SqliteRebuilder::new(&workspace).unwrap();
    let rebuilt = rebuild::execute(
        &workspace,
        &backend,
        &RebuildInput {
            yes: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(rebuilt.runtime_table_counts["proposal_applications"], 1);
    let mut store = SqliteStore::open(&workspace.index_path().unwrap()).unwrap();
    let path = workspace.root.join("knowledge/nodes/model-a.md");
    let mut doc = NodeDocument::parse(&fs::read_to_string(&path).unwrap()).unwrap();
    doc.metadata.aliases.push("Later canonical change".into());
    fs::write(path, doc.render().unwrap()).unwrap();
    sync::synchronize(&workspace, &mut store).unwrap();
    assert_eq!(store.generation().unwrap(), 3);
    let repeated = apply::execute(&workspace, &mut store, &input, "retry", now()).unwrap();
    assert_eq!(serde_json::to_value(repeated).unwrap(), expected);
    assert_eq!(store.generation().unwrap(), 3);
    let backup = SqliteStore::open_read_only(&rebuilt.backup_paths[0]).unwrap();
    assert_eq!(
        serde_json::to_value(
            backup
                .proposal_application(&input.proposal_id)
                .unwrap()
                .unwrap()
                .report
        )
        .unwrap(),
        expected
    );
}
