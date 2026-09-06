use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use regex::{Regex, RegexBuilder};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::workspace::{Workspace, confined_existing_path, read_bounded};
use crate::{
    domain::sha256,
    error::{AppError, AppResult, ErrorType},
};

mod identifiers;
pub use identifiers::IdentifierNormalization;

const MAX_PACKS: usize = 128;
const MAX_TEXT_LENGTH: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchemaPack {
    pub id: String,
    pub version: u32,
    pub display_name: String,
    #[serde(default)]
    pub extends: Vec<String>,
    #[serde(default)]
    pub node_types: BTreeMap<String, NodeType>,
    #[serde(default)]
    pub predicates: BTreeMap<String, Predicate>,
    #[serde(default)]
    pub policies: Policies,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeType {
    pub label: String,
    pub color: String,
    pub icon: String,
    #[serde(default, rename = "override")]
    pub override_inherited: bool,
    #[serde(default)]
    pub properties: BTreeMap<String, Property>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Property {
    #[serde(rename = "type")]
    pub value_type: PropertyType,
    #[serde(default)]
    pub required: bool,
    pub pattern: Option<String>,
    pub max_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<IdentifierNormalization>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PropertyType {
    String,
    Number,
    Integer,
    Boolean,
    StringList,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Predicate {
    pub label: String,
    pub source_types: BTreeSet<String>,
    pub target_types: BTreeSet<String>,
    pub directed: bool,
    pub inverse: Option<String>,
    pub evidence_required: bool,
    #[serde(default, rename = "override")]
    pub override_inherited: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewMode {
    Relaxed,
    Strict,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Policies {
    pub review_mode: ReviewMode,
    pub compiler_requires_evidence: bool,
    pub synthesis_requires_citation: bool,
    pub human_verification_required: bool,
    pub allow_accept_all: bool,
    pub allow_direct_apply: bool,
}

impl Default for Policies {
    fn default() -> Self {
        Self {
            review_mode: ReviewMode::Relaxed,
            compiler_requires_evidence: true,
            synthesis_requires_citation: true,
            human_verification_required: false,
            allow_accept_all: true,
            allow_direct_apply: false,
        }
    }
}

impl Policies {
    fn restrict(&mut self, other: &Self) {
        if other.review_mode == ReviewMode::Strict {
            self.review_mode = ReviewMode::Strict;
        }
        self.compiler_requires_evidence |= other.compiler_requires_evidence;
        self.synthesis_requires_citation |= other.synthesis_requires_citation;
        self.human_verification_required |= other.human_verification_required;
        self.allow_accept_all &= other.allow_accept_all;
        self.allow_direct_apply &= other.allow_direct_apply;
        if self.review_mode == ReviewMode::Strict || self.human_verification_required {
            self.allow_accept_all = false;
            self.allow_direct_apply = false;
        }
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Schema {
    pub packs: Vec<SchemaPack>,
    pub node_types: BTreeMap<String, NodeType>,
    pub predicates: BTreeMap<String, Predicate>,
    pub policies: Policies,
    pub hash: String,
    #[serde(skip)]
    patterns: BTreeMap<(String, String), Regex>,
}

impl SchemaPack {
    pub fn parse(bytes: &[u8]) -> AppResult<Self> {
        if bytes.len() > 1024 * 1024 {
            return Err(schema_error(
                "SCHEMA_TOO_LARGE",
                "Schema packs must not exceed 1 MiB.",
            ));
        }
        let pack: Self = serde_yaml::from_slice(bytes).map_err(|_| {
            schema_error(
                "INVALID_SCHEMA_PACK",
                "Schema YAML contains invalid, repeated, or unknown fields.",
            )
        })?;
        pack.validate()?;
        Ok(pack)
    }

    pub fn key(&self) -> String {
        format!("{}@{}", self.id, self.version)
    }

    fn validate(&self) -> AppResult<()> {
        if !identifier(&self.id, b'-')
            || self.version == 0
            || !label(&self.display_name)
            || self.node_types.len() > 1024
            || self.predicates.len() > 1024
            || self.extends.len() > MAX_PACKS
            || self.extends.iter().any(|id| !pack_reference(id))
        {
            return Err(schema_error(
                "INVALID_SCHEMA_PACK",
                "Schema metadata or dependency references are invalid.",
            ));
        }
        for (name, node) in &self.node_types {
            if !type_name(name)
                || !label(&node.label)
                || node.color.len() != 7
                || !node.color.starts_with('#')
                || !node.color[1..].bytes().all(|c| c.is_ascii_hexdigit())
                || !identifier(&node.icon, b'-')
                || node.properties.len() > 128
            {
                return Err(schema_error(
                    "INVALID_SCHEMA_DEFINITION",
                    "Node type metadata is invalid.",
                ));
            }
            for (name, property) in &node.properties {
                if !identifier(name, b'_')
                    || property
                        .max_length
                        .is_some_and(|length| length == 0 || length > MAX_TEXT_LENGTH)
                    || (property.value_type != PropertyType::String
                        && (property.pattern.is_some()
                            || property.max_length.is_some()
                            || property.identifier.is_some()))
                {
                    return Err(schema_error(
                        "INVALID_SCHEMA_PROPERTY",
                        "Property definition or length limit is invalid.",
                    ));
                }
            }
        }
        for (name, predicate) in &self.predicates {
            if !identifier(name, b'_')
                || !label(&predicate.label)
                || predicate.source_types.is_empty()
                || predicate.target_types.is_empty()
                || predicate.source_types.len() > 1024
                || predicate.target_types.len() > 1024
                || predicate
                    .inverse
                    .as_ref()
                    .is_some_and(|name| !identifier(name, b'_'))
            {
                return Err(schema_error(
                    "INVALID_SCHEMA_DEFINITION",
                    "Predicate definition is invalid.",
                ));
            }
        }
        Ok(())
    }
}

impl Schema {
    pub fn load(workspace: &Workspace) -> AppResult<Self> {
        if workspace.config.schema.packs.len() > MAX_PACKS {
            return Err(schema_error(
                "SCHEMA_LIMIT_EXCEEDED",
                "Too many schema packs.",
            ));
        }
        let mut packs = Vec::new();
        for reference in &workspace.config.schema.packs {
            packs.push(if let Some(key) = reference.strip_prefix("builtin:") {
                builtin(key)?
            } else {
                SchemaPack::parse(&read_bounded(
                    &confined_existing_path(&workspace.root, Path::new(reference))?,
                    1024 * 1024,
                )?)?
            });
        }
        let mut included: BTreeSet<_> = packs.iter().map(SchemaPack::key).collect();
        let mut index = 0;
        while index < packs.len() {
            for dependency in packs[index].extends.clone() {
                if included.insert(dependency.clone()) {
                    if packs.len() >= MAX_PACKS {
                        return Err(schema_error(
                            "SCHEMA_LIMIT_EXCEEDED",
                            "Too many schema packs.",
                        ));
                    }
                    packs.push(builtin(&dependency).map_err(|_| {
                        schema_error(
                            "SCHEMA_DEPENDENCY_MISSING",
                            "Include the referenced custom pack in knowmesh.yaml.",
                        )
                    })?);
                }
            }
            index += 1;
        }
        Self::compose(packs)
    }

    pub fn compose(packs: Vec<SchemaPack>) -> AppResult<Self> {
        if packs.is_empty() || packs.len() > MAX_PACKS {
            return Err(schema_error(
                "SCHEMA_LIMIT_EXCEEDED",
                "Expected between 1 and 128 schema packs.",
            ));
        }
        let mut registry = BTreeMap::new();
        for pack in packs {
            pack.validate()?;
            if registry.insert(pack.key(), pack).is_some() {
                return Err(schema_error(
                    "DUPLICATE_SCHEMA_PACK",
                    "Schema pack IDs and versions must be unique.",
                ));
            }
        }
        let mut order = Vec::new();
        let mut visiting = BTreeSet::new();
        let mut ancestors = BTreeMap::new();
        for key in registry.keys() {
            visit(key, &registry, &mut visiting, &mut ancestors, &mut order)?;
        }
        let mut schema = Self {
            packs: Vec::new(),
            node_types: BTreeMap::new(),
            predicates: BTreeMap::new(),
            policies: Policies::default(),
            hash: String::new(),
            patterns: BTreeMap::new(),
        };
        let mut node_owners = BTreeMap::new();
        let mut predicate_owners = BTreeMap::new();
        for key in order {
            let pack = &registry[&key];
            for (name, definition) in &pack.node_types {
                check_override(
                    name,
                    &key,
                    definition.override_inherited,
                    &ancestors[&key],
                    &mut node_owners,
                )?;
                schema.node_types.insert(name.clone(), definition.clone());
            }
            for (name, definition) in &pack.predicates {
                check_override(
                    name,
                    &key,
                    definition.override_inherited,
                    &ancestors[&key],
                    &mut predicate_owners,
                )?;
                schema.predicates.insert(name.clone(), definition.clone());
            }
            schema.policies.restrict(&pack.policies);
            schema.packs.push(pack.clone());
        }
        for predicate in schema.predicates.values() {
            for name in predicate.source_types.iter().chain(&predicate.target_types) {
                if !schema.node_types.contains_key(name) {
                    return Err(schema_error(
                        "UNKNOWN_NODE_TYPE",
                        "A predicate refers to a node type absent from the composed schema.",
                    ));
                }
            }
            if !predicate.directed && predicate.source_types != predicate.target_types {
                return Err(schema_error(
                    "INVALID_UNDIRECTED_PREDICATE",
                    "Undirected predicates require identical endpoint type sets.",
                ));
            }
        }
        for (node_name, node) in &schema.node_types {
            for (property_name, property) in &node.properties {
                if let Some(pattern) = &property.pattern {
                    if pattern.len() > 1024 {
                        return Err(invalid_pattern());
                    }
                    let regex = RegexBuilder::new(pattern)
                        .size_limit(1024 * 1024)
                        .nest_limit(64)
                        .build()
                        .map_err(|_| invalid_pattern())?;
                    schema
                        .patterns
                        .insert((node_name.clone(), property_name.clone()), regex);
                }
            }
        }
        schema.hash = sha256(
            &serde_json::to_vec(&(
                &schema.packs,
                &schema.node_types,
                &schema.predicates,
                &schema.policies,
            ))
            .map_err(|_| {
                schema_error("SCHEMA_HASH_FAILED", "Could not hash the composed schema.")
            })?,
        );
        Ok(schema)
    }

    pub fn validate_relation(
        &self,
        predicate: &str,
        source_type: &str,
        target_type: &str,
        has_evidence: bool,
    ) -> AppResult<()> {
        let definition = self.predicates.get(predicate).ok_or_else(|| {
            schema_error(
                "UNKNOWN_PREDICATE",
                "The predicate is not defined in this workspace.",
            )
        })?;
        if !definition.source_types.contains(source_type)
            || !definition.target_types.contains(target_type)
        {
            return Err(schema_error(
                "RELATION_TYPE_MISMATCH",
                "The predicate does not permit these endpoint types.",
            ));
        }
        if definition.evidence_required && !has_evidence {
            return Err(schema_error(
                "EVIDENCE_REQUIRED",
                "This predicate requires verified evidence.",
            ));
        }
        Ok(())
    }

    pub fn validate_properties(
        &self,
        node_type: &str,
        properties: &BTreeMap<String, Value>,
    ) -> AppResult<()> {
        let definition = self.node_types.get(node_type).ok_or_else(|| {
            schema_error(
                "UNKNOWN_NODE_TYPE",
                "The node type is not defined in this workspace.",
            )
        })?;
        for (name, property) in &definition.properties {
            if property.required && properties.get(name).is_none_or(Value::is_null) {
                return Err(schema_error(
                    "REQUIRED_PROPERTY_MISSING",
                    "A required node property is missing.",
                )
                .with_param(name));
            }
        }
        for (name, value) in properties {
            let property = definition.properties.get(name).ok_or_else(|| {
                schema_error(
                    "UNKNOWN_PROPERTY",
                    "The property is not defined for this node type.",
                )
                .with_param(name)
            })?;
            let valid_type = match property.value_type {
                PropertyType::String => value.is_string(),
                PropertyType::Number => value.is_number(),
                PropertyType::Integer => value.is_i64() || value.is_u64(),
                PropertyType::Boolean => value.is_boolean(),
                PropertyType::StringList => value.as_array().is_some_and(|items| {
                    items.len() <= 1024
                        && items
                            .iter()
                            .all(|v| v.as_str().is_some_and(|s| s.len() <= MAX_TEXT_LENGTH))
                }),
            };
            if !valid_type {
                return Err(schema_error(
                    "INVALID_PROPERTY_TYPE",
                    "A node property has the wrong value type.",
                )
                .with_param(name));
            }
            if let Some(text) = value.as_str() {
                if text.len() > property.max_length.unwrap_or(MAX_TEXT_LENGTH) {
                    return Err(schema_error(
                        "PROPERTY_TOO_LONG",
                        "A property exceeds its UTF-8 byte limit.",
                    )
                    .with_param(name));
                }
                if self
                    .patterns
                    .get(&(node_type.to_owned(), name.clone()))
                    .is_some_and(|pattern| !pattern.is_match(text))
                {
                    return Err(schema_error(
                        "PROPERTY_PATTERN_MISMATCH",
                        "A property does not match the configured pattern.",
                    )
                    .with_param(name));
                }
            }
        }
        Ok(())
    }

    pub fn entity_identifiers(
        &self,
        node_type: &str,
        properties: &BTreeMap<String, Value>,
    ) -> AppResult<BTreeMap<String, String>> {
        self.validate_properties(node_type, properties)?;
        let mut identifiers = BTreeMap::new();
        for (name, value) in properties {
            if let Some(rule) = self.node_types[node_type].properties[name].identifier {
                identifiers.insert(
                    name.clone(),
                    rule.normalize(value.as_str().expect("validated string property"))
                        .map_err(|error| error.with_param(name))?,
                );
            }
        }
        Ok(identifiers)
    }
}

fn visit(
    key: &str,
    registry: &BTreeMap<String, SchemaPack>,
    visiting: &mut BTreeSet<String>,
    ancestors: &mut BTreeMap<String, BTreeSet<String>>,
    order: &mut Vec<String>,
) -> AppResult<()> {
    if ancestors.contains_key(key) {
        return Ok(());
    }
    if !visiting.insert(key.to_owned()) {
        return Err(schema_error(
            "SCHEMA_CYCLE",
            "Schema pack inheritance contains a cycle.",
        ));
    }
    let pack = registry.get(key).ok_or_else(|| {
        schema_error(
            "SCHEMA_DEPENDENCY_MISSING",
            "A required schema pack is missing.",
        )
    })?;
    let mut inherited = BTreeSet::new();
    let dependencies: BTreeSet<_> = pack.extends.iter().collect();
    for dependency in dependencies {
        visit(dependency, registry, visiting, ancestors, order)?;
        inherited.insert(dependency.clone());
        inherited.extend(ancestors[dependency].iter().cloned());
    }
    visiting.remove(key);
    ancestors.insert(key.to_owned(), inherited);
    order.push(key.to_owned());
    Ok(())
}

fn check_override(
    name: &str,
    pack: &str,
    explicit: bool,
    ancestors: &BTreeSet<String>,
    owners: &mut BTreeMap<String, String>,
) -> AppResult<()> {
    if let Some(owner) = owners.get(name) {
        if !explicit {
            return Err(schema_error(
                "SCHEMA_OVERRIDE_REQUIRED",
                "Replacing a definition requires override: true.",
            ));
        }
        if !ancestors.contains(owner) {
            return Err(schema_error(
                "SCHEMA_OVERRIDE_NOT_INHERITED",
                "A pack can only override a definition owned by an ancestor.",
            ));
        }
    } else if explicit {
        return Err(schema_error(
            "SCHEMA_OVERRIDE_NOT_INHERITED",
            "An override must replace an inherited definition.",
        ));
    }
    owners.insert(name.to_owned(), pack.to_owned());
    Ok(())
}

pub fn builtin(key: &str) -> AppResult<SchemaPack> {
    let text = match key {
        "base@1" => include_str!("../../../../schemas/base.yaml"),
        "research@1" => include_str!("../../../../schemas/research.yaml"),
        "clinical@1" => include_str!("../../../../schemas/clinical.yaml"),
        _ => {
            return Err(schema_error(
                "SCHEMA_PACK_NOT_FOUND",
                "The requested builtin schema pack does not exist.",
            ));
        }
    };
    SchemaPack::parse(text.as_bytes())
}

fn identifier(value: &str, separator: u8) -> bool {
    value.len() <= 64
        && value.bytes().next().is_some_and(|c| c.is_ascii_lowercase())
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == separator)
}
fn type_name(value: &str) -> bool {
    value.len() <= 64
        && value.bytes().next().is_some_and(|c| c.is_ascii_uppercase())
        && value.bytes().all(|c| c.is_ascii_alphanumeric())
}
fn pack_reference(value: &str) -> bool {
    value.split_once('@').is_some_and(|(id, version)| {
        identifier(id, b'-')
            && version
                .parse::<u32>()
                .is_ok_and(|v| v > 0 && v.to_string() == version)
    })
}
fn label(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}
fn invalid_pattern() -> AppError {
    schema_error(
        "INVALID_SCHEMA_PATTERN",
        "Patterns must use bounded Rust regex syntax without lookaround or backreferences.",
    )
}
fn schema_error(code: &str, message: &str) -> AppError {
    AppError::new(ErrorType::Validation, code, message)
        .with_hint("Check the workspace Schema Pack definitions.")
}
