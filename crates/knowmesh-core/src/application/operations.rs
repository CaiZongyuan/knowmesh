use schemars::{JsonSchema, Schema, schema_for};
use serde::{Deserialize, Serialize};

use crate::{
    canonical::workspace::InitReport,
    error::{AppError, AppResult, ErrorType},
    wire::API_CONTRACT_VERSION,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum EffectLevel {
    Read,
    RuntimeWrite,
    CanonicalWrite,
    DerivedWrite,
    DestructiveDerived,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct OperationDescriptor {
    pub name: String,
    #[schemars(with = "serde_json::Value")]
    pub input_schema: Schema,
    #[schemars(with = "serde_json::Value")]
    pub output_schema: Schema,
    pub effect: EffectLevel,
    pub supports_dry_run: bool,
    pub supports_idempotency: bool,
    pub policy: String,
}

impl OperationDescriptor {
    pub fn read<I: JsonSchema, O: JsonSchema>(name: &str) -> Self {
        Self {
            name: name.into(),
            input_schema: schema_for!(I),
            output_schema: schema_for!(O),
            effect: EffectLevel::Read,
            supports_dry_run: false,
            supports_idempotency: false,
            policy: "public".into(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmptyInput {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchemaCommandInput {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct VersionInfo {
    pub version: String,
    pub api_contract_version: String,
}

pub fn version() -> VersionInfo {
    VersionInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        api_contract_version: API_CONTRACT_VERSION.into(),
    }
}

pub fn descriptors() -> Vec<OperationDescriptor> {
    let mut init = OperationDescriptor::read::<super::workspace::InitInput, InitReport>("init");
    init.effect = EffectLevel::CanonicalWrite;
    init.supports_dry_run = true;
    init.supports_idempotency = true;
    init.policy = "workspace-initialization".into();
    vec![
        init,
        OperationDescriptor::read::<EmptyInput, VersionInfo>("version"),
        OperationDescriptor::read::<SchemaCommandInput, OperationDescriptor>("schema.command"),
        OperationDescriptor::read::<EmptyInput, Vec<OperationDescriptor>>("schema.list"),
        OperationDescriptor::read::<super::schema::PackInput, crate::canonical::schema::SchemaPack>(
            "schema.pack",
        ),
    ]
}

pub fn describe(name: &str) -> AppResult<OperationDescriptor> {
    descriptors()
        .into_iter()
        .find(|op| op.name == name)
        .ok_or_else(|| {
            AppError::new(
                ErrorType::NotFound,
                "OPERATION_NOT_FOUND",
                "The operation is not registered.",
            )
            .with_param("name")
            .with_hint("Run `knowmesh schema list` to discover available operations.")
        })
}
