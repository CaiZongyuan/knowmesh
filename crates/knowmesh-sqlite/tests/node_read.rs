#[path = "../../../tests/support/mod.rs"]
mod support;

use knowmesh_core::{
    application::node_read::{self, NodeGetInput, NodeListInput},
    canonical::{node::NodeDocument, workspace::Workspace},
    error::ErrorType,
};
use knowmesh_sqlite::SqliteStore;

fn store(workspace: &Workspace) -> SqliteStore {
    let store = SqliteStore::open(&workspace.index_path().unwrap()).unwrap();
    store
        .bind_workspace(&workspace.config.workspace.id, "")
        .unwrap();
    store
}

fn get(node: &str, no_sync: bool) -> NodeGetInput {
    NodeGetInput {
        node: node.into(),
        no_sync,
    }
}

#[test]
fn node_pages_are_bounded_filtered_and_bound_to_workspace_query_and_generation() {
    let (_temp, workspace) = support::fixture();
    let mut store = store(&workspace);
    let mut input = NodeListInput {
        limit: 1,
        ..Default::default()
    };
    let first = node_read::list(&workspace, &mut store, &input).unwrap();
    assert_eq!(first.total, 2);
    assert!(first.index_complete);
    assert_eq!(first.items.len(), 1);
    let listed_type = first.items[0].node_type.clone();
    let cursor = first.next_cursor.clone().unwrap();
    let mut ids = vec![first.items[0].id.clone()];
    input.cursor = Some(cursor.clone());
    loop {
        let page = node_read::list(&workspace, &mut store, &input).unwrap();
        ids.extend(page.items.iter().map(|item| item.id.clone()));
        input.cursor = page.next_cursor;
        if input.cursor.is_none() {
            break;
        }
    }
    assert_eq!(ids.len(), 2);
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
    input.cursor = Some(cursor.clone());
    input.node_type = Some(if listed_type == "Model" {
        "Dataset".into()
    } else {
        "Model".into()
    });
    assert_eq!(
        node_read::list(&workspace, &mut store, &input)
            .unwrap_err()
            .code,
        "CURSOR_QUERY_MISMATCH"
    );
    input.cursor = None;
    input.node_type = Some("Model".into());
    assert_eq!(
        node_read::list(&workspace, &mut store, &input)
            .unwrap()
            .total,
        1
    );
    input.node_type = Some("model".into());
    let invalid_type = node_read::list(&workspace, &mut store, &input).unwrap_err();
    assert_eq!(invalid_type.error_type, ErrorType::NotFound);
    assert_eq!(invalid_type.code, "SCHEMA_ENTITY_NOT_FOUND");
    assert_eq!(invalid_type.param.as_deref(), Some("node_type"));
    input.node_type = Some("Benchmark".into());
    assert_eq!(
        node_read::list(&workspace, &mut store, &input)
            .unwrap()
            .total,
        0
    );
    input.node_type = None;
    input.tag = Some("fixture".into());
    assert_eq!(
        node_read::list(&workspace, &mut store, &input)
            .unwrap()
            .total,
        2
    );
    input.tag = Some("' OR 1=1 --".into());
    assert_eq!(
        node_read::list(&workspace, &mut store, &input)
            .unwrap()
            .total,
        0
    );
    input.tag = None;
    input.status = Some("active".into());
    assert_eq!(
        node_read::list(&workspace, &mut store, &input)
            .unwrap()
            .total,
        2
    );
    input.status = Some("retracted".into());
    assert_eq!(
        node_read::list(&workspace, &mut store, &input)
            .unwrap()
            .total,
        0
    );
    input.status = Some("obsolete".into());
    assert_eq!(
        node_read::list(&workspace, &mut store, &input)
            .unwrap_err()
            .code,
        "INVALID_NODE_STATUS"
    );
    let (_other_temp, other_workspace) = support::fixture();
    let mut other_store = self::store(&other_workspace);
    input = NodeListInput {
        cursor: Some(cursor.clone()),
        ..Default::default()
    };
    assert_eq!(
        node_read::list(&other_workspace, &mut other_store, &input)
            .unwrap_err()
            .code,
        "CURSOR_QUERY_MISMATCH"
    );
    let model_path = workspace.root.join("knowledge/nodes/model-a.md");
    let mut document = knowmesh_core::canonical::node::NodeDocument::parse(
        &std::fs::read_to_string(&model_path).unwrap(),
    )
    .unwrap();
    document.metadata.name = "Renamed Model".into();
    std::fs::write(&model_path, document.render().unwrap()).unwrap();
    assert_eq!(
        node_read::list(&workspace, &mut store, &input)
            .unwrap_err()
            .code,
        "CURSOR_STALE"
    );
    input.cursor = None;
    assert_eq!(
        node_read::list(&workspace, &mut store, &input)
            .unwrap()
            .total,
        2
    );
    for limit in [0, 101] {
        input.limit = limit;
        assert_eq!(
            node_read::list(&workspace, &mut store, &input)
                .unwrap_err()
                .code,
            "INVALID_PAGE_LIMIT"
        );
    }
    input.limit = 20;
    for cursor in ["not a cursor".to_owned(), "a".repeat(4097)] {
        input.cursor = Some(cursor);
        assert_eq!(
            node_read::list(&workspace, &mut store, &input)
                .unwrap_err()
                .code,
            "INVALID_CURSOR"
        );
    }
}

