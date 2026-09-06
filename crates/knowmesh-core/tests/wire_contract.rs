use knowmesh_core::{
    error::{AppError, ErrorType},
    wire::{Failure, Metadata, Success},
};
use serde_json::{Value, json};

fn metadata() -> Metadata {
    Metadata::new(
        "source.list",
        "run_01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
        32,
    )
}

#[test]
fn every_error_type_has_stable_cli_and_http_mappings() {
    let actual: Vec<_> = [
        ErrorType::Validation,
        ErrorType::NotFound,
        ErrorType::Configuration,
        ErrorType::Io,
        ErrorType::Network,
        ErrorType::Internal,
        ErrorType::Policy,
        ErrorType::Conflict,
        ErrorType::Model,
        ErrorType::Confirmation,
        ErrorType::Cancelled,
    ]
    .into_iter()
    .map(|kind| {
        let error = AppError::new(kind, "FUTURE_ERROR_CODE", "A diagnostic.");
        json!({
            "type": kind,
            "exit": error.exit_code(),
            "http": error.http_status(),
        })
    })
    .collect();
    let expected: Value =
        serde_json::from_str(include_str!("snapshots/error-mappings.json")).unwrap();
    assert_eq!(json!(actual), expected);
}

#[test]
fn timeout_mapping_uses_the_typed_code_and_preserves_exit_and_body() {
    let error = AppError::new(ErrorType::Network, "FETCH_TIMEOUT", "Timed out.")
        .retryable(true)
        .with_hint("Retry the import.");
    assert_eq!(error.http_status(), 504);
    assert_eq!(error.exit_code(), 4);
    let encoded = serde_json::to_value(&error).unwrap();
    let decoded: AppError = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(decoded.http_status(), 504);
    assert_eq!(serde_json::to_value(&decoded).unwrap(), encoded);

    let renamed =
        AppError::new(ErrorType::Network, "FETCH_FAILED", "FETCH_TIMEOUT").retryable(true);
    assert_eq!(renamed.http_status(), 502);
    let different_type = AppError::new(ErrorType::Internal, "FETCH_TIMEOUT", "Timed out.");
    assert_eq!(different_type.http_status(), 500);
}

#[test]
fn full_envelope_snapshots_preserve_fields_and_omit_absent_options() {
    let mut page_meta = metadata();
    page_meta.workspace_id = Some("ws_01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap());
    page_meta.next_cursor = Some("opaque-continuation".into());
    let error = AppError::new(
        ErrorType::NotFound,
        "SOURCE_NOT_FOUND",
        "Source was not found.",
    )
    .with_hint("Run knowmesh source list to resolve the source first.")
    .with_param("source_id")
    .with_details(json!({"source_id": "src_01ARZ3NDEKTSV4RRFFQ69G5FAV"}));
    let actual = json!({
        "success": Success::new(json!({"items": []}), metadata()),
        "page": Success::new(json!({"items": []}), page_meta),
        "failure": Failure::new(error, metadata()),
        "minimal_failure": Failure::new(
            AppError::new(ErrorType::Internal, "INVARIANT_VIOLATION", "Invalid state."),
            metadata(),
        ),
    });
    let expected: Value = serde_json::from_str(include_str!("snapshots/envelopes.json")).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn error_consumers_ignore_future_fields() {
    let value = json!({
        "type": "conflict",
        "code": "FUTURE_CONFLICT",
        "message": "Conflict.",
        "retryable": false,
        "future_field": {"value": true},
    });
    let error: AppError = serde_json::from_value(value).unwrap();
    assert_eq!(error.exit_code(), 7);
    assert_eq!(error.http_status(), 409);
}
