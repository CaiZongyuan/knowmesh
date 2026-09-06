use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use super::{
    CanonicalSnapshot, FileManifest, Schema, SourceProjection, SynthesisProjection,
    canonical_paths, check_recovery, check_workspace, file_changed, snapshot_error,
};
use crate::{
    canonical::{
        node::NodeDocument,
        source::SourceFile,
        synthesis::SynthesisDocument,
        transaction::{checked_path, file_hash, path_key, validate_canonical_path},
        workspace::{Workspace, read_bounded},
    },
    domain::sha256,
    error::{AppError, AppResult},
};

#[derive(Debug)]
pub struct CanonicalPreview {
    snapshot: CanonicalSnapshot,
}

impl CanonicalPreview {
    pub fn content_sha256(&self) -> &str {
        &self.snapshot.content_sha256
    }
    pub fn files(&self) -> &[FileManifest] {
        &self.snapshot.files
    }
    pub fn nodes(&self) -> &[super::NodeProjection] {
        &self.snapshot.nodes
    }
    pub fn claims(&self) -> &[super::ClaimProjection] {
        &self.snapshot.claims
    }
    pub fn relations(&self) -> &[super::RelationProjection] {
        &self.snapshot.relations
    }
    pub fn evidence(&self) -> &[super::EvidenceProjection] {
        &self.snapshot.evidence
    }
    pub fn sources(&self) -> &[SourceProjection] {
        &self.snapshot.sources
    }
    pub fn syntheses(&self) -> &[SynthesisProjection] {
        &self.snapshot.syntheses
    }
    pub fn mentions(&self) -> &[super::MentionProjection] {
        &self.snapshot.mentions
    }
    pub fn warnings(&self) -> &[super::SnapshotWarning] {
        &self.snapshot.warnings
    }
    pub fn validate(&self) -> AppResult<()> {
        self.snapshot.validate()
    }
}

impl CanonicalSnapshot {
    pub fn preview_documents(
        &self,
        workspace: &Workspace,
        changes: &BTreeMap<PathBuf, Vec<u8>>,
    ) -> AppResult<CanonicalPreview> {
        self.validate()?;
        self.check_preview_base(workspace)?;
        if changes.len() > 10_000
            || changes
                .values()
                .fold(0usize, |sum, bytes| sum.saturating_add(bytes.len()))
                > 64 * 1024 * 1024
        {
            return Err(snapshot_error(
                "CANONICAL_PREVIEW_LIMIT",
                "A preview supports at most 10000 documents and 64 MiB of replacement bytes.",
            ));
        }
        let base: BTreeMap<_, _> = self
            .files
            .iter()
            .map(|file| (file.path.clone(), file))
            .collect();
        let base_keys: BTreeMap<_, _> = self
            .files
            .iter()
            .map(|file| (path_key(&file.path), &file.path))
            .collect();
        let base_nodes: BTreeMap<_, _> = self
            .nodes
            .iter()
            .map(|node| (&node.canonical_path, node))
            .collect();
        let base_syntheses: BTreeMap<_, _> = self
            .syntheses
            .iter()
            .map(|synthesis| (&synthesis.canonical_path, synthesis))
            .collect();
        let mut keys = BTreeSet::new();
        let mut kinds = BTreeMap::new();
        for (path, bytes) in changes {
            validate_canonical_path(path).map_err(|_| forbidden())?;
            checked_path(&workspace.root, path)?;
            let key = path_key(path);
            if !keys.insert(key.clone())
                || base_keys
                    .get(&key)
                    .is_some_and(|existing| *existing != path)
            {
                return Err(snapshot_error(
                    "CANONICAL_PREVIEW_PATH_CONFLICT",
                    "Preview paths must be unique across portable path aliases.",
                ));
            }
            let kind = if let Some(file) = base.get(path) {
                match file.kind.as_str() {
                    "node" | "synthesis" | "source" => file.kind.as_str(),
                    _ => return Err(forbidden()),
                }
            } else {
                if file_hash(&checked_path(&workspace.root, path)?)?.is_some() {
                    return Err(file_changed());
                }
                if path
                    .extension()
                    .is_none_or(|ext| !ext.eq_ignore_ascii_case("md"))
                {
                    return Err(forbidden());
                }
                if path.starts_with("knowledge/nodes") {
                    "node"
                } else if path.starts_with("knowledge/syntheses") {
                    "synthesis"
                } else {
                    return Err(forbidden());
                }
            };
            let limit = if kind == "source" { 16 } else { 8 } * 1024 * 1024;
            if bytes.len() > limit {
                return Err(snapshot_error(
                    "DOCUMENT_TOO_LARGE",
                    "The preview document exceeds its format size limit.",
                ));
            }
            kinds.insert(path.clone(), kind.to_owned());
        }
        let mut projected = self.clone();
        projected.nodes.clear();
        projected.claims.clear();
        projected.relations.clear();
        projected.evidence.clear();
        projected.syntheses.clear();
        projected
            .files
            .retain(|file| !matches!(file.kind.as_str(), "node" | "synthesis"));
        let source_files: BTreeMap<_, _> = projected
            .files
            .iter()
            .enumerate()
            .map(|(index, file)| (file.path.clone(), index))
            .collect();
        for source in &mut projected.sources {
            if let Some(bytes) = changes.get(&source.manifest_path) {
                let replacement = SourceFile::parse(source.manifest_path.clone(), bytes)?;
                replacement.manifest.validate_update(&source.manifest)?;
                if replacement.manifest.revisions != source.manifest.revisions
                    || replacement.manifest.current_revision_id
                        != source.manifest.current_revision_id
                    || replacement.manifest.removed_at != source.manifest.removed_at
                {
                    return Err(snapshot_error(
                        "SOURCE_REVISION_CHANGED",
                        "Proposal previews may edit source metadata, not revision or removal state.",
                    ));
                }
                let file = &mut projected.files[*source_files
                    .get(&source.manifest_path)
                    .ok_or_else(file_changed)?];
                file.sha256 = sha256(bytes);
                file.byte_size = bytes.len() as u64;
                *source = SourceProjection {
                    manifest: replacement.manifest,
                    manifest_path: source.manifest_path.clone(),
                };
            }
        }
        let mut paths: BTreeSet<_> = self
            .nodes
            .iter()
            .map(|node| node.canonical_path.clone())
            .collect();
        paths.extend(
            kinds
                .iter()
                .filter(|(_, kind)| *kind == "node")
                .map(|(path, _)| path.clone()),
        );
        let mut evidence = BTreeMap::new();
        let mut links = vec![];
        for path in paths {
            let bytes = document_bytes(workspace, &path, changes, base.get(&path).copied())?;
            let doc = NodeDocument::parse(utf8(&bytes)?)?;
            if let Some(previous) = base_nodes.get(&path)
                && (previous.metadata.id != doc.metadata.id
                    || previous.metadata.created_at != doc.metadata.created_at)
            {
                return Err(snapshot_error(
                    "NODE_IDENTITY_CHANGED",
                    "A preview cannot replace an existing Node identity or creation time.",
                ));
            }
            let file = projected_file(
                &path,
                "node",
                doc.metadata.id.to_string(),
                &bytes,
                base.get(&path).copied(),
            );
            projected.project_node(doc, file, &mut evidence, &mut links)?;
        }
        projected.evidence = evidence.into_values().collect();
        let available = projected
            .evidence
            .iter()
            .map(|item| item.evidence.id.clone())
            .collect();
        let mut paths: BTreeSet<_> = self
            .syntheses
            .iter()
            .map(|synthesis| synthesis.canonical_path.clone())
            .collect();
        paths.extend(
            kinds
                .iter()
                .filter(|(_, kind)| *kind == "synthesis")
                .map(|(path, _)| path.clone()),
        );
        for path in paths {
            let bytes = document_bytes(workspace, &path, changes, base.get(&path).copied())?;
            let doc = SynthesisDocument::parse(utf8(&bytes)?)?;
            doc.validate_citations(&available)?;
            if let Some(previous) = base_syntheses.get(&path)
                && (previous.metadata.id != doc.metadata.id
                    || previous.metadata.created_at != doc.metadata.created_at)
            {
                return Err(snapshot_error(
                    "SYNTHESIS_IDENTITY_CHANGED",
                    "A preview cannot replace an existing Synthesis identity or creation time.",
                ));
            }
            let file = projected_file(
                &path,
                "synthesis",
                doc.metadata.id.to_string(),
                &bytes,
                base.get(&path).copied(),
            );
            projected.syntheses.push(SynthesisProjection {
                metadata: doc.metadata,
                body_markdown: doc.body,
                canonical_path: path,
                content_sha256: file.sha256.clone(),
            });
            projected.files.push(file);
        }
        projected.resolve_links(links)?;
        projected.finish_projection()?;
        self.check_preview_base(workspace)?;
        Ok(CanonicalPreview {
            snapshot: projected,
        })
    }

