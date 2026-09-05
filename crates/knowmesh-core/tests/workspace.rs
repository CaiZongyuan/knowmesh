use std::{collections::BTreeMap, fs};

use knowmesh_core::canonical::workspace::{InitOptions, Workspace, initialize, resolve_workspace};

#[test]
fn initialize_creates_portable_canonical_files_and_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("research");
    let options = InitOptions {
        name: "Virtual Cell".into(),
        template: "research".into(),
        dry_run: false,
    };
    let first = initialize(&root, &options).unwrap();
    let loaded = Workspace::load(&root).unwrap();
    assert_eq!(first.workspace_id, loaded.config.workspace.id);
    assert_eq!(loaded.config.workspace.name, "Virtual Cell");
    assert!(root.join("purpose.md").is_file());
    assert!(root.join("schemas/research.yaml").is_file());
    assert!(root.join("sources").is_dir());
    assert!(root.join("knowledge/syntheses").is_dir());
    assert!(
        fs::read_to_string(root.join(".gitignore"))
            .unwrap()
            .lines()
            .any(|line| line == ".knowmesh/")
    );
    let before = fs::read(root.join("knowmesh.yaml")).unwrap();
    let second = initialize(&root, &options).unwrap();
    assert_eq!(first.workspace_id, second.workspace_id);
    assert!(second.created_paths.is_empty());
    assert_eq!(fs::read(root.join("knowmesh.yaml")).unwrap(), before);
}

#[test]
fn dry_run_and_conflicting_initialization_do_not_overwrite_user_data() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("planned");
    let mut options = InitOptions {
        name: "Research".into(),
        template: "research".into(),
        dry_run: true,
    };
    let report = initialize(&root, &options).unwrap();
    assert!(!report.created_paths.is_empty());
    assert!(!root.exists());
    options.dry_run = false;
    fs::create_dir(&root).unwrap();
    fs::write(root.join("purpose.md"), "Human-authored research scope").unwrap();
    let error = initialize(&root, &options).unwrap_err();
    assert_eq!(error.code, "INITIALIZATION_CONFLICT");
    assert!(!root.join("knowmesh.yaml").exists());
    assert_eq!(
        fs::read_to_string(root.join("purpose.md")).unwrap(),
        "Human-authored research scope"
    );
}

#[test]
fn workspace_resolution_obeys_explicit_then_environment_then_ancestors() {
    let temp = tempfile::tempdir().unwrap();
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    let options = InitOptions::default();
    initialize(&a, &options).unwrap();
    initialize(&b, &options).unwrap();
    let nested = a.join("knowledge/nodes/model");
    fs::create_dir_all(&nested).unwrap();
    assert_eq!(
        resolve_workspace(None, None, &nested).unwrap(),
        fs::canonicalize(&a).unwrap()
    );
    assert_eq!(
        resolve_workspace(None, Some(b.as_path()), &nested).unwrap(),
        fs::canonicalize(&b).unwrap()
    );
    assert_eq!(
        resolve_workspace(Some(a.as_path()), Some(b.as_path()), &nested).unwrap(),
        fs::canonicalize(&a).unwrap()
    );
    let missing = temp.path().join("missing");
    assert_eq!(
        resolve_workspace(Some(&missing), Some(&b), &nested)
            .unwrap_err()
            .code,
        "WORKSPACE_NOT_FOUND"
    );
    assert!(!missing.exists());
}

#[test]
fn future_config_versions_are_rejected_before_use() {
    let temp = tempfile::tempdir().unwrap();
    initialize(temp.path(), &InitOptions::default()).unwrap();
    let path = temp.path().join("knowmesh.yaml");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(path, text.replacen("version: 1", "version: 99", 1)).unwrap();
    assert_eq!(
        Workspace::load(temp.path()).unwrap_err().code,
        "UNSUPPORTED_CONFIG_VERSION"
    );
}

