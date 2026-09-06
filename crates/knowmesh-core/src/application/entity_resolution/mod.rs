mod advice;
mod retrieve;
mod types;

use std::collections::{BTreeMap, BTreeSet};

pub use advice::{EntityAdvice, advise};
pub use retrieve::{EntityBatchData, EntityBatchQuery, EntityBatchReport, resolve_batch};
pub use types::*;

use crate::{
    canonical::schema::Schema,
    domain::{LifecycleStatus, NodeMetadata, normalize_name, sha256},
    error::{AppError, AppResult, ErrorType},
};

pub const RESOLVER_VERSION: &str = "2";

struct IndexedNode<'a> {
    metadata: &'a NodeMetadata,
    identifiers: BTreeMap<String, String>,
}

/// The complete catalog is indexed once; candidate display limits never decide uniqueness.
pub struct EntityResolver<'a> {
    schema: &'a Schema,
    nodes: Vec<IndexedNode<'a>>,
    names: BTreeMap<String, BTreeSet<usize>>,
    aliases: BTreeMap<String, BTreeSet<usize>>,
    identifiers: BTreeMap<(String, String), BTreeSet<usize>>,
    catalog_sha256: String,
    options_sha256: String,
    options: ResolutionOptions,
}

impl<'a> EntityResolver<'a> {
    pub fn new(
        schema: &'a Schema,
        nodes: &'a [NodeMetadata],
        options: ResolutionOptions,
    ) -> AppResult<Self> {
        if !(1..=100).contains(&options.candidate_limit)
            || !(1..=100_000).contains(&options.max_catalog_nodes)
        {
            return Err(error(
                "INVALID_RESOLUTION_OPTIONS",
                "Entity resolution options exceed supported bounds.",
            ));
        }
        if nodes.len() > options.max_catalog_nodes {
            return Err(error(
                "ENTITY_CATALOG_LIMIT",
                "The complete entity catalog exceeds its node budget.",
            ));
        }
        let mut ordered: Vec<_> = nodes.iter().collect();
        ordered.sort_by(|left, right| left.id.cmp(&right.id));
        if ordered.windows(2).any(|pair| pair[0].id == pair[1].id) {
            return Err(error(
                "DUPLICATE_ENTITY_ID",
                "An entity catalog cannot contain repeated Node IDs.",
            ));
        }
        let mut resolver = Self {
            schema,
            nodes: vec![],
            names: BTreeMap::new(),
            aliases: BTreeMap::new(),
            identifiers: BTreeMap::new(),
            catalog_sha256: hash(&(RESOLVER_VERSION, &schema.hash, &ordered))?,
            options_sha256: hash(&options)?,
            options,
        };
        for node in ordered {
            node.validate()?;
            validate_names(&node.name, &node.aliases)?;
            schema.validate_properties(&node.node_type, &node.properties)?;
            if !schema.packs.iter().any(|pack| pack.key() == node.schema) {
                return Err(error(
                    "ENTITY_SCHEMA_MISMATCH",
                    "An entity refers to a Schema pack outside the catalog schema.",
                ));
            }
            if node.lifecycle_status != LifecycleStatus::Active {
                continue;
            }
            let identifiers = schema.entity_identifiers(&node.node_type, &node.properties)?;
            let index = resolver.nodes.len();
            resolver
                .names
                .entry(normalize_name(&node.name))
                .or_default()
                .insert(index);
            for alias in &node.aliases {
                resolver
                    .aliases
                    .entry(normalize_name(alias))
                    .or_default()
                    .insert(index);
            }
            for (name, value) in &identifiers {
                resolver
                    .identifiers
                    .entry((name.clone(), value.clone()))
                    .or_default()
                    .insert(index);
            }
            resolver.nodes.push(IndexedNode {
                metadata: node,
                identifiers,
            });
        }
        Ok(resolver)
    }

    pub fn resolve(&self, input: &EntityInput) -> AppResult<ResolutionReport> {
        self.report(input, self.matching_candidates(input)?)
    }

    fn matching_candidates(&self, input: &EntityInput) -> AppResult<Vec<ResolutionCandidate>> {
        validate_names(&input.name, &input.aliases)?;
        let identifiers = self
            .schema
            .entity_identifiers(&input.node_type, &input.properties)?;
        let mut matches = BTreeMap::<usize, BTreeSet<String>>::new();
        for (name, value) in &identifiers {
            if let Some(indices) = self.identifiers.get(&(name.clone(), value.clone())) {
                for index in indices {
                    matches
                        .entry(*index)
                        .or_default()
                        .insert(format!("identifier:{name}"));
                }
            }
        }
        for (name, is_alias) in std::iter::once((&input.name, false))
            .chain(input.aliases.iter().map(|alias| (alias, true)))
        {
            let normalized = normalize_name(name);
            for (lookup, reason) in [
                (
                    &self.names,
                    if is_alias { "alias" } else { "canonical_name" },
                ),
                (&self.aliases, "alias"),
            ] {
                if let Some(indices) = lookup.get(&normalized) {
                    for index in indices {
                        matches.entry(*index).or_default().insert(reason.into());
                    }
                }
            }
        }
        let candidates: Vec<_> = matches
            .into_iter()
            .map(|(index, reasons)| {
                let indexed = &self.nodes[index];
                let node = indexed.metadata;
                let mut warnings = vec![];
                if identifiers.iter().any(|(key, value)| {
                    indexed
                        .identifiers
                        .get(key)
                        .is_some_and(|other| other != value)
                }) {
                    warnings.push("ENTITY_IDENTIFIER_CONFLICT".into());
                }
                if node.node_type != input.node_type {
                    warnings.push("ENTITY_TYPE_MISMATCH".into());
                }
                ResolutionCandidate {
                    node_id: node.id.clone(),
                    name: node.name.clone(),
                    node_type: node.node_type.clone(),
                    matched_by: reasons.into_iter().collect(),
                    retrieval_score: None,
                    warnings,
                }
            })
            .collect();
        Ok(candidates)
    }

