use std::{fs, path::PathBuf};

use super::{
    source::{ImportInput, ImportedContent, SourceLibrary, SourcePlan},
    transaction::WorkspaceWriter,
    workspace::{InitOptions, Workspace, initialize},
};
use crate::domain::{SourceId, StorageMode};

fn setup() -> (tempfile::TempDir, Workspace) {
    let temp = tempfile::tempdir().unwrap();
    initialize(temp.path(), &InitOptions::default()).unwrap();
    let workspace = Workspace::load(temp.path()).unwrap();
    (temp, workspace)
}

fn input(path: PathBuf, source_id: Option<SourceId>) -> ImportInput {
    ImportInput {
        path,
        source_id,
        storage: None,
        title: None,
        kind: "paper".into(),
        tags: vec![],
        dry_run: false,
    }
}

fn apply(workspace: &Workspace, plan: &mut SourcePlan) {
    if plan.changes.is_empty() {
        return;
    }
    let writer = WorkspaceWriter::acquire(&workspace.root).unwrap();
    let id = writer.prepare(std::mem::take(&mut plan.changes)).unwrap();
    writer.apply(&id).unwrap();
    writer.mark_indexed(&id).unwrap();
}

#[test]
fn managed_source_revisions_are_immutable_and_repeat_hash_does_not_move_head() {
    let (temp, workspace) = setup();
    let path = temp.path().join("input.md");
    fs::write(&path, "# Original\n\nFirst revision.\n").unwrap();
    let library = SourceLibrary::new(&workspace);
    let mut first = library.plan_add(&input(path.clone(), None), None).unwrap();
    apply(&workspace, &mut first);
    let first_revision = first.revision.id.clone();
    let first_snapshot = library.content(&first.source, &first_revision).unwrap();
    fs::write(&path, "# Updated\n\nSecond revision.\n").unwrap();
    let mut second = library
        .plan_add(&input(path.clone(), Some(first.source.id.clone())), None)
        .unwrap();
    assert_ne!(first_revision, second.revision.id);
    assert_eq!(second.source.revisions.len(), 2);
    apply(&workspace, &mut second);
    assert_eq!(
        library.content(&second.source, &first_revision).unwrap(),
        first_snapshot
    );
    fs::write(&path, &first_snapshot).unwrap();
    let repeated = library
        .plan_add(&input(path, Some(first.source.id.clone())), None)
        .unwrap();
    assert!(repeated.changes.is_empty());
    assert!(repeated.deduplicated);
    assert_eq!(repeated.revision.id, first_revision);
    assert_eq!(repeated.source.current_revision_id, second.revision.id);
}

#[test]
fn distinct_imports_do_not_merge_sources_by_filename_or_content() {
    let (temp, workspace) = setup();
    let path = temp.path().join("same-name.txt");
    fs::write(&path, "Identical source content.").unwrap();
    let library = SourceLibrary::new(&workspace);
    let mut first = library.plan_add(&input(path.clone(), None), None).unwrap();
    apply(&workspace, &mut first);
    let second = library.plan_add(&input(path, None), None).unwrap();
    assert_ne!(first.source.id, second.source.id);
}

#[test]
fn referenced_content_is_verified_on_every_read() {
    let (temp, workspace) = setup();
    let path = temp.path().join("large.txt");
    fs::write(&path, "Referenced content.").unwrap();
    let mut request = input(path.clone(), None);
    request.storage = Some(StorageMode::Referenced);
    let library = SourceLibrary::new(&workspace);
    let mut plan = library.plan_add(&request, None).unwrap();
    assert!(PathBuf::from(&plan.revision.path).is_absolute());
    assert_eq!(plan.changes.len(), 1);
    apply(&workspace, &mut plan);
    assert_eq!(
        library.content(&plan.source, &plan.revision.id).unwrap(),
        b"Referenced content."
    );
    fs::write(path, "Externally changed.").unwrap();
    assert_eq!(
        library
            .content(&plan.source, &plan.revision.id)
            .unwrap_err()
            .code,
        "SOURCE_REVISION_CHANGED"
    );
}

