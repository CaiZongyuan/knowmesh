use knowmesh_core::domain::{NodeId, RunId, SourceId, Timestamp, sha256};
use knowmesh_core::error::{AppError, ErrorType};
use proptest::prelude::*;

#[test]
fn ids_are_stable_typed_strings_across_json_round_trips() {
    let id = NodeId::new();
    assert!(id.as_str().starts_with("kn_"));
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(serde_json::from_str::<NodeId>(&json).unwrap(), id);
    assert!(serde_json::from_str::<SourceId>(&json).is_err());
    assert!(id.as_str().parse::<SourceId>().is_err());
    assert_ne!(NodeId::new(), id);
    assert!(RunId::new().as_str().starts_with("run_"));
}

#[test]
fn ids_reject_invalid_prefix_payload_and_ulid_overflow() {
    for invalid in [
        "kn_",
        "kn_NOT_A_ULID",
        "kn_ZZZZZZZZZZZZZZZZZZZZZZZZZZ",
        "src_01ARZ3NDEKTSV4RRFFQ69G5FAV",
    ] {
        assert!(invalid.parse::<NodeId>().is_err(), "accepted {invalid}");
        assert!(serde_json::from_value::<NodeId>(serde_json::json!(invalid)).is_err());
    }
}

proptest! {
    #[test]
    fn arbitrary_user_strings_never_panic_in_id_parsing(value in ".*") {
        if let Ok(id) = value.parse::<NodeId>() {
            prop_assert_eq!(id.to_string(), value);
        }
    }
}

#[test]
fn hashes_and_timestamps_have_canonical_wire_formats() {
    assert_eq!(
        sha256(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let time: Timestamp = "2026-09-05T23:00:00+08:00".parse().unwrap();
    assert_eq!(time.to_string(), "2026-09-05T15:00:00Z");
    assert!("not-a-time".parse::<Timestamp>().is_err());
    assert_eq!(serde_json::to_value(time).unwrap(), "2026-09-05T15:00:00Z");
}

#[test]
fn error_contract_preserves_machine_fields_and_exit_mapping() {
    let error = AppError::new(
        ErrorType::Conflict,
        "STALE_PROPOSAL",
        "The proposal is stale.",
    )
    .with_hint("Create a new proposal.")
    .with_param("proposal_id")
    .with_details(serde_json::json!({"expected_generation": 3}))
    .retryable(false);
    assert_eq!(error.exit_code(), 7);
    assert_eq!(
        serde_json::to_value(&error).unwrap(),
        serde_json::json!({
            "type": "conflict",
            "code": "STALE_PROPOSAL",
            "message": "The proposal is stale.",
            "hint": "Create a new proposal.",
            "retryable": false,
            "param": "proposal_id",
            "details": {"expected_generation": 3}
        })
    );
    assert_eq!(
        AppError::new(ErrorType::Cancelled, "RUN_CANCELLED", "Cancelled.").exit_code(),
        130
    );
}
