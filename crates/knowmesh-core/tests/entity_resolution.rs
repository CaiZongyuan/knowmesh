use std::collections::BTreeMap;

use knowmesh_core::{
    application::entity_resolution::{
        EntityInput, EntityResolver, ResolutionDecision, ResolutionOptions,
    },
    canonical::schema::{Schema, SchemaPack, builtin},
    domain::{LifecycleStatus, NodeId, NodeKind, NodeMetadata},
};
use serde_json::json;

fn schema() -> Schema {
    Schema::compose(vec![
        builtin("base@1").unwrap(),
        builtin("research@1").unwrap(),
    ])
    .unwrap()
}

fn node(name: &str, node_type: &str, aliases: &[&str]) -> NodeMetadata {
    NodeMetadata {
        version: 1,
        id: NodeId::new(),
        kind: NodeKind::Node,
        schema: "research@1".into(),
        node_type: node_type.into(),
        name: name.into(),
        aliases: aliases.iter().map(|name| (*name).into()).collect(),
        tags: vec![],
        lifecycle_status: LifecycleStatus::Active,
        created_at: "2026-09-06T00:00:00Z".parse().unwrap(),
        updated_at: "2026-09-06T00:00:00Z".parse().unwrap(),
        properties: BTreeMap::new(),
        extra: BTreeMap::new(),
    }
}

fn input(name: &str, node_type: &str) -> EntityInput {
    EntityInput {
        name: name.into(),
        node_type: node_type.into(),
        aliases: vec![],
        properties: BTreeMap::new(),
    }
}

#[test]
fn schema_doi_identity_can_link_a_renamed_paper_and_records_the_match_reason() {
    let schema = schema();
    let mut existing = node("Original title", "Paper", &[]);
    existing
        .properties
        .insert("doi".into(), json!("10.1234/ABC.1"));
    let nodes = vec![existing];
    let resolver = EntityResolver::new(&schema, &nodes, Default::default()).unwrap();
    let mut request = input("Translated title", "Paper");
    request
        .properties
        .insert("doi".into(), json!("https://doi.org/10.1234/abc.1"));
    let result = resolver.resolve(&request).unwrap();
    assert_eq!(result.decision, ResolutionDecision::Existing);
    assert_eq!(result.selected_node_id, Some(nodes[0].id.clone()));
    assert!(result.automatic);
    assert!(
        result.candidates[0]
            .matched_by
            .iter()
            .any(|reason| reason == "identifier:doi")
    );
}

#[test]
fn exact_canonical_name_is_a_review_suggestion_but_unique_compatible_alias_is_automatic() {
    let schema = schema();
    let nodes = vec![node("Canonical", "Model", &["ＦＯＯ   Bar"])];
    let resolver = EntityResolver::new(&schema, &nodes, Default::default()).unwrap();
    let named = resolver.resolve(&input("canonical", "Model")).unwrap();
    assert_eq!(named.decision, ResolutionDecision::Existing);
    assert!(!named.automatic);
    let aliased = resolver.resolve(&input("foo bar", "Model")).unwrap();
    assert!(aliased.automatic);
    assert_eq!(aliased.selected_node_id, Some(nodes[0].id.clone()));
    let wrong_type = resolver.resolve(&input("foo bar", "Dataset")).unwrap();
    assert_eq!(wrong_type.decision, ResolutionDecision::Ambiguous);
    assert!(!wrong_type.automatic);
    assert!(
        wrong_type.candidates[0]
            .warnings
            .iter()
            .any(|warning| warning == "ENTITY_TYPE_MISMATCH")
    );
}

#[test]
fn conflicting_identifiers_block_alias_linking_even_when_another_identifier_matches() {
    let schema = Schema::compose(vec![
        builtin("base@1").unwrap(),
        SchemaPack::parse(
            br#"
id: identity
version: 1
display_name: Identity
extends: [base@1]
node_types:
  Paper:
    label: Paper
    color: '#2563EB'
    icon: file-text
    properties:
      doi: {type: string, identifier: doi}
      catalog_id: {type: string, identifier: opaque}
"#,
        )
        .unwrap(),
    ])
    .unwrap();
    let mut existing = node("Canonical", "Paper", &["Alias"]);
    existing.schema = "identity@1".into();
    existing.properties = BTreeMap::from([
        ("doi".into(), json!("10.1234/same")),
        ("catalog_id".into(), json!("A")),
    ]);
    let nodes = vec![existing];
    let resolver = EntityResolver::new(&schema, &nodes, Default::default()).unwrap();
    let mut request = input("Alias", "Paper");
    request.properties = BTreeMap::from([
        ("doi".into(), json!("10.1234/same")),
        ("catalog_id".into(), json!("B")),
    ]);
    let result = resolver.resolve(&request).unwrap();
    assert_eq!(result.decision, ResolutionDecision::Ambiguous);
    assert!(!result.automatic);
    assert!(
        result.candidates[0]
            .warnings
            .iter()
            .any(|warning| warning == "ENTITY_IDENTIFIER_CONFLICT")
    );
}