#[test]
fn purpose_is_optional_but_configured_files_are_bounded_and_confined() {
    let temp = tempfile::tempdir().unwrap();
    initialize(temp.path(), &InitOptions::default()).unwrap();
    let path = temp.path().join("knowmesh.yaml");
    let mut value: serde_yaml::Value = serde_yaml::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["workspace"]["purpose"] = serde_yaml::Value::String("../outside.md".into());
    fs::write(&path, serde_yaml::to_string(&value).unwrap()).unwrap();
    assert_eq!(
        Workspace::load(temp.path()).unwrap_err().code,
        "PATH_OUTSIDE_WORKSPACE"
    );
    value["workspace"]["purpose"] = serde_yaml::Value::Null;
    fs::write(&path, serde_yaml::to_string(&value).unwrap()).unwrap();
    assert!(Workspace::load(temp.path()).unwrap().purpose.is_none());
    value["workspace"]["purpose"] = serde_yaml::Value::String("purpose.md".into());
    fs::write(&path, serde_yaml::to_string(&value).unwrap()).unwrap();
    fs::write(temp.path().join("purpose.md"), "x".repeat(16 * 1024 + 1)).unwrap();
    assert_eq!(
        Workspace::load(temp.path()).unwrap_err().code,
        "PURPOSE_TOO_LARGE"
    );
}

#[test]
fn model_environment_resolution_never_writes_or_formats_secrets() {
    let temp = tempfile::tempdir().unwrap();
    initialize(temp.path(), &InitOptions::default()).unwrap();
    let workspace = Workspace::load(temp.path()).unwrap();
    let mut env = BTreeMap::new();
    env.insert("KNOWMESH_LLM_API_KEY".into(), "fixture-secret".into());
    env.insert(
        "KNOWMESH_LLM_BASE_URL".into(),
        "https://example.invalid/v1".into(),
    );
    env.insert("KNOWMESH_COMPILER_MODEL".into(), "fixture-model".into());
    let resolved = workspace.config.compiler.resolve(&env).unwrap();
    assert_eq!(resolved.model, "fixture-model");
    assert!(!format!("{resolved:?}").contains("fixture-secret"));
    assert!(
        !fs::read_to_string(temp.path().join("knowmesh.yaml"))
            .unwrap()
            .contains("fixture-secret")
    );
    assert!(workspace.config.compiler.resolve(&BTreeMap::new()).is_err());
}

#[cfg(unix)]
#[test]
fn configured_purpose_cannot_escape_through_a_symlink() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    initialize(&root, &InitOptions::default()).unwrap();
    let outside = temp.path().join("outside.md");
    fs::write(&outside, "private data").unwrap();
    fs::remove_file(root.join("purpose.md")).unwrap();
    std::os::unix::fs::symlink(outside, root.join("purpose.md")).unwrap();
    assert_eq!(
        Workspace::load(&root).unwrap_err().code,
        "PATH_OUTSIDE_WORKSPACE"
    );
}

#[test]
fn general_template_has_no_research_specific_purpose() {
    let temp = tempfile::tempdir().unwrap();
    initialize(
        temp.path(),
        &InitOptions {
            template: "general".into(),
            ..InitOptions::default()
        },
    )
    .unwrap();
    let workspace = Workspace::load(temp.path()).unwrap();
    assert!(workspace.purpose.is_none());
    assert!(!temp.path().join("purpose.md").exists());
}

#[test]
fn invalid_ignore_file_is_detected_before_canonical_files_are_created() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join(".gitignore")).unwrap();
    assert!(initialize(temp.path(), &InitOptions::default()).is_err());
    assert!(!temp.path().join("knowmesh.yaml").exists());
    assert!(!temp.path().join("purpose.md").exists());
}

#[cfg(unix)]
#[test]
fn initialization_rejects_symlinked_ignore_before_writing_any_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let outside = temp.path().join("ignore");
    fs::write(&outside, "human ignore rules\n").unwrap();
    std::os::unix::fs::symlink(&outside, root.join(".gitignore")).unwrap();
    assert_eq!(
        initialize(&root, &InitOptions::default()).unwrap_err().code,
        "PATH_OUTSIDE_WORKSPACE"
    );
    assert!(!root.join("knowmesh.yaml").exists());
    assert!(!root.join("purpose.md").exists());
    assert_eq!(fs::read_to_string(outside).unwrap(), "human ignore rules\n");
}
