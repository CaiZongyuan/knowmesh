use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    canonical::{
        schema::{NodeType, Schema, SchemaPack},
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

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EntityInput {
    pub name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct EntityPredicate {
    pub name: String,
    pub label: String,
    pub directed: bool,
    pub inverse: Option<String>,
    pub evidence_required: bool,
    pub source_types: BTreeSet<String>,
    pub target_types: BTreeSet<String>,
    pub source: bool,
    pub target: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct EntityReport {
    pub schema_hash: String,
    pub entity: NodeType,
    pub predicates: Vec<EntityPredicate>,
}

pub fn entity(workspace: &Workspace, input: &EntityInput) -> AppResult<EntityReport> {
    if input.name.trim().is_empty() {
        return Err(AppError::new(
            ErrorType::Validation,
            "INVALID_ARGUMENT",
            "An entity type name is required.",
        )
        .with_param("name"));
    }
    let schema = Schema::load(workspace)?;
    let entity = schema.node_types.get(&input.name).ok_or_else(|| {
        AppError::new(
            ErrorType::NotFound,
            "SCHEMA_ENTITY_NOT_FOUND",
            "The entity type is not defined in the effective schema.",
        )
        .with_param("name")
        .with_hint("Run `knowmesh schema pack <pack-id>` to inspect the configured schema packs.")
    })?;
    let predicates = schema
        .predicates
        .iter()
        .filter(|(_, predicate)| {
            predicate.source_types.contains(&input.name)
                || predicate.target_types.contains(&input.name)
        })
        .map(|(name, predicate)| EntityPredicate {
            name: name.clone(),
            label: predicate.label.clone(),
            directed: predicate.directed,
            inverse: predicate.inverse.clone(),
            evidence_required: predicate.evidence_required,
            source_types: predicate.source_types.clone(),
            target_types: predicate.target_types.clone(),
            source: predicate.source_types.contains(&input.name),
            target: predicate.target_types.contains(&input.name),
        })
        .collect();
    Ok(EntityReport {
        schema_hash: schema.hash,
        entity: entity.clone(),
        predicates,
    })
}