    fn check_preview_base(&self, workspace: &Workspace) -> AppResult<()> {
        check_recovery(workspace, None)?;
        check_workspace(workspace)?;
        if self.workspace_id != workspace.config.workspace.id
            || self.schema_hash != Schema::load(workspace)?.hash
        {
            return Err(file_changed());
        }
        let indexed = self
            .files
            .iter()
            .filter(|file| file.kind != "source_blob")
            .map(|file| file.path.clone())
            .collect();
        if canonical_paths(workspace)? != indexed {
            return Err(file_changed());
        }
        for file in &self.files {
            if file_hash(&checked_path(&workspace.root, &file.path)?)?.as_deref()
                != Some(&file.sha256)
            {
                return Err(file_changed());
            }
        }
        Ok(())
    }
}

fn document_bytes(
    workspace: &Workspace,
    path: &Path,
    changes: &BTreeMap<PathBuf, Vec<u8>>,
    before: Option<&FileManifest>,
) -> AppResult<Vec<u8>> {
    if let Some(bytes) = changes.get(path) {
        return Ok(bytes.clone());
    }
    let bytes = read_bounded(&checked_path(&workspace.root, path)?, 8 * 1024 * 1024)?;
    if before.is_none_or(|file| sha256(&bytes) != file.sha256) {
        return Err(file_changed());
    }
    Ok(bytes)
}

fn projected_file(
    path: &Path,
    kind: &str,
    id: String,
    bytes: &[u8],
    before: Option<&FileManifest>,
) -> FileManifest {
    FileManifest {
        path: path.to_owned(),
        kind: kind.into(),
        public_id: Some(id),
        byte_size: bytes.len() as u64,
        mtime_ns: before.map_or(0, |file| file.mtime_ns),
        sha256: sha256(bytes),
        format_version: 1,
    }
}

fn utf8(bytes: &[u8]) -> AppResult<&str> {
    std::str::from_utf8(bytes).map_err(|_| {
        snapshot_error(
            "INVALID_DOCUMENT_ENCODING",
            "Canonical documents must use UTF-8.",
        )
    })
}

fn forbidden() -> AppError {
    snapshot_error(
        "CANONICAL_PREVIEW_PATH_FORBIDDEN",
        "Preview changes are limited to Node/Synthesis Markdown and existing source metadata.",
    )
}
