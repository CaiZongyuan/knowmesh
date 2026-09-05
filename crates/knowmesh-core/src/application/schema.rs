use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    canonical::{
        schema::{Schema, SchemaPack},
        workspace::Workspace,
    },
    error::{AppError, AppResult, ErrorType},
};

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackInput {
    pub id: String,
}

pub fn pack(workspace: &Workspace, input: &PackInput) -> AppResult<SchemaPack> {
    let schema = Schema::load(workspace)?;
    let mut matches = schema
        .packs
        .into_iter()
        .filter(|pack| pack.id == input.id || pack.key() == input.id);
    let pack = matches.next().ok_or_else(|| {
        AppError::new(
            ErrorType::NotFound,
            "SCHEMA_PACK_NOT_FOUND",
            "The pack is not configured in this workspace.",
        )
    })?;
    if matches.next().is_some() {
        return Err(AppError::new(
            ErrorType::Validation,
            "AMBIGUOUS_SCHEMA_PACK",
            "Use the full pack ID and version, such as research@1.",
        ));
    }
    Ok(pack)
}
