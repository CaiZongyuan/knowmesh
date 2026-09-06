use knowmesh_core::application::operations::{EffectLevel, describe, descriptors};
use knowmesh_core::error::ErrorType;
use std::collections::HashSet;

#[path = "support/operations.rs"]
mod guard;

#[test]
fn operation_catalog_has_unique_names_and_structured_contracts() {
    let operations = descriptors();
    let names: HashSet<_> = operations.iter().map(|op| op.name.as_str()).collect();
    assert_eq!(operations.len(), names.len());
    for operation in &operations {
        assert!(!operation.input_schema.as_value().is_null());
        assert!(!operation.output_schema.as_value().is_null());
        assert!(!operation.policy.is_empty());
    }
    let version = describe("version").unwrap();
    assert_eq!(version.effect, EffectLevel::Read);
    assert!(!version.supports_dry_run);
    assert!(!version.supports_idempotency);
    assert_eq!(
        version.output_schema.as_value()["properties"]["version"]["type"],
        "string"
    );
    assert!(names.contains("schema.command"));
    assert!(names.contains("schema.list"));
}

#[test]
fn unknown_operations_have_a_stable_typed_error() {
    let error = describe("made.up").unwrap_err();
    assert_eq!(error.error_type, ErrorType::NotFound);
    assert_eq!(error.code, "OPERATION_NOT_FOUND");
    assert!(error.hint.is_some());
}

#[test]
fn public_cli_handlers_must_all_have_application_descriptors() {
    let source = include_str!("../../knowmesh/src/cli.rs");
    let descriptors = descriptors();
    let names = descriptors.iter().map(|operation| operation.name.as_str()).collect();
    let errors = guard::check_cli(source, &names);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn registration_guard_rejects_an_unregistered_handler_without_executing_it() {
    let names = HashSet::from(["version"]);
    let source = r#"
        enum Command { Version, Hidden }
        impl Command { fn operation_name(&self) -> &'static str {
            match self { Self::Version => "version", Self::Hidden => "unregistered.handler" }
        } }
    "#;
    assert!(guard::check_cli(source, &names).iter().any(|error| error.contains("unregistered.handler")));
    let valid = source.replace("unregistered.handler", "version");
    assert!(guard::check_cli(&valid, &names).is_empty());
}

#[test]
fn registration_guard_requires_an_inspectable_exhaustive_operation_mapping() {
    let names = HashSet::from(["version"]);
    for source in [
        "impl Command { fn operation_name(&self) -> &'static str { hidden_mapping(self) } }",
        "impl Command { fn operation_name(&self) -> &'static str { match self { _ => \"version\" } } }",
        "impl Command { fn operation_name(&self) -> &'static str { match self { Self::Version => hidden_name() } } }",
        "impl Command { fn something_else(&self) {} }",
    ] {
        assert!(!guard::check_cli(source, &names).is_empty(), "{source}");
    }
}
