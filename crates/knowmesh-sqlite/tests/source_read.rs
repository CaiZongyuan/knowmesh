#[path = "../../../tests/support/mod.rs"]
mod support;

use std::fs;

use knowmesh_core::{
    application::{
        source,
        source_read::{self, ContentInput, GetInput, ListInput},
    },
    canonical::{
        source::{ImportInput, SourceLibrary},
        workspace::Workspace,
    },
    domain::{SourceId, SourceRevisionId, StorageMode},
};
use knowmesh_sqlite::SqliteStore;

fn store(workspace: &Workspace) -> SqliteStore {
    let store = SqliteStore::open(&workspace.index_path().unwrap()).unwrap();
    store
        .bind_workspace(&workspace.config.workspace.id, "")
        .unwrap();
    store
}

fn add(workspace: &Workspace, store: &mut SqliteStore, name: &str) -> SourceId {
    let path = workspace.root.join(format!("{name}.txt"));
    fs::write(&path, name).unwrap();
    source::add(
        workspace,
        store,
        &ImportInput {
            path,
            source_id: None,
            storage: Some(StorageMode::Managed),
            title: Some(name.into()),
            kind: "note".into(),
            tags: vec!["new".into()],
            dry_run: false,
        },
        None,
    )
    .unwrap()
    .import
    .source
    .id
}

#[test]
fn source_pages_are_bounded_filtered_and_bound_to_workspace_query_and_generation() {
    let (_temp, workspace) = support::fixture();
    let mut store = store(&workspace);
    let second = add(&workspace, &mut store, "Second");
    add(&workspace, &mut store, "Third");
    let mut input = ListInput {
        limit: 1,
        ..Default::default()
    };
    let first = source_read::list(&workspace, &mut store, &input).unwrap();
    assert_eq!(first.total, 3);
    assert!(first.index_complete);
    assert_eq!(first.items.len(), 1);
    let cursor = first.next_cursor.clone().unwrap();
    let mut ids = vec![first.items[0].id.clone()];
    input.cursor = Some(cursor.clone());
    loop {
        let page = source_read::list(&workspace, &mut store, &input).unwrap();
        ids.extend(page.items.iter().map(|item| item.id.clone()));
        input.cursor = page.next_cursor;
        if input.cursor.is_none() {
            break;
        }
    }
    assert_eq!(ids.len(), 3);
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
    input.cursor = Some(cursor.clone());
    input.kind = Some("note".into());
    assert_eq!(
        source_read::list(&workspace, &mut store, &input)
            .unwrap_err()
            .code,
        "CURSOR_QUERY_MISMATCH"
    );
    input.cursor = None;
    input.tag = Some("new".into());
    assert_eq!(
        source_read::list(&workspace, &mut store, &input)
            .unwrap()
            .total,
        2
    );
    input.tag = Some("' OR 1=1 --".into());
    assert_eq!(
        source_read::list(&workspace, &mut store, &input)
            .unwrap()
            .total,
        0
    );
    let (_other_temp, other_workspace) = support::fixture();
    let mut other_store = self::store(&other_workspace);
    input = ListInput {
        cursor: Some(cursor.clone()),
        ..Default::default()
    };
    assert_eq!(
        source_read::list(&other_workspace, &mut other_store, &input)
            .unwrap_err()
            .code,
        "CURSOR_QUERY_MISMATCH"
    );
    source::remove(
        &workspace,
        &mut store,
        &source::RemoveInput {
            source_id: second,
            dry_run: false,
            yes: true,
        },
    )
    .unwrap();
    assert_eq!(
        source_read::list(&workspace, &mut store, &input)
            .unwrap_err()
            .code,
        "CURSOR_STALE"
    );
    input.cursor = None;
    assert_eq!(
        source_read::list(&workspace, &mut store, &input)
            .unwrap()
            .total,
        2
    );
    input.include_removed = true;
    assert_eq!(
        source_read::list(&workspace, &mut store, &input)
            .unwrap()
            .total,
        3
    );
    for limit in [0, 101] {
        input.limit = limit;
        assert_eq!(
            source_read::list(&workspace, &mut store, &input)
                .unwrap_err()
                .code,
            "INVALID_PAGE_LIMIT"
        );
    }
    input.limit = 20;
    for cursor in ["not a cursor".into(), "a".repeat(4097)] {
        input.cursor = Some(cursor);
        assert_eq!(
            source_read::list(&workspace, &mut store, &input)
                .unwrap_err()
                .code,
            "INVALID_CURSOR"
        );
    }
}

