use std::{collections::BTreeMap, fs};

use knowmesh_core::canonical::{
    schema::{Schema, SchemaPack, builtin},
    workspace::{InitOptions, Workspace, initialize},
};
use serde_json::json;

fn custom(id: &str, extends: &[&str], definitions: &str) -> SchemaPack {
    SchemaPack::parse(
        format!(
            "id: {id}\nversion: 1\ndisplay_name: Fixture\nextends: [{}]\n{definitions}",
            extends.join(", ")
        )
        .as_bytes(),
    )
    .unwrap()
}

#[test]
fn research_and_clinical_builtins_have_complete_constraints() {
    let research = Schema::compose(vec![
        builtin("research@1").unwrap(),
        builtin("base@1").unwrap(),
    ])
    .unwrap();
    for name in [
        "Paper",
        "Concept",
        "Method",
        "Model",
        "Dataset",
        "Benchmark",
        "Finding",
        "Hypothesis",
        "Experiment",
        "Gene",
        "CellType",
    ] {
        assert!(research.node_types.contains_key(name));
    }
    assert_eq!(research.predicates.len(), 12);
    research
        .validate_relation("evaluated_on", "Model", "Dataset", true)
        .unwrap();
    assert_eq!(
        research
            .validate_relation("evaluated_on", "Dataset", "Model", true)
            .unwrap_err()
            .code,
        "RELATION_TYPE_MISMATCH"
    );
    assert_eq!(
        research
            .validate_relation("evaluated_on", "Model", "Dataset", false)
            .unwrap_err()
            .code,
        "EVIDENCE_REQUIRED"
    );
    research
        .validate_relation("compared_with", "Method", "Model", true)
        .unwrap();
    let clinical = Schema::compose(vec![
        builtin("clinical@1").unwrap(),
        builtin("base@1").unwrap(),
    ])
    .unwrap();
    assert_eq!(clinical.predicates.len(), 10);
    assert!(clinical.policies.human_verification_required);
    assert!(!clinical.policies.allow_accept_all);
    assert!(!clinical.policies.allow_direct_apply);
    assert_eq!(
        serde_json::to_value(clinical.policies).unwrap()["review_mode"],
        "strict"
    );
}

#[test]
fn composition_is_order_independent_and_rejects_duplicate_ids() {
    let a = Schema::compose(vec![
        builtin("base@1").unwrap(),
        builtin("research@1").unwrap(),
    ])
    .unwrap();
    let b = Schema::compose(vec![
        builtin("research@1").unwrap(),
        builtin("base@1").unwrap(),
    ])
    .unwrap();
    assert_eq!(a.hash, b.hash);
    assert_eq!(
        Schema::compose(vec![builtin("base@1").unwrap(), builtin("base@1").unwrap()])
            .unwrap_err()
            .code,
        "DUPLICATE_SCHEMA_PACK"
    );
}

#[test]
fn cycles_and_missing_dependencies_have_typed_errors() {
    let a = custom("a", &["b@1"], "");
    let b = custom("b", &["a@1"], "");
    assert_eq!(
        Schema::compose(vec![a.clone(), b]).unwrap_err().code,
        "SCHEMA_CYCLE"
    );
    assert_eq!(
        Schema::compose(vec![a]).unwrap_err().code,
        "SCHEMA_DEPENDENCY_MISSING"
    );
}

#[test]
fn replacing_a_definition_requires_explicit_override_of_an_ancestor() {
    let replacement = "node_types:\n  Concept:\n    label: Specific Concept\n    color: '#112233'\n    icon: box\n";
    let implicit = custom("custom", &["base@1"], replacement);
    assert_eq!(
        Schema::compose(vec![builtin("base@1").unwrap(), implicit])
            .unwrap_err()
            .code,
        "SCHEMA_OVERRIDE_REQUIRED"
    );
    let explicit = custom(
        "custom",
        &["base@1"],
        &replacement.replace("    label:", "    override: true\n    label:"),
    );
    let schema = Schema::compose(vec![builtin("base@1").unwrap(), explicit]).unwrap();
    assert_eq!(schema.node_types["Concept"].label, "Specific Concept");
    let unrelated = custom(
        "custom",
        &[],
        &replacement.replace("    label:", "    override: true\n    label:"),
    );
    assert_eq!(
        Schema::compose(vec![builtin("base@1").unwrap(), unrelated])
            .unwrap_err()
            .code,
        "SCHEMA_OVERRIDE_NOT_INHERITED"
    );
}

