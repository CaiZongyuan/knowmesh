use knowmesh_core::application::operations::{EffectLevel, describe, descriptors};
use knowmesh_core::error::ErrorType;
use std::collections::HashSet;

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
    assert_eq!(version.output_schema.as_value()["properties"]["version"]["type"], "string");
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

