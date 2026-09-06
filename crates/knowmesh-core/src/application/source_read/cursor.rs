use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

use super::{ListInput, ListPosition};
use crate::{
    canonical::workspace::Workspace,
    domain::sha256,
    error::{AppError, AppResult, ErrorType},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u32,
    query_sha256: String,
    position: ListPosition,
}

pub(super) fn fingerprint(workspace: &Workspace, input: &ListInput) -> AppResult<String> {
    Ok(sha256(
        &serde_json::to_vec(&(
            "source.list",
            &workspace.config.workspace.id,
            input.include_removed,
            &input.kind,
            &input.tag,
        ))
        .map_err(|_| invalid())?,
    ))
}

pub(super) fn decode(value: &str, fingerprint: &str) -> AppResult<ListPosition> {
    if value.len() > 4096 {
        return Err(invalid());
    }
    let bytes = URL_SAFE_NO_PAD.decode(value).map_err(|_| invalid())?;
    let cursor: Cursor = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if cursor.version != 1 {
        return Err(invalid());
    }
    if cursor.query_sha256 != fingerprint {
        return Err(AppError::new(
            ErrorType::Validation,
            "CURSOR_QUERY_MISMATCH",
            "The cursor belongs to another workspace or source filter.",
        ));
    }
    Ok(cursor.position)
}

pub(super) fn encode(fingerprint: &str, position: ListPosition) -> AppResult<String> {
    Ok(URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&Cursor {
            version: 1,
            query_sha256: fingerprint.into(),
            position,
        })
        .map_err(|_| invalid())?,
    ))
}

fn invalid() -> AppError {
    AppError::new(
        ErrorType::Validation,
        "INVALID_CURSOR",
        "The source list cursor is invalid or unsupported.",
    )
    .with_param("cursor")
}