#[test]
fn source_get_fast_syncs_metadata_and_reports_no_sync_without_inventing_current_state() {
    let (_temp, workspace) = support::fixture();
    let mut store = store(&workspace);
    let mut file = SourceLibrary::new(&workspace).list(true).unwrap().remove(0);
    let mut input = GetInput {
        source_id: file.manifest.id.clone(),
        no_sync: false,
    };
    let before = source_read::get(&workspace, &mut store, &input).unwrap();
    file.manifest.title = "Changed external title".into();
    fs::write(workspace.root.join(&file.path), file.render().unwrap()).unwrap();
    input.no_sync = true;
    let stale = source_read::get(&workspace, &mut store, &input).unwrap();
    assert_eq!(stale.source.title, before.source.title);
    assert_eq!(stale.generation, before.generation);
    assert!(!stale.index_complete);
    input.no_sync = false;
    let current = source_read::get(&workspace, &mut store, &input).unwrap();
    assert_eq!(current.source.title, "Changed external title");
    assert!(current.generation > before.generation);
    assert!(current.index_complete);
    input.source_id = SourceId::new();
    assert_eq!(
        source_read::get(&workspace, &mut store, &input)
            .unwrap_err()
            .code,
        "SOURCE_NOT_FOUND"
    );
    let missing = ContentInput {
        id: SourceRevisionId::new().as_str().parse().unwrap(),
        no_sync: false,
    };
    assert_eq!(
        source_read::content(&workspace, &mut store, &missing)
            .unwrap_err()
            .code,
        "SOURCE_REVISION_NOT_FOUND"
    );
}

#[test]
fn content_preserves_historical_bytes_after_removal_and_checks_the_indexed_revision_hash() {
    let (_temp, workspace) = support::fixture();
    let mut store = store(&workspace);
    let old = SourceLibrary::new(&workspace).list(true).unwrap().remove(0);
    let old_revision = old.manifest.revisions[0].clone();
    let mut input = ContentInput {
        id: old_revision.id.as_str().parse().unwrap(),
        no_sync: false,
    };
    let old_content = source_read::content(&workspace, &mut store, &input).unwrap();
    assert!(old_content.bytes.starts_with(b"# Fixture"));
    let next_path = workspace.root.join("updated.txt");
    fs::write(&next_path, "Updated evidence\n").unwrap();
    source::add(
        &workspace,
        &mut store,
        &ImportInput {
            path: next_path,
            source_id: Some(old.manifest.id.clone()),
            storage: None,
            title: None,
            kind: "paper".into(),
            tags: vec![],
            dry_run: false,
        },
        None,
    )
    .unwrap();
    assert_eq!(
        source_read::content(&workspace, &mut store, &input)
            .unwrap()
            .bytes,
        old_content.bytes
    );
    input.id = old.manifest.id.as_str().parse().unwrap();
    assert_eq!(
        source_read::content(&workspace, &mut store, &input)
            .unwrap()
            .bytes,
        b"Updated evidence\n"
    );
    source::remove(
        &workspace,
        &mut store,
        &source::RemoveInput {
            source_id: old.manifest.id.clone(),
            dry_run: false,
            yes: true,
        },
    )
    .unwrap();
    input.id = old_revision.id.as_str().parse().unwrap();
    assert_eq!(
        source_read::content(&workspace, &mut store, &input)
            .unwrap()
            .bytes,
        old_content.bytes
    );
    let removed = source_read::get(
        &workspace,
        &mut store,
        &GetInput {
            source_id: old.manifest.id.clone(),
            no_sync: false,
        },
    )
    .unwrap();
    assert!(removed.source.removed_at.is_some());
    fs::write(
        workspace
            .root
            .join(old.path.parent().unwrap())
            .join(&old_revision.path),
        "corrupt",
    )
    .unwrap();
    input.no_sync = true;
    assert_eq!(
        source_read::content(&workspace, &mut store, &input)
            .unwrap_err()
            .code,
        "SOURCE_REVISION_CHANGED"
    );
}