    fn report(
        &self,
        input: &EntityInput,
        mut candidates: Vec<ResolutionCandidate>,
    ) -> AppResult<ResolutionReport> {
        candidates.sort_by(|left, right| {
            strength(left)
                .cmp(&strength(right))
                .then_with(|| {
                    right
                        .retrieval_score
                        .unwrap_or(0.0)
                        .total_cmp(&left.retrieval_score.unwrap_or(0.0))
                })
                .then_with(|| left.node_id.cmp(&right.node_id))
        });
        let (decision, selected, automatic) = decide(&candidates);
        let selected_node_id = selected.map(|candidate| candidate.node_id.clone());
        let total_candidates = candidates.len();
        let candidates_truncated = total_candidates > self.options.candidate_limit;
        candidates.truncate(self.options.candidate_limit);
        let warnings = if candidates_truncated {
            vec!["ENTITY_CANDIDATES_TRUNCATED".into()]
        } else {
            vec![]
        };
        Ok(ResolutionReport {
            decision,
            selected_node_id,
            automatic,
            candidates,
            total_candidates,
            candidates_truncated,
            retrieval_available: false,
            retrieval_sha256: None,
            catalog_sha256: self.catalog_sha256.clone(),
            input_sha256: hash(input)?,
            options_sha256: self.options_sha256.clone(),
            warnings,
        })
    }
}

fn decide(
    candidates: &[ResolutionCandidate],
) -> (ResolutionDecision, Option<&ResolutionCandidate>, bool) {
    let strong: Vec<_> = candidates
        .iter()
        .filter(|candidate| strength(candidate) == 0)
        .collect();
    match strong.as_slice() {
        [only] if only.warnings.is_empty() => {
            return (ResolutionDecision::Existing, Some(only), true);
        }
        [] => {}
        _ => return (ResolutionDecision::Ambiguous, None, false),
    }
    let deterministic: Vec<_> = candidates
        .iter()
        .filter(|candidate| strength(candidate) < 3)
        .collect();
    if deterministic.is_empty() {
        return match candidates {
            [] => (ResolutionDecision::New, None, false),
            [first, rest @ ..]
                if first.warnings.is_empty()
                    && rest.first().is_none_or(|second| {
                        first.retrieval_score.unwrap_or(0.0) - second.retrieval_score.unwrap_or(0.0)
                            > 0.05
                    }) =>
            {
                (ResolutionDecision::Existing, Some(first), false)
            }
            _ => (ResolutionDecision::Ambiguous, None, false),
        };
    }
    match deterministic.as_slice() {
        [] => (ResolutionDecision::New, None, false),
        [only] if only.warnings.is_empty() => (
            ResolutionDecision::Existing,
            Some(only),
            only.matched_by.iter().any(|reason| reason == "alias"),
        ),
        _ => (ResolutionDecision::Ambiguous, None, false),
    }
}

fn strength(candidate: &ResolutionCandidate) -> u8 {
    if candidate
        .matched_by
        .iter()
        .any(|reason| reason.starts_with("identifier:"))
    {
        0
    } else if candidate
        .matched_by
        .iter()
        .any(|reason| reason == "canonical_name")
    {
        1
    } else if candidate.matched_by.iter().any(|reason| reason == "alias") {
        2
    } else {
        3
    }
}

fn validate_names(name: &str, aliases: &[String]) -> AppResult<()> {
    if aliases.len() > 1024
        || std::iter::once(name)
            .chain(aliases.iter().map(String::as_str))
            .any(|name| name.trim().is_empty() || name.len() > 2048 || name.contains('\0'))
    {
        return Err(error(
            "INVALID_ENTITY_NAME",
            "Entity names and aliases must be nonempty bounded strings without NUL.",
        ));
    }
    Ok(())
}

fn hash(value: &impl serde::Serialize) -> AppResult<String> {
    serde_json::to_vec(value)
        .map(|bytes| sha256(&bytes))
        .map_err(|_| {
            error(
                "ENTITY_CONTEXT_INVALID",
                "Could not identify entity resolution context.",
            )
        })
}

fn error(code: &str, message: &str) -> AppError {
    AppError::new(ErrorType::Validation, code, message)
}
