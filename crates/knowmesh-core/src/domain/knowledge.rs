use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    ClaimId, EvidenceId, NodeId, RelationId, RunId, SourceId, SourceRevisionId, SynthesisId,
    Timestamp, sha256, valid_sha256,
};
use crate::error::{AppError, AppResult, ErrorType};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleStatus {
    Active,
    Superseded,
    Retracted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStatus {
    Supported,
    Uncertain,
    Conflicting,
    Unreviewed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStance {
    Supports,
    Contradicts,
    Context,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionMethod {
    Parser,
    Model,
    Human,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Locator {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub section_path: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paragraph: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub char_start: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub char_end: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub id: EvidenceId,
    pub source_revision_id: SourceRevisionId,
    pub stance: EvidenceStance,
    pub quote: String,
    pub quote_sha256: String,
    pub locator: Locator,
    pub extraction_method: ExtractionMethod,
    pub confidence: f64,
}

impl Evidence {
    pub fn validate(&self) -> AppResult<()> {
        if self.quote.trim().is_empty()
            || self.quote.chars().count() > 1000
            || self.quote_sha256 != sha256(self.quote.as_bytes())
        {
            return Err(knowledge_error(
                "INVALID_EVIDENCE_QUOTE",
                "Evidence quotes must be nonempty, at most 1000 code points, and match their SHA-256.",
            ));
        }
        validate_confidence(self.confidence)?;
        if self.locator.page == Some(0)
            || self.locator.paragraph == Some(0)
            || self.locator.char_start.is_some() != self.locator.char_end.is_some()
            || self
                .locator
                .char_start
                .zip(self.locator.char_end)
                .is_some_and(|(start, end)| start >= end)
            || self.locator.section_path.len() > 64
            || self.locator.section_path.iter().any(|s| s.len() > 2048)
        {
            return Err(knowledge_error(
                "INVALID_EVIDENCE_LOCATOR",
                "Evidence locators require positive page/paragraph numbers and ordered character offsets.",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimRecord {
    pub id: ClaimId,
    pub statement: String,
    pub lifecycle_status: LifecycleStatus,
    pub evidence_status: EvidenceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub qualifiers: BTreeMap<String, Value>,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
}

impl ClaimRecord {
    pub fn normalized_hash(&self) -> AppResult<String> {
        semantic_hash(&(super::normalize_name(&self.statement), &self.qualifiers))
    }
    pub fn validate(&self) -> AppResult<()> {
        if self.statement.trim().is_empty() || self.statement.len() > 16 * 1024 {
            return Err(knowledge_error(
                "INVALID_CLAIM",
                "Claims require a nonempty bounded statement.",
            ));
        }
        validate_assertion(self.confidence, self.evidence_status, &self.evidence)
    }

    pub fn semantic_hash(&self, subject: &NodeId) -> AppResult<String> {
        self.validate()?;
        let evidence = ordered_evidence(&self.evidence);
        semantic_hash(&(
            subject,
            self.statement
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
            &self.qualifiers,
            self.lifecycle_status,
            self.evidence_status,
            evidence,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RelationRecord {
    pub id: RelationId,
    pub predicate: String,
    pub target_node_id: NodeId,
    pub directed: bool,
    pub lifecycle_status: LifecycleStatus,
    pub evidence_status: EvidenceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub qualifiers: BTreeMap<String, Value>,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
}

impl RelationRecord {
    pub fn validate(&self) -> AppResult<()> {
        if self.predicate.is_empty()
            || self.predicate.len() > 64
            || !self
                .predicate
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        {
            return Err(knowledge_error(
                "INVALID_RELATION",
                "Relation predicates must be bounded ASCII snake_case names.",
            ));
        }
        validate_assertion(self.confidence, self.evidence_status, &self.evidence)
    }

    pub fn semantic_hash(&self, source: &NodeId) -> AppResult<String> {
        self.validate()?;
        let (source, target) = if !self.directed && source > &self.target_node_id {
            (&self.target_node_id, source)
        } else {
            (source, &self.target_node_id)
        };
        semantic_hash(&(
            source,
            target,
            &self.predicate,
            self.directed,
            &self.qualifiers,
            self.lifecycle_status,
            self.evidence_status,
            ordered_evidence(&self.evidence),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Claim {
    pub subject_node_id: NodeId,
    #[serde(flatten)]
    pub assertion: ClaimRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Relation {
    pub source_node_id: NodeId,
    #[serde(flatten)]
    pub assertion: RelationRecord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Node,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NodeMetadata {
    pub version: u32,
    pub id: NodeId,
    pub kind: NodeKind,
    pub schema: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub lifecycle_status: LifecycleStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl NodeMetadata {
    pub fn validate(&self) -> AppResult<()> {
        if self.version != 1 {
            return Err(unsupported_version());
        }
        if self.name.trim().is_empty()
            || self.name.len() > 2048
            || self.node_type.is_empty()
            || self.updated_at < self.created_at
            || self.aliases.len() > 1024
            || self.tags.len() > 1024
        {
            return Err(knowledge_error(
                "INVALID_NODE_METADATA",
                "Node metadata is invalid or exceeds its limits.",
            ));
        }
        validate_schema_reference(&self.schema)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SynthesisKind {
    Synthesis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SynthesisStatus {
    Draft,
    Reviewed,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GeneratedBy {
    pub run_id: RunId,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssertionDependency {
    Claim {
        id: ClaimId,
        semantic_sha256: String,
    },
    Relation {
        id: RelationId,
        semantic_sha256: String,
    },
}

impl AssertionDependency {
    pub fn id(&self) -> &str {
        match self {
            Self::Claim { id, .. } => id.as_str(),
            Self::Relation { id, .. } => id.as_str(),
        }
    }
    pub fn hash(&self) -> &str {
        match self {
            Self::Claim {
                semantic_sha256, ..
            }
            | Self::Relation {
                semantic_sha256, ..
            } => semantic_sha256,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceHead {
    pub source_id: SourceId,
    pub revision_id: SourceRevisionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DependencySnapshot {
    pub version: u32,
    pub assertions: Vec<AssertionDependency>,
    pub source_heads: Vec<SourceHead>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SynthesisMetadata {
    pub version: u32,
    pub id: SynthesisId,
    pub kind: SynthesisKind,
    pub schema: String,
    pub title: String,
    pub question: String,
    pub status: SynthesisStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_by: Option<GeneratedBy>,
    #[serde(default)]
    pub related_nodes: Vec<NodeId>,
    #[serde(default)]
    pub evidence_ids: Vec<EvidenceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_snapshot: Option<DependencySnapshot>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl SynthesisMetadata {
    pub fn validate(&self) -> AppResult<()> {
        if self.version != 1 {
            return Err(unsupported_version());
        }
        if self.title.trim().is_empty()
            || self.title.len() > 2048
            || self.question.trim().is_empty()
            || self.question.len() > 16 * 1024
            || self.updated_at < self.created_at
        {
            return Err(knowledge_error(
                "INVALID_SYNTHESIS_METADATA",
                "Synthesis metadata is invalid.",
            ));
        }
        validate_schema_reference(&self.schema)?;
        if let Some(snapshot) = &self.dependency_snapshot {
            if snapshot.version != 1 {
                return Err(unsupported_version());
            }
            let mut ids = BTreeSet::new();
            for assertion in &snapshot.assertions {
                if !ids.insert(assertion.id()) || !valid_sha256(assertion.hash()) {
                    return Err(knowledge_error(
                        "INVALID_DEPENDENCY_SNAPSHOT",
                        "Assertion dependency IDs and hashes must be unique and valid.",
                    ));
                }
            }
            let mut source_ids = BTreeSet::new();
            if snapshot
                .source_heads
                .iter()
                .any(|head| !source_ids.insert(&head.source_id))
            {
                return Err(knowledge_error(
                    "INVALID_DEPENDENCY_SNAPSHOT",
                    "Source heads must be unique in a dependency snapshot.",
                ));
            }
        }
        Ok(())
    }
}

fn validate_assertion(
    confidence: Option<f64>,
    status: EvidenceStatus,
    evidence: &[Evidence],
) -> AppResult<()> {
    if let Some(confidence) = confidence {
        validate_confidence(confidence)?;
    }
    if evidence.is_empty() && status != EvidenceStatus::Unreviewed {
        return Err(knowledge_error(
            "EVIDENCE_REQUIRED",
            "Assertions without evidence must be marked unreviewed.",
        ));
    }
    if evidence.len() > 1024 {
        return Err(knowledge_error(
            "EVIDENCE_LIMIT_EXCEEDED",
            "An assertion contains too many evidence entries.",
        ));
    }
    let mut ids = BTreeSet::new();
    for item in evidence {
        item.validate()?;
        if !ids.insert(&item.id) {
            return Err(knowledge_error(
                "DUPLICATE_EVIDENCE_ID",
                "Evidence IDs must be unique.",
            ));
        }
    }
    Ok(())
}

fn validate_confidence(value: f64) -> AppResult<()> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(knowledge_error(
            "INVALID_CONFIDENCE",
            "Confidence must be a finite value between 0 and 1.",
        ));
    }
    Ok(())
}
fn validate_schema_reference(value: &str) -> AppResult<()> {
    if !value.split_once('@').is_some_and(|(id, version)| {
        !id.is_empty()
            && id.len() <= 64
            && id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            && version
                .parse::<u32>()
                .is_ok_and(|v| v > 0 && v.to_string() == version)
    }) {
        return Err(knowledge_error(
            "INVALID_SCHEMA_REFERENCE",
            "Expected a versioned Schema Pack ID.",
        ));
    }
    Ok(())
}
fn ordered_evidence(evidence: &[Evidence]) -> Vec<&Evidence> {
    let mut values: Vec<_> = evidence.iter().collect();
    values.sort_by(|a, b| a.id.cmp(&b.id));
    values
}
fn semantic_hash(value: &impl Serialize) -> AppResult<String> {
    serde_json::to_vec(value)
        .map(|bytes| sha256(&bytes))
        .map_err(|_| {
            knowledge_error(
                "SEMANTIC_HASH_FAILED",
                "Could not encode assertion semantics.",
            )
        })
}
pub(crate) fn knowledge_error(code: &str, message: &str) -> AppError {
    AppError::new(ErrorType::Validation, code, message)
}
pub(crate) fn unsupported_version() -> AppError {
    knowledge_error(
        "UNSUPPORTED_FORMAT_VERSION",
        "Only canonical document format version 1 is supported.",
    )
}
