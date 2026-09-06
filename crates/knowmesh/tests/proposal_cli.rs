#[path = "../../../tests/support/mod.rs"]
mod support;

use std::path::Path;

use assert_cmd::cargo::cargo_bin_cmd;
use knowmesh_core::{canonical::snapshot::CanonicalSnapshot, domain::{proposal::{PatchOp, ProposalItem, ProposalKind}, Timestamp}};
use serde_json::{Value, json};

fn call(root: &Path, args: &[&str], input: Option<Value>) -> std::process::Output {
    let mut command = cargo_bin_cmd!("knowmesh");
    command.arg("--workspace").arg(root).args(args);
    if let Some(input) = input { command.write_stdin(serde_json::to_vec(&input).unwrap()); }
    command.output().unwrap()
}

fn success(output: std::process::Output) -> Value {
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn proposal_cli_creates_reviews_and_applies_json_from_stdin() {
    let (_temp, workspace) = support::fixture();
    success(call(&workspace.root, &["sync"], None));
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let input = json!({"proposal":{
        "kind":ProposalKind::Manual,"base_generation":1,"schema_hash":snapshot.schema_hash,
        "source_revision_id":null,"compiler_run_id":null,"summary":"CLI proposal fixture.",
        "items":[ProposalItem::new(PatchOp::AddAlias,snapshot.nodes[0].metadata.id.to_string(),json!({"alias":"CLI alias"})).unwrap()]
    }});
    let preview = success(call(&workspace.root, &["proposal","create","--input","-","--dry-run"], Some(input.clone())));
    assert_eq!(preview["data"]["dry_run"], true);
    let created = success(call(&workspace.root, &["proposal","create","--input","-"], Some(input)));
    let id = created["data"]["record"]["proposal"]["id"].as_str().unwrap();
    assert_eq!(created["meta"]["command"], "proposal.create");
    let reviewed = success(call(&workspace.root, &["proposal","review","--input","-"], Some(json!({"proposal_id":id,"review":{"expected_revision":1,"accept_all":true,"decisions":[]}}))));
    assert_eq!(reviewed["data"]["record"]["proposal"]["state"], "approved");
    let historical = success(call(&workspace.root, &["proposal","get",id,"--revision","1"], None));
    assert_eq!(historical["data"]["proposal"]["state"], "draft");
    let unconfirmed = call(&workspace.root, &["proposal","apply",id,"--expected-revision","2"], None);
    assert!(unconfirmed.stdout.is_empty());
    assert_eq!(serde_json::from_slice::<Value>(&unconfirmed.stderr).unwrap()["error"]["code"], "CONFIRMATION_REQUIRED");
    let applied = success(call(&workspace.root, &["proposal","apply",id,"--expected-revision","2","--yes"], None));
    assert_eq!(applied["data"]["projection"]["generation"], 2);
    assert_eq!(success(call(&workspace.root, &["proposal","get",id], None))["data"]["proposal"]["state"], "applied");
}

#[test]
fn proposal_request_files_match_descriptors_and_all_commands_are_discoverable() {
    let (_temp, workspace) = support::fixture();
    success(call(&workspace.root, &["sync"], None));
    for (name, effect) in [
        ("proposal.create","runtime-write"),("proposal.get","read"),("proposal.review","runtime-write"),
        ("proposal.edit","runtime-write"),("proposal.revalidate","runtime-write"),("proposal.reject","runtime-write"),("proposal.apply","canonical-write")
    ] {
        let descriptor = success(call(&workspace.root, &["schema","command",name], None));
        assert_eq!(descriptor["data"]["effect"], effect);
    }
    let patch = success(call(&workspace.root, &["schema","patch","create_node"], None));
    assert!(patch["data"]["properties"]["metadata"].is_object());
    let snapshot = CanonicalSnapshot::scan(&workspace).unwrap();
    let proposal = json!({"proposal":{"kind":"manual","base_generation":1,"schema_hash":snapshot.schema_hash,
        "source_revision_id":null,"compiler_run_id":null,"summary":"File input fixture.","items":[ProposalItem::new(PatchOp::AddAlias,snapshot.nodes[0].metadata.id.to_string(),json!({"alias":"File alias"})).unwrap()]}});
    let path = workspace.root.join("proposal-input.json");
    std::fs::write(&path, serde_json::to_vec(&proposal).unwrap()).unwrap();
    let created = success(call(&workspace.root, &["proposal","create","--input",path.to_str().unwrap()], None));
    assert_eq!(created["data"]["record"]["proposal"]["created_by"], "human_cli");
    let _: Timestamp = created["data"]["record"]["proposal"]["created_at"].as_str().unwrap().parse().unwrap();
}

#[test]
fn malformed_proposal_json_has_typed_stderr_and_does_not_create_an_index() {
    let (_temp, workspace) = support::fixture();
    let output = call(&workspace.root, &["proposal","create","--input","-"], Some(json!({"unknown":"field"})));
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"], "INVALID_INPUT_JSON");
    assert!(!workspace.index_path().unwrap().exists());
}
