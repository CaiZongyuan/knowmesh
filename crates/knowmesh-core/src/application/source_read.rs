mod cursor;

use std::str::FromStr;

use base64::{Engine, engine::general_purpose::STANDARD};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    canonical::{snapshot::SourceProjection, source::SourceLibrary, workspace::Workspace},
    domain::{SourceId, SourceManifest, SourceRevision, SourceRevisionId, StorageMode, Timestamp},
    error::{AppError, AppResult, ErrorType},
    ports::SourceReadStore,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ContentId {
    Source(SourceId),
    Revision(SourceRevisionId),
}

impl FromStr for ContentId {
    type Err = AppError;

    fn from_str(value: &str) -> AppResult<Self> {
        if value.starts_with("rev_") {
            value.parse().map(Self::Revision)
        } else {
            value.parse().map(Self::Source)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ListInput {
    pub include_removed: bool,
    pub kind: Option<String>,
    pub tag: Option<String>,
    pub limit: u32,
    pub cursor: Option<String>,
    pub no_sync: bool,
}

impl Default for ListInput {
    fn default() -> Self {
        Self {
            include_removed: false,
            kind: None,
            tag: None,
            limit: 20,
            cursor: None,
            no_sync: false,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetInput {
    pub source_id: SourceId,
    #[serde(default)]
    pub no_sync: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContentInput {
    pub id: ContentId,
    #[serde(default)]
    pub no_sync: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SourceSummary {
    pub id: SourceId,
    pub title: String,
    pub kind: String,
    pub tags: Vec<String>,
    pub storage: StorageMode,
    pub current_revision_id: SourceRevisionId,
    pub status: String,
    pub updated_at: Timestamp,
    pub removed_at: Option<Timestamp>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListReport {
    pub generation: u64,
    pub index_complete: bool,
    pub total: u64,
    pub items: Vec<SourceSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SourceReport {
    pub generation: u64,
    pub index_complete: bool,
    pub source: SourceManifest,
}

#[derive(Debug)]
pub struct SourceContent {
    pub generation: u64,
    pub index_complete: bool,
    pub source_id: SourceId,
    pub revision: SourceRevision,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ContentEncoding {
    #[serde(rename = "utf-8")]
    Utf8,
    Base64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ContentReport {
    pub generation: u64,
    pub index_complete: bool,
    pub source_id: SourceId,
    pub revision: SourceRevision,
    pub encoding: ContentEncoding,
    pub content: String,
}

impl SourceContent {
    pub fn into_report(self) -> AppResult<ContentReport> {
        let (encoding, content) = if self.revision.mime_type.starts_with("text/") {
            (
                ContentEncoding::Utf8,
                crate::domain::decode_source_text(&self.bytes, self.revision.encoding.as_ref())
                    .map_err(|mut error| {
                        error.code = "SOURCE_CONTENT_ENCODING_INVALID".into();
                        error.param = Some("revision.encoding".into());
                        error.with_hint("Restore the original revision metadata and bytes, or import a distinct source with the correct encoding.")
                    })?.into_owned(),
            )
        } else {
            (ContentEncoding::Base64, STANDARD.encode(self.bytes))
        };
        Ok(ContentReport {
            generation: self.generation,
            index_complete: self.index_complete,
            source_id: self.source_id,
            revision: self.revision,
            encoding,
            content,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListPosition {
    pub generation: u64,
    pub snapshot_sha256: String,
    pub after: SourceId,
}

pub struct ListQuery {
    pub include_removed: bool,
    pub kind: Option<String>,
    pub tag: Option<String>,
    pub limit: u32,
    pub position: Option<ListPosition>,
}

pub struct ListData {
    pub generation: u64,
    pub snapshot_sha256: String,
    pub total: u64,
    pub items: Vec<SourceSummary>,
    pub has_more: bool,
}

pub struct SourceData {
    pub generation: u64,
    pub snapshot_sha256: String,
    pub source: SourceProjection,
}

pub fn list(
    workspace: &Workspace,
    store: &mut dyn SourceReadStore,
    input: &ListInput,
) -> AppResult<ListReport> {
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
    let data = store.source_list(&ListQuery {
        include_removed: input.include_removed,
        kind: input.kind.clone(),
        tag: input.tag.clone(),
        limit: input.limit,
        position,
    })?;
    let index_complete = complete(&status, data.generation, &data.snapshot_sha256);
    let next_cursor = if data.has_more {
        data.items
            .last()
            .map(|last| {
                cursor::encode(
                    &fingerprint,
                    ListPosition {
                        generation: data.generation,
                        snapshot_sha256: data.snapshot_sha256.clone(),
                        after: last.id.clone(),
                    },
                )
            })
            .transpose()?
    } else {
        None
    };
    Ok(ListReport {
        generation: data.generation,
        index_complete,
        total: data.total,
        items: data.items,
        next_cursor,
    })
}

pub fn get(
    workspace: &Workspace,
    store: &mut dyn SourceReadStore,
    input: &GetInput,
) -> AppResult<SourceReport> {
    let (data, index_complete) = read(
        workspace,
        store,
        &ContentId::Source(input.source_id.clone()),
        input.no_sync,
    )?;
    Ok(SourceReport {
        generation: data.generation,
        index_complete,
        source: data.source.manifest,
    })
}

pub fn content(
    workspace: &Workspace,
    store: &mut dyn SourceReadStore,
    input: &ContentInput,
) -> AppResult<SourceContent> {
    let (data, index_complete) = read(workspace, store, &input.id, input.no_sync)?;
    let manifest = &data.source.manifest;
    let revision_id = match &input.id {
        ContentId::Source(_) => &manifest.current_revision_id,
        ContentId::Revision(id) => id,
    };
    let revision = manifest
        .revisions
        .iter()
        .find(|revision| &revision.id == revision_id)
        .cloned()
        .ok_or_else(|| {
            AppError::new(
                ErrorType::NotFound,
                "SOURCE_REVISION_NOT_FOUND",
                "The revision does not belong to this source.",
            )
        })?;
    let bytes = SourceLibrary::new(workspace).content_at(
        &data.source.manifest_path,
        manifest,
        revision_id,
    )?;
    Ok(SourceContent {
        generation: data.generation,
        index_complete,
        source_id: manifest.id.clone(),
        revision,
        bytes,
    })
}

fn read(
    workspace: &Workspace,
    store: &mut dyn SourceReadStore,
    id: &ContentId,
    no_sync: bool,
) -> AppResult<(SourceData, bool)> {
    let status = super::status::get(workspace, store, &super::status::StatusInput { no_sync })?;
    let data = store.source_get(id)?;
    data.source.manifest.validate()?;
    let index_complete = complete(&status, data.generation, &data.snapshot_sha256);
    Ok((data, index_complete))
}

fn complete(status: &super::status::StatusReport, generation: u64, hash: &str) -> bool {
    status.sync_skipped.is_none()
        && !status.recovery_required
        && status.projection.generation == generation
        && !hash.is_empty()
}