#[test]
fn node_get_reads_ids_exactly_resolves_names_and_aliases_and_reports_ambiguity() {
    let (_temp, workspace) = support::fixture();
    let mut store = store(&workspace);
    let listing = node_read::list(&workspace, &mut store, &NodeListInput::default()).unwrap();
    let model = listing
        .items
        .iter()
        .find(|item| item.node_type == "Model")
        .unwrap();
    let dataset = listing
        .items
        .iter()
        .find(|item| item.node_type == "Dataset")
        .unwrap();
    let by_id = node_read::get(&workspace, &mut store, &get(model.id.as_str(), false)).unwrap();
    assert_eq!(by_id.node.node.metadata.id, model.id);
    assert_eq!(by_id.node.node.metadata.name, "Model A");
    assert_eq!(by_id.node.node.metadata.node_type, "Model");
    assert_eq!(by_id.node.node.summary, "A fictional model.");
    assert!(by_id.index_complete);
    assert_eq!(by_id.node.relations_total, 1);
    assert_eq!(by_id.node.relations.len(), 1);
    assert_eq!(by_id.node.relations[0].predicate, "evaluated_on");
    assert_eq!(by_id.node.relations[0].target_node_id, dataset.id);
    let by_name = node_read::get(&workspace, &mut store, &get("  MODEL   a ", false)).unwrap();
    assert_eq!(by_name.node.node.metadata.id, model.id);
    let ambiguous =
        node_read::get(&workspace, &mut store, &get("shared alias", false)).unwrap_err();
    assert_eq!(ambiguous.code, "AMBIGUOUS_NODE_NAME");
    assert_eq!(ambiguous.error_type, ErrorType::Validation);
    let candidates = ambiguous.details.as_ref().unwrap()["candidates"]
        .as_array()
        .unwrap();
    assert_eq!(candidates.len(), 2);
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate["id"] == model.id.as_str())
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate["id"] == dataset.id.as_str())
    );
    let model_path = workspace.root.join("knowledge/nodes/model-a.md");
    let mut document = NodeDocument::parse(&std::fs::read_to_string(&model_path).unwrap()).unwrap();
    document.metadata.aliases.push("GLM Model".into());
    std::fs::write(&model_path, document.render().unwrap()).unwrap();
    let by_alias = node_read::get(&workspace, &mut store, &get("glm model", false)).unwrap();
    assert_eq!(by_alias.node.node.metadata.id, model.id);
    let missing = node_read::get(&workspace, &mut store, &get("Absent Node", false)).unwrap_err();
    assert_eq!(missing.code, "NODE_NOT_FOUND");
    assert_eq!(missing.error_type, ErrorType::NotFound);
    let foreign_id = node_read::get(
        &workspace,
        &mut store,
        &get("src_01ABCDEFGHJKMNPQRSTVWX2345", false),
    )
    .unwrap_err();
    assert_eq!(foreign_id.code, "INVALID_NODE_ID");
    let malformed_node_id =
        node_read::get(&workspace, &mut store, &get("kn_not_a_node", false)).unwrap_err();
    assert_eq!(malformed_node_id.code, "INVALID_NODE_ID");
    let unknown_node_id = knowmesh_core::domain::NodeId::new();
    assert_ne!(&unknown_node_id.to_string(), model.id.as_str());
    let unknown = node_read::get(
        &workspace,
        &mut store,
        &get(unknown_node_id.as_str(), false),
    )
    .unwrap_err();
    assert_eq!(unknown.code, "NODE_NOT_FOUND");
}

#[test]
fn node_get_reports_no_sync_staleness_and_externally_renamed_nodes_accurately() {
    let (_temp, workspace) = support::fixture();
    let mut store = store(&workspace);
    let before = node_read::get(&workspace, &mut store, &get("Dataset B", false)).unwrap();
    assert!(before.index_complete);
    let dataset_path = workspace.root.join("knowledge/nodes/dataset-b.md");
    let mut document =
        NodeDocument::parse(&std::fs::read_to_string(&dataset_path).unwrap()).unwrap();
    let old_id = document.metadata.id.clone();
    document.metadata.name = "Dataset B Two".into();
    std::fs::write(&dataset_path, document.render().unwrap()).unwrap();
    let stale = node_read::get(&workspace, &mut store, &get("Dataset B", true)).unwrap();
    assert_eq!(stale.node.node.metadata.id, old_id);
    assert!(!stale.index_complete);
    let renamed = node_read::get(&workspace, &mut store, &get("dataset b two", false)).unwrap();
    assert_eq!(renamed.node.node.metadata.id, old_id);
    assert!(renamed.index_complete);
    let old_name = node_read::get(&workspace, &mut store, &get("Dataset B", false)).unwrap_err();
    assert_eq!(old_name.code, "NODE_NOT_FOUND");
}