#[test]
fn unknown_node_types_in_predicates_are_rejected() {
    let pack = custom(
        "invalid",
        &["base@1"],
        "predicates:\n  uses:\n    label: Uses\n    source_types: [Missing]\n    target_types: [Concept]\n    directed: true\n    inverse: null\n    evidence_required: true\n",
    );
    assert_eq!(
        Schema::compose(vec![builtin("base@1").unwrap(), pack])
            .unwrap_err()
            .code,
        "UNKNOWN_NODE_TYPE"
    );
}

#[test]
fn properties_validate_required_types_and_bounded_patterns() {
    let definition = "node_types:\n  Record:\n    label: Record\n    color: '#112233'\n    icon: box\n    properties:\n      code: {type: string, required: true, pattern: '^[A-Z]+$', max_length: 8}\n      count: {type: integer}\n";
    let schema = Schema::compose(vec![custom("custom", &[], definition)]).unwrap();
    schema
        .validate_properties(
            "Record",
            &BTreeMap::from([("code".into(), json!("ABC")), ("count".into(), json!(3))]),
        )
        .unwrap();
    assert_eq!(
        schema
            .validate_properties("Record", &BTreeMap::new())
            .unwrap_err()
            .code,
        "REQUIRED_PROPERTY_MISSING"
    );
    for value in [json!(10), json!("lowercase"), json!("TOOLONGCODE")] {
        assert!(
            schema
                .validate_properties("Record", &BTreeMap::from([("code".into(), value)]))
                .is_err()
        );
    }
    assert_eq!(
        schema
            .validate_properties(
                "Record",
                &BTreeMap::from([("code".into(), json!("ABC")), ("count".into(), json!(1.5))])
            )
            .unwrap_err()
            .code,
        "INVALID_PROPERTY_TYPE"
    );
    let dangerous = definition.replace("^[A-Z]+$", "(a+)\\1");
    assert_eq!(
        Schema::compose(vec![custom("custom", &[], &dangerous)])
            .unwrap_err()
            .code,
        "INVALID_SCHEMA_PATTERN"
    );
}

#[test]
fn workspace_schema_loading_is_read_only_and_supports_builtins() {
    let temp = tempfile::tempdir().unwrap();
    initialize(temp.path(), &InitOptions::default()).unwrap();
    let config_path = temp.path().join("knowmesh.yaml");
    let mut config: serde_yaml::Value =
        serde_yaml::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["schema"]["packs"] = serde_yaml::to_value(["builtin:research@1"]).unwrap();
    fs::write(&config_path, serde_yaml::to_string(&config).unwrap()).unwrap();
    let before = fs::read(&config_path).unwrap();
    let schema = Schema::load(&Workspace::load(temp.path()).unwrap()).unwrap();
    assert!(schema.node_types.contains_key("Model"));
    assert_eq!(before, fs::read(config_path).unwrap());
    assert_eq!(
        builtin("research@99").unwrap_err().code,
        "SCHEMA_PACK_NOT_FOUND"
    );
}

#[test]
fn schema_yaml_rejects_unknown_fields_duplicate_keys_and_non_ascii_identifiers() {
    for yaml in [
        "id: custom\nversion: 1\ndisplay_name: Test\nexecute: rm\n",
        "id: custom\nid: other\nversion: 1\ndisplay_name: Test\n",
        "id: custom\nversion: 1\ndisplay_name: Test\nnode_types:\n  wrong_case: {label: A, color: '#112233', icon: box}\n",
    ] {
        let result =
            SchemaPack::parse(yaml.as_bytes()).and_then(|pack| Schema::compose(vec![pack]));
        assert!(result.is_err());
    }
}

#[test]
fn clinical_template_initializes_with_strict_review_and_no_research_purpose() {
    let temp = tempfile::tempdir().unwrap();
    initialize(
        temp.path(),
        &InitOptions {
            template: "clinical".into(),
            ..InitOptions::default()
        },
    )
    .unwrap();
    let workspace = Workspace::load(temp.path()).unwrap();
    assert!(workspace.purpose.is_none());
    let schema = Schema::load(&workspace).unwrap();
    assert!(schema.node_types.contains_key("Disease"));
    assert!(schema.policies.human_verification_required);
    assert!(!schema.policies.allow_accept_all);
}

#[cfg(unix)]
#[test]
fn custom_schema_files_cannot_escape_the_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    initialize(&root, &InitOptions::default()).unwrap();
    let outside = temp.path().join("pack.yaml");
    fs::write(&outside, "private data").unwrap();
    fs::remove_file(root.join("schemas/research.yaml")).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("schemas/research.yaml")).unwrap();
    assert_eq!(
        Schema::load(&Workspace::load(&root).unwrap())
            .unwrap_err()
            .code,
        "PATH_OUTSIDE_WORKSPACE"
    );
}