#[test]
fn soft_remove_preserves_snapshots_and_blocks_new_revisions() {
    let (temp, workspace) = setup();
    let path = temp.path().join("source.txt");
    fs::write(&path, "Historical evidence.").unwrap();
    let library = SourceLibrary::new(&workspace);
    let mut added = library.plan_add(&input(path.clone(), None), None).unwrap();
    apply(&workspace, &mut added);
    let mut removed = library.plan_remove(&added.source.id).unwrap();
    apply(&workspace, &mut removed);
    assert!(library.list(false).unwrap().is_empty());
    assert!(
        library
            .get(&added.source.id)
            .unwrap()
            .manifest
            .removed_at
            .is_some()
    );
    assert_eq!(
        library
            .content(&removed.source, &added.revision.id)
            .unwrap(),
        b"Historical evidence."
    );
    assert!(
        library
            .plan_remove(&added.source.id)
            .unwrap()
            .changes
            .is_empty()
    );
    assert_eq!(
        library
            .plan_add(&input(path, Some(added.source.id)), None)
            .unwrap_err()
            .code,
        "SOURCE_REMOVED"
    );
}

#[test]
fn mime_size_and_storage_policies_fail_before_writing() {
    let (temp, mut workspace) = setup();
    workspace.config.sources.max_file_mib = 1;
    let path = temp.path().join("oversized.txt");
    fs::write(&path, vec![b'x'; 1024 * 1024 + 1]).unwrap();
    let library = SourceLibrary::new(&workspace);
    assert_eq!(
        library.plan_add(&input(path, None), None).unwrap_err().code,
        "SOURCE_TOO_LARGE"
    );
    let path = temp.path().join("unsupported.docx");
    fs::write(&path, b"PK fixture").unwrap();
    assert_eq!(
        library.plan_add(&input(path, None), None).unwrap_err().code,
        "SOURCE_TYPE_UNSUPPORTED"
    );
    let path = temp.path().join("fake.pdf");
    fs::write(&path, b"not a PDF").unwrap();
    assert_eq!(
        library.plan_add(&input(path, None), None).unwrap_err().code,
        "SOURCE_MIME_MISMATCH"
    );
    assert!(library.list(true).unwrap().is_empty());
}

#[test]
fn fetched_single_resource_is_stored_as_a_snapshot() {
    let (_temp, workspace) = setup();
    let library = SourceLibrary::new(&workspace);
    let request = input(PathBuf::from("https://example.invalid/paper"), None);
    let mut plan = library
        .plan_add(
            &request,
            Some(ImportedContent {
                bytes: b"<html><body>Source text</body></html>".to_vec(),
                mime_type: "text/html".into(),
                final_url: "https://example.invalid/paper".into(),
            }),
        )
        .unwrap();
    assert_eq!(plan.source.storage, StorageMode::SnapshotUrl);
    assert_eq!(
        plan.revision.url.as_deref(),
        Some("https://example.invalid/paper")
    );
    apply(&workspace, &mut plan);
    assert!(
        String::from_utf8(library.content(&plan.source, &plan.revision.id).unwrap())
            .unwrap()
            .contains("Source text")
    );
}

#[test]
fn source_planning_is_read_only_and_manifests_survive_round_trips() {
    let (temp, workspace) = setup();
    let path = temp.path().join("paper.md");
    fs::write(&path, "# Paper\n\nEvidence.\n").unwrap();
    let mut request = input(path, None);
    request.dry_run = true;
    let library = SourceLibrary::new(&workspace);
    let mut plan = library.plan_add(&request, None).unwrap();
    assert!(library.list(true).unwrap().is_empty());
    apply(&workspace, &mut plan);
    let loaded = library.get(&plan.source.id).unwrap();
    assert_eq!(loaded.render().unwrap(), loaded.original);
    let future = loaded.original.replacen("version: 1", "version: 99", 1);
    assert_eq!(
        super::source::SourceFile::parse(loaded.path.clone(), future.as_bytes())
            .unwrap_err()
            .code,
        "UNSUPPORTED_SOURCE_VERSION"
    );
}
