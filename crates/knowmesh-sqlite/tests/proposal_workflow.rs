#[path = "../../../tests/support/mod.rs"]
mod support;

use std::fs;

use knowmesh_core::{
    application::proposal::{apply::{self, ApplyInput}, workflow::{self, CreateInput, EditInput, GetInput, RevalidateInput, RejectInput, ReviewRequest}},
    canonical::{node::NodeDocument, schema::Schema, snapshot::CanonicalSnapshot, workspace::Workspace},
    domain::{Timestamp, proposal::{Decision, PatchOp, ProposalInput, ProposalItem, ProposalRevision, ProposalState, ReviewInput}},
    ports::{ProjectionStore, ProposalStore},
};
use knowmesh_sqlite::SqliteStore;
use serde_json::json;

fn now() -> Timestamp { "2026-09-07T00:00:00Z".parse().unwrap() }

fn fixture() -> (tempfile::TempDir, Workspace, SqliteStore, CreateInput) {
    let (temp, workspace) = support::fixture();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let mut store = SqliteStore::open(&workspace.index_path().unwrap()).unwrap();
    store.bind_workspace(&workspace.config.workspace.id, &snapshot.schema_hash).unwrap();
    let generation = store.reconcile(&snapshot).unwrap().generation;
    let input = CreateInput { dry_run:false, proposal:ProposalInput {
        kind:knowmesh_core::domain::proposal::ProposalKind::Manual,base_generation:generation,schema_hash:snapshot.schema_hash,
        source_revision_id:None,compiler_run_id:None,summary:"Review workflow changes.".into(),
        items:snapshot.nodes.iter().map(|node| ProposalItem::new(PatchOp::AddAlias,node.metadata.id.to_string(),json!({"alias":format!("Workflow {}",node.metadata.name)})).unwrap()).collect(),
    }};
    (temp, workspace, store, input)
}

#[test]
fn create_review_and_apply_share_validated_records_and_keep_history_readable() {
    let (_temp, workspace, mut store, input) = fixture();
    let created = workflow::create(&workspace, &mut store, &input, "author", now()).unwrap();
    let id = created.record.proposal.id.clone();
    assert!(created.record.proposal.items.iter().all(|item| item.before_sha256.is_some()));
    let reviewed = workflow::review(&workspace, &mut store, &ReviewRequest {
        proposal_id:id.clone(),dry_run:false,review:ReviewInput { expected_revision:1,accept_all:true,decisions:vec![] },
    }, "reviewer", now()).unwrap();
    assert_eq!(reviewed.record.proposal.state, ProposalState::Approved);
    let historical = workflow::get(&workspace, &store, &GetInput { proposal_id:id.clone(),revision:Some(1) }).unwrap();
    assert_eq!(historical, created.record);
    let applied = apply::execute(&workspace, &mut store, &ApplyInput { proposal_id:id.clone(),expected_revision:2,dry_run:false,yes:true }, "author", now()).unwrap();
    assert_eq!(applied.changed_paths.len(), 2);
    assert_eq!(workflow::get(&workspace, &store, &GetInput { proposal_id:id,revision:None }).unwrap().proposal.state, ProposalState::Applied);
}

#[test]
fn every_runtime_mutation_has_a_read_only_preview() {
    let (_temp, workspace, mut store, mut input) = fixture();
    input.dry_run = true;
    let preview = workflow::create(&workspace, &mut store, &input, "author", now()).unwrap();
    assert!(preview.dry_run);
    assert_eq!(store.proposal_get(&preview.record.proposal.id, None).unwrap_err().code, "PROPOSAL_NOT_FOUND");
    input.dry_run = false;
    let created = workflow::create(&workspace, &mut store, &input, "author", now()).unwrap().record;
    let reviewed = workflow::review(&workspace, &mut store, &ReviewRequest { proposal_id:created.proposal.id.clone(),dry_run:true,
        review:ReviewInput { expected_revision:1,accept_all:true,decisions:vec![] } }, "reviewer", now()).unwrap();
    assert_eq!(reviewed.record.proposal.state, ProposalState::Approved);
    let rejected = workflow::reject(&workspace, &mut store, &RejectInput { proposal_id:created.proposal.id.clone(),expected_revision:1,reason:"Preview rejection".into(),dry_run:true }, "author", now()).unwrap();
    assert_eq!(rejected.record.proposal.state, ProposalState::Rejected);
    let mut items = created.proposal.items.clone();
    items[0].payload = json!({"alias":"Edited preview"});
    let edited = workflow::edit(&workspace, &mut store, &EditInput { proposal_id:created.proposal.id.clone(),dry_run:true,
        revision:ProposalRevision { expected_revision:1,base_generation:1,schema_hash:created.proposal.schema_hash.clone(),summary:created.proposal.summary.clone(),items } }, "editor", now()).unwrap();
    assert_eq!(edited.record.proposal.revision, 2);
    assert_eq!(store.proposal_get(&created.proposal.id, None).unwrap(), created);
    assert_eq!(CanonicalSnapshot::scan(&workspace).unwrap().content_sha256, created.base_snapshot_sha256);
}

