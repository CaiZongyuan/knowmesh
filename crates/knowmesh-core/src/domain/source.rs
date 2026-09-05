use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{NodeId, SourceId, SourceRevisionId, Timestamp};
use crate::error::{AppError, AppResult, ErrorType};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum StorageMode {
    Managed,
    Referenced,
    SnapshotUrl,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Author {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orcid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceManifest {
    pub version: u32,
    pub id: SourceId,
    pub slug: String,
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub authors: Vec<Author>,
    #[serde(default)]
    pub identifiers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub storage: StorageMode,
    pub current_revision_id: SourceRevisionId,
    #[serde(default)]
    pub represented_nodes: Vec<NodeId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed_at: Option<Timestamp>,
    pub revisions: Vec<SourceRevision>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceRevision {
    pub id: SourceRevisionId,
    pub path: String,
    pub mime_type: String,
    pub sha256: String,
    pub byte_size: u64,
    pub captured_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl SourceManifest {
    pub fn validate(&self) -> AppResult<()> {
        if self.version != 1 {
            return Err(source_error(
                "UNSUPPORTED_SOURCE_VERSION",
                "Only source manifest version 1 is supported.",
            ));
        }
        if self.slug.is_empty()
            || self.slug.len() > 80
            || !self
                .slug
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            || self.title.trim().is_empty()
            || self.title.len() > 2048
            || self.kind.is_empty()
            || self.kind.len() > 64
            || !self
                .kind
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c == b'-' || c == b'_')
            || self.revisions.is_empty()
            || self.revisions.len() > 10_000
            || self.updated_at < self.created_at
        {
            return Err(source_error(
                "INVALID_SOURCE_MANIFEST",
                "Source metadata is invalid.",
            ));
        }
        let mut ids = BTreeSet::new();
        let mut hashes = BTreeSet::new();
        for revision in &self.revisions {
            if !ids.insert(&revision.id) || !hashes.insert(&revision.sha256) {
                return Err(source_error(
                    "DUPLICATE_SOURCE_REVISION",
                    "Revision IDs and content hashes must be unique within a source.",
                ));
            }
            if !valid_sha256(&revision.sha256) {
                return Err(source_error(
                    "INVALID_SOURCE_HASH",
                    "Revision hashes must be lowercase SHA-256 hex.",
                ));
            }
            let path = Path::new(&revision.path);
            if self.storage == StorageMode::Referenced {
                if !path.is_absolute()
                    || path
                        .components()
                        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
                {
                    return Err(source_error(
                        "INVALID_REVISION_PATH",
                        "Referenced revisions require a normalized absolute path.",
                    ));
                }
            } else {
                let expected_prefix = format!("revisions/{}/original.", revision.id);
                if !revision.path.starts_with(&expected_prefix)
                    || path.is_absolute()
                    || path
                        .components()
                        .any(|c| !matches!(c, Component::Normal(_)))
                    || !["md", "txt", "html", "pdf"]
                        .contains(&revision.path[expected_prefix.len()..].as_ref())
                {
                    return Err(source_error(
                        "INVALID_REVISION_PATH",
                        "Managed revision paths must identify their immutable original snapshot.",
                    ));
                }
            }
            if ![
                "text/markdown",
                "text/plain",
                "text/html",
                "application/pdf",
            ]
            .contains(&revision.mime_type.as_str())
            {
                return Err(source_error(
                    "SOURCE_TYPE_UNSUPPORTED",
                    "This revision MIME type is not supported.",
                ));
            }
            if self.storage == StorageMode::SnapshotUrl && revision.url.is_none() {
                return Err(source_error(
                    "SOURCE_URL_REQUIRED",
                    "URL snapshots must retain the fetched resource URL.",
                ));
            }
            if let Some(url) = &revision.url {
                validate_source_url(url)?;
            }
        }
        if !ids.contains(&self.current_revision_id) {
            return Err(source_error(
                "SOURCE_HEAD_MISSING",
                "The current revision must belong to the source.",
            ));
        }
        Ok(())
    }

    pub fn validate_update(&self, previous: &Self) -> AppResult<()> {
        self.validate()?;
        if self.id != previous.id
            || self.storage != previous.storage
            || self.created_at != previous.created_at
            || !self.revisions.starts_with(&previous.revisions)
        {
            return Err(source_error(
                "IMMUTABLE_REVISION_CHANGED",
                "Source identity, storage mode, and historical revisions cannot be rewritten.",
            ));
        }
        Ok(())
    }
}

pub(crate) fn validate_source_url(value: &str) -> AppResult<url::Url> {
    let url = url::Url::parse(value)
        .map_err(|_| source_error("INVALID_SOURCE_URL", "Expected an absolute HTTP(S) URL."))?;
    if !["http", "https"].contains(&url.scheme())
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(source_error(
            "INVALID_SOURCE_URL",
            "Source URLs require HTTP(S), a host, and no embedded credentials.",
        ));
    }
    Ok(url)
}

pub(crate) fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
pub(crate) fn source_error(code: &str, message: &str) -> AppError {
    AppError::new(ErrorType::Validation, code, message)
}
