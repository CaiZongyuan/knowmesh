use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    canonical::workspace::{self, InitOptions, InitReport},
    error::AppResult,
};

pub fn load(
    explicit: Option<&Path>,
    environment: Option<&Path>,
    cwd: &Path,
) -> AppResult<workspace::Workspace> {
    workspace::Workspace::load(&workspace::resolve_workspace(explicit, environment, cwd)?)
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InitInput {
    pub path: PathBuf,
    pub name: String,
    pub template: String,
    #[serde(default)]
    pub dry_run: bool,
}

pub fn init(input: &InitInput) -> AppResult<InitReport> {
    workspace::initialize(
        &input.path,
        &InitOptions {
            name: input.name.clone(),
            template: input.template.clone(),
            dry_run: input.dry_run,
        },
    )
}