#[test]
fn editing_revalidates_payloads_and_resets_only_changed_item_reviews() {
    let (_temp, workspace, mut store, input) = fixture();
    let created = workflow::create(&workspace, &mut store, &input, "author", now()).unwrap().record;
    let approved = workflow::review(&workspace, &mut store, &ReviewRequest { proposal_id:created.proposal.id.clone(),dry_run:false,
        review:ReviewInput { expected_revision:1,accept_all:true,decisions:vec![] } }, "reviewer", now()).unwrap().record;
    let mut items = approved.proposal.items.clone();
    items[0].payload = json!({"alias":""});
    let mut edit = EditInput { proposal_id:approved.proposal.id.clone(),dry_run:false,
        revision:ProposalRevision { expected_revision:2,base_generation:1,schema_hash:approved.proposal.schema_hash.clone(),summary:approved.proposal.summary.clone(),items } };
    let invalid = workflow::edit(&workspace, &mut store, &edit, "editor", now()).unwrap().record;
    assert_eq!(invalid.proposal.items[0].decision, Decision::Pending);
    assert_eq!(invalid.proposal.items[1].decision, Decision::Accepted);
    assert!(invalid.proposal.items[0].issues.iter().any(|issue| issue.blocking));
    edit.revision.expected_revision = 3;
    edit.revision.items = invalid.proposal.items.clone();
    edit.revision.items[0].payload = json!({"alias":"Repaired payload"});
    let repaired = workflow::edit(&workspace, &mut store, &edit, "editor", now()).unwrap().record;
    assert!(repaired.proposal.items[0].issues.is_empty());
    assert_eq!(repaired.proposal.items[1].decision, Decision::Accepted);
    assert_eq!(store.proposal_get(&created.proposal.id, Some(2)).unwrap(), approved);
}

#[test]
fn stale_review_requires_explicit_revalidation_and_all_previous_reviews_reset() {
    let (_temp, workspace, mut store, input) = fixture();
    let created = workflow::create(&workspace, &mut store, &input, "author", now()).unwrap().record;
    workflow::review(&workspace, &mut store, &ReviewRequest { proposal_id:created.proposal.id.clone(),dry_run:false,
        review:ReviewInput { expected_revision:1,accept_all:true,decisions:vec![] } }, "reviewer", now()).unwrap();
    let path = workspace.root.join("knowledge/nodes/model-a.md");
    let mut doc = NodeDocument::parse(&fs::read_to_string(&path).unwrap()).unwrap();
    doc.metadata.aliases.push("External update".into());
    fs::write(&path, doc.render().unwrap()).unwrap();
    assert_eq!(workflow::review(&workspace, &mut store, &ReviewRequest { proposal_id:created.proposal.id.clone(),dry_run:false,
        review:ReviewInput { expected_revision:2,accept_all:true,decisions:vec![] } }, "reviewer", now()).unwrap_err().code, "STALE_PROPOSAL");
    let stale = store.proposal_get(&created.proposal.id, None).unwrap();
    assert_eq!(stale.proposal.state, ProposalState::Stale);
    let preview = workflow::revalidate(&workspace, &mut store, &RevalidateInput { proposal_id:created.proposal.id.clone(),expected_revision:stale.proposal.revision,dry_run:true }, "editor", now()).unwrap();
    assert!(preview.record.proposal.items.iter().all(|item| item.decision==Decision::Pending));
    assert_eq!(store.generation().unwrap(), 1);
    assert_eq!(store.proposal_get(&created.proposal.id, None).unwrap(), stale);
    let revised = workflow::revalidate(&workspace, &mut store, &RevalidateInput { proposal_id:created.proposal.id.clone(),expected_revision:stale.proposal.revision,dry_run:false }, "editor", now()).unwrap().record;
    assert_eq!(revised.proposal.base_generation, 2);
    assert!(revised.proposal.items.iter().all(|item| item.decision==Decision::Pending));
    assert_eq!(revised.base_snapshot_sha256, CanonicalSnapshot::scan(&workspace).unwrap().content_sha256);
}

#[test]
fn workspace_policy_is_enforced_and_stale_proposals_can_be_explicitly_rejected() {
    let (_temp, workspace, mut store, mut input) = fixture();
    let path = workspace.root.join("schemas/research.yaml");
    let mut pack = knowmesh_core::canonical::schema::SchemaPack::parse(&fs::read(&path).unwrap()).unwrap();
    pack.policies.review_mode = knowmesh_core::canonical::schema::ReviewMode::Strict;
    fs::write(&path, serde_yaml::to_string(&pack).unwrap()).unwrap();
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    input.proposal.base_generation = store.reconcile(&snapshot).unwrap().generation;
    input.proposal.schema_hash = Schema::load(&workspace).unwrap().hash;
    let created = workflow::create(&workspace, &mut store, &input, "author", now()).unwrap().record;
    assert_eq!(workflow::review(&workspace, &mut store, &ReviewRequest { proposal_id:created.proposal.id.clone(),dry_run:false,
        review:ReviewInput { expected_revision:1,accept_all:true,decisions:vec![] } }, "reviewer", now()).unwrap_err().code, "STRICT_REVIEW_REQUIRED");
    fs::write(&path, "Invalid current schema").unwrap();
    let rejected = workflow::reject(&workspace, &mut store, &RejectInput { proposal_id:created.proposal.id,expected_revision:1,reason:"Reject obsolete draft".into(),dry_run:false }, "author", now()).unwrap();
    assert_eq!(rejected.record.proposal.state, ProposalState::Rejected);
}
