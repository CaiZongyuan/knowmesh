use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

use crate::{
    canonical::workspace::Workspace,
    domain::sha256,
    error::{AppError, AppResult, ErrorType},
};

use super::{ImpactInput, ImpactPosition};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u32,
    query_sha256: String,
    position: ImpactPosition,
}

pub(super) fn fingerprint(workspace: &Workspace, input: &ImpactInput) -> AppResult<String> {
    Ok(sha256(
        &serde_json::to_vec(&(
            "source.impact",
            &workspace.config.workspace.id,
            &input.source_id,
            &input.revision,
            input.kind,
        ))
        .map_err(|_| invalid_cursor())?,
    ))
}

pub(super) fn decode(value: &str, fingerprint: &str) -> AppResult<ImpactPosition> {
    if value.len() > 4096 {
        return Err(invalid_cursor());
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| invalid_cursor())?;
    let cursor: Cursor = serde_json::from_slice(&bytes).map_err(|_| invalid_cursor())?;
    if cursor.version != 1 {
        return Err(invalid_cursor());
    }
    if cursor.query_sha256 != fingerprint {
        return Err(AppError::new(
            ErrorType::Validation,
            "CURSOR_QUERY_MISMATCH",
            "The cursor belongs to another workspace, source, or filter.",
        ));
    }
    Ok(cursor.position)
}

pub(super) fn encode(fingerprint: &str, position: ImpactPosition) -> AppResult<String> {
    Ok(URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&Cursor {
            version: 1,
            query_sha256: fingerprint.to_owned(),
            position,
        })
        .map_err(|_| invalid_cursor())?,
    ))
}

fn invalid_cursor() -> AppError {
    AppError::new(
        ErrorType::Validation,
        "INVALID_CURSOR",
        "The impact cursor is invalid or unsupported.",
    )
    .with_param("cursor")
}