#[test]
fn duplicate_identifiers_aliases_and_name_alias_collisions_never_choose_the_first_node() {
    let schema = schema();
    for use_identifier in [false, true] {
        let mut nodes = vec![
            node("One", "Paper", &["Alias"]),
            node("Two", "Paper", &["Alias"]),
        ];
        let mut request = input("Alias", "Paper");
        if use_identifier {
            for node in &mut nodes {
                node.properties
                    .insert("doi".into(), json!("10.1234/shared"));
            }
            request
                .properties
                .insert("doi".into(), json!("10.1234/shared"));
        }
        let resolver = EntityResolver::new(
            &schema,
            &nodes,
            ResolutionOptions {
                candidate_limit: 1,
                ..Default::default()
            },
        )
        .unwrap();
        let result = resolver.resolve(&request).unwrap();
        assert_eq!(result.decision, ResolutionDecision::Ambiguous);
        assert_eq!(result.selected_node_id, None);
        assert!(!result.automatic);
        assert_eq!(result.total_candidates, 2);
        assert!(result.candidates_truncated);
    }
    let nodes = vec![
        node("Collision", "Model", &[]),
        node("Other", "Model", &["Collision"]),
    ];
    let resolver = EntityResolver::new(&schema, &nodes, Default::default()).unwrap();
    assert_eq!(
        resolver
            .resolve(&input("Collision", "Model"))
            .unwrap()
            .decision,
        ResolutionDecision::Ambiguous
    );
}

#[test]
fn node_order_does_not_change_catalog_identity_or_candidate_decisions() {
    let schema = schema();
    let nodes = vec![
        node("One", "Model", &["Alias"]),
        node("Two", "Model", &["Alias"]),
    ];
    let request = input("Alias", "Model");
    let first = EntityResolver::new(&schema, &nodes, Default::default())
        .unwrap()
        .resolve(&request)
        .unwrap();
    let mut reversed = nodes.clone();
    reversed.reverse();
    let second = EntityResolver::new(&schema, &reversed, Default::default())
        .unwrap()
        .resolve(&request)
        .unwrap();
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(&second).unwrap()
    );
    reversed[0].aliases.push("New alias".into());
    let changed = EntityResolver::new(&schema, &reversed, Default::default())
        .unwrap()
        .resolve(&request)
        .unwrap();
    assert_ne!(first.catalog_sha256, changed.catalog_sha256);
}

#[test]
fn new_and_inactive_entities_are_reviewed_and_invalid_identifiers_are_not_matching_keys() {
    let schema = schema();
    let mut inactive = node("Old", "Model", &["Alias"]);
    inactive.lifecycle_status = LifecycleStatus::Retracted;
    let nodes = vec![inactive];
    let resolver = EntityResolver::new(&schema, &nodes, Default::default()).unwrap();
    let result = resolver.resolve(&input("Alias", "Model")).unwrap();
    assert_eq!(result.decision, ResolutionDecision::New);
    assert!(!result.automatic);
    for doi in [
        "",
        "https://example.com/10.1234/x",
        "10.1234/has space",
        "not-a-doi",
    ] {
        let mut request = input("Paper", "Paper");
        request.properties.insert("doi".into(), json!(doi));
        assert_eq!(
            resolver.resolve(&request).unwrap_err().code,
            "INVALID_ENTITY_IDENTIFIER"
        );
    }
}

#[test]
fn identifier_rules_are_explicit_schema_properties_and_gene_names_are_not_identifiers() {
    let schema = schema();
    let nodes = vec![node("TP53", "Gene", &[])];
    let resolver = EntityResolver::new(&schema, &nodes, Default::default()).unwrap();
    assert!(!resolver.resolve(&input("TP53", "Gene")).unwrap().automatic);
    let invalid = SchemaPack::parse(
        br#"
id: bad
version: 1
display_name: Bad
node_types:
  Gene:
    label: Gene
    color: '#2563EB'
    icon: dna
    properties:
      gene_id: {type: integer, identifier: ncbi_gene}
"#,
    )
    .unwrap_err();
    assert_eq!(invalid.code, "INVALID_SCHEMA_PROPERTY");
}
