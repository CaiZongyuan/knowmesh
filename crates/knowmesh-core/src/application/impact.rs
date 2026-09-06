mod cursor;

use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    canonical::{snapshot::CanonicalSnapshot, workspace::Workspace},
    domain::{
        ClaimId, DependencySnapshot, EvidenceId, RelationId, SourceId, SourceRevisionId,
        SynthesisId,
        freshness::{FreshnessContext, FreshnessReport, assertion_freshness, synthesis_freshness},
    },
    error::{AppError, AppResult, ErrorType},
    ports::{ImpactPreviewBackend, ImpactStore},
};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ImpactKind {
    Claim,
    Evidence,
    Relation,
    Synthesis,
}

impl ImpactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claim => "claim",
            Self::Evidence => "evidence",
            Self::Relation => "relation",
            Self::Synthesis => "synthesis",
        }
    }
}

impl FromStr for ImpactKind {
    type Err = AppError;
    fn from_str(value: &str) -> AppResult<Self> {
        match value {
            "claim" => Ok(Self::Claim),
            "evidence" => Ok(Self::Evidence),
            "relation" => Ok(Self::Relation),
            "synthesis" => Ok(Self::Synthesis),
            _ => Err(AppError::new(
                ErrorType::Validation,
                "INVALID_IMPACT_KIND",
                "Choose claim, evidence, relation, or synthesis.",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum ImpactObject {
    Claim(ClaimId),
    Evidence(EvidenceId),
    Relation(RelationId),
    Synthesis(SynthesisId),
}

impl ImpactObject {
    pub fn kind(&self) -> ImpactKind {
        match self {
            Self::Claim(_) => ImpactKind::Claim,
            Self::Evidence(_) => ImpactKind::Evidence,
            Self::Relation(_) => ImpactKind::Relation,
            Self::Synthesis(_) => ImpactKind::Synthesis,
        }
    }
    pub fn id(&self) -> &str {
        match self {
            Self::Claim(id) => id.as_str(),
            Self::Evidence(id) => id.as_str(),
            Self::Relation(id) => id.as_str(),
            Self::Synthesis(id) => id.as_str(),
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ImpactReason {
    SourceRevision,
    EvidenceReference,
    AssertionDependency,
    SourceHead,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImpactInput {
    pub source_id: SourceId,
    #[serde(default)]
    pub revision: Option<SourceRevisionId>,
    #[serde(default)]
    pub kind: Option<ImpactKind>,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub no_sync: bool,
}

fn default_limit() -> u32 {
    20
}

#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct ImpactCounts {
    pub evidence: u64,
    pub claims: u64,
    pub relations: u64,
    pub syntheses: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ImpactItem {
    #[serde(flatten)]
    pub object: ImpactObject,
    pub dependency_ids: Vec<String>,
    pub reasons: Vec<ImpactReason>,
    #[serde(flatten)]
    pub freshness: FreshnessReport,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ImpactReport {
    pub preview: bool,
    pub source_id: SourceId,
    pub revision: Option<SourceRevisionId>,
    pub generation: u64,
    pub index_complete: bool,
    pub counts: ImpactCounts,
    pub items: Vec<ImpactItem>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactPosition {
    pub generation: u64,
    pub snapshot_sha256: String,
    pub after: ImpactObject,
}

pub struct ImpactQuery {
    pub source_id: SourceId,
    pub revision: Option<SourceRevisionId>,
    pub kind: Option<ImpactKind>,
    pub limit: u32,
    pub position: Option<ImpactPosition>,
}

pub struct ImpactRow {
    pub object: ImpactObject,
    pub dependency_ids: Vec<String>,
    pub reasons: Vec<ImpactReason>,
    pub evidence_ids: Vec<EvidenceId>,
    pub snapshot: Option<DependencySnapshot>,
}

pub struct ImpactData {
    pub generation: u64,
    pub snapshot_sha256: String,
    pub counts: ImpactCounts,
    pub items: Vec<ImpactRow>,
    pub context: FreshnessContext,
    pub has_more: bool,
}

pub fn execute(
    workspace: &Workspace,
    store: &mut dyn ImpactStore,
    input: &ImpactInput,
) -> AppResult<ImpactReport> {
    if !(1..=100).contains(&input.limit) {
        return Err(AppError::new(
            ErrorType::Validation,
            "INVALID_PAGE_LIMIT",
            "The page limit must be between 1 and 100.",
        )
        .with_param("limit"));
    }
    let fingerprint = cursor::fingerprint(workspace, input)?;
    let position = input
        .cursor
        .as_deref()
        .map(|value| cursor::decode(value, &fingerprint))
        .transpose()?;
    let status = super::status::get(
        workspace,
        store,
        &super::status::StatusInput {
            no_sync: input.no_sync,
        },
    )?;
    let query = ImpactQuery {
        source_id: input.source_id.clone(),
        revision: input.revision.clone(),
        kind: input.kind,
        limit: input.limit,
        position,
    };
    let data = store.source_impact(&query)?;
    let index_complete = status.sync_skipped.is_none()
        && !status.recovery_required
        && status.projection.generation == data.generation
        && !data.snapshot_sha256.is_empty();
    report(input, data, index_complete, false, &fingerprint)
}

pub(super) fn preview(
    workspace: &Workspace,
    snapshot: &CanonicalSnapshot,
    source_id: &SourceId,
    backend: &dyn ImpactPreviewBackend,
) -> AppResult<ImpactReport> {
    let input = ImpactInput {
        source_id: source_id.clone(),
        revision: None,
        kind: None,
        limit: default_limit(),
        cursor: None,
        no_sync: false,
    };
    let query = ImpactQuery {
        source_id: source_id.clone(),
        revision: None,
        kind: None,
        limit: input.limit,
        position: None,
    };
    let data = backend.preview(snapshot, &query)?;
    report(
        &input,
        data,
        true,
        true,
        &cursor::fingerprint(workspace, &input)?,
    )
}

fn report(
    input: &ImpactInput,
    mut data: ImpactData,
    index_complete: bool,
    preview: bool,
    fingerprint: &str,
) -> AppResult<ImpactReport> {
    data.context.index_complete = index_complete;
    let next_cursor = if data.has_more {
        data.items
            .last()
            .map(|item| {
                cursor::encode(
                    fingerprint,
                    ImpactPosition {
                        generation: data.generation,
                        snapshot_sha256: data.snapshot_sha256.clone(),
                        after: item.object.clone(),
                    },
                )
            })
            .transpose()?
    } else {
        None
    };
    let items = data
        .items
        .into_iter()
        .map(|row| {
            let freshness = if matches!(row.object, ImpactObject::Synthesis(_)) {
                synthesis_freshness(&row.evidence_ids, row.snapshot.as_ref(), &data.context)
            } else {
                assertion_freshness(&row.evidence_ids, &data.context)
            };
            ImpactItem {
                object: row.object,
                dependency_ids: row.dependency_ids,
                reasons: row.reasons,
                freshness,
            }
        })
        .collect();
    Ok(ImpactReport {
        preview,
        source_id: input.source_id.clone(),
        revision: input.revision.clone(),
        generation: data.generation,
        index_complete,
        counts: data.counts,
        items,
        next_cursor,
    })
}
