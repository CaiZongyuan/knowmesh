use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    markdown::markdown_options,
    node::NodeDocument,
    schema::Schema,
    source::SourceLibrary,
    synthesis::SynthesisDocument,
    transaction::{
        TransactionState, checked_path, file_hash, io_error, pending, recovery_required,
    },
    workspace::{Workspace, confined_existing_path, read_bounded},
};
use crate::{
    domain::{
        Claim, Evidence, EvidenceId, EvidenceStatus, LifecycleStatus, NodeId, NodeMetadata,
        Relation, SourceManifest, SynthesisMetadata, Timestamp, WorkspaceId, knowledge_error,
        normalize_name, sha256,
    },
    error::{AppError, AppResult, ErrorType},
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FileManifest {
    pub path: PathBuf,
    pub kind: String,
    pub public_id: Option<String>,
    pub byte_size: u64,
    pub mtime_ns: u64,
    pub sha256: String,
    pub format_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SourceProjection {
    pub manifest: SourceManifest,
    pub manifest_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NodeProjection {
    #[serde(flatten)]
    pub metadata: NodeMetadata,
    pub summary: String,
    pub canonical_path: PathBuf,
    pub content_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClaimProjection {
    pub claim: Claim,
    pub canonical_path: PathBuf,
    pub canonical_order: usize,
    pub semantic_sha256: String,
    pub normalized_hash: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RelationProjection {
    pub relation: Relation,
    pub canonical_path: PathBuf,
    pub canonical_order: usize,
    pub semantic_sha256: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EvidenceProjection {
    pub evidence: Evidence,
    pub canonical_path: PathBuf,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SynthesisProjection {
    pub metadata: SynthesisMetadata,
    pub body_markdown: String,
    pub canonical_path: PathBuf,
    pub content_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MentionProjection {
    pub id: String,
    pub source_node_id: NodeId,
    pub target_node_id: NodeId,
    pub surface: String,
    pub byte_start: usize,
    pub byte_end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotWarning {
    pub code: String,
    pub message: String,
    pub path: PathBuf,
}

#[derive(Debug)]
pub struct CanonicalSnapshot {
    pub workspace_id: WorkspaceId,
    pub schema_hash: String,
    pub content_sha256: String,
    pub files: Vec<FileManifest>,
    pub sources: Vec<SourceProjection>,
    pub nodes: Vec<NodeProjection>,
    pub claims: Vec<ClaimProjection>,
    pub relations: Vec<RelationProjection>,
    pub evidence: Vec<EvidenceProjection>,
    pub syntheses: Vec<SynthesisProjection>,
    pub mentions: Vec<MentionProjection>,
    pub warnings: Vec<SnapshotWarning>,
    schema: Schema,
}

impl CanonicalSnapshot {
    pub fn scan(workspace: &Workspace) -> AppResult<Self> {
        Self::scan_inner(workspace, None)
    }

    pub fn scan_committed(workspace: &Workspace, transaction_id: &str) -> AppResult<Self> {
        Self::scan_inner(workspace, Some(transaction_id))
    }

    fn scan_inner(workspace: &Workspace, transaction_id: Option<&str>) -> AppResult<Self> {
        check_recovery(workspace, transaction_id)?;
        check_workspace(workspace)?;
        let schema = Schema::load(workspace)?;
        let mut snapshot = Self {
            workspace_id: workspace.config.workspace.id.clone(),
            schema_hash: schema.hash.clone(),
            content_sha256: String::new(),
            files: vec![],
            sources: vec![],
            nodes: vec![],
            claims: vec![],
            relations: vec![],
            evidence: vec![],
            syntheses: vec![],
            mentions: vec![],
            warnings: vec![],
            schema,
        };
        snapshot.files.push(file_record(
            workspace,
            Path::new("knowmesh.yaml"),
            "workspace",
            Some(workspace.config.workspace.id.to_string()),
        )?);
        for reference in &workspace.config.schema.packs {
            if reference.starts_with("builtin:") {
                continue;
            }
            let path = confined_existing_path(&workspace.root, Path::new(reference))?;
            snapshot.files.push(file_record(
                workspace,
                path.strip_prefix(&workspace.root).map_err(|_| {
                    snapshot_error(
                        "PATH_OUTSIDE_WORKSPACE",
                        "Schema path escapes the workspace.",
                    )
                })?,
                "schema",
                None,
            )?);
        }
        if let Some(reference) = &workspace.config.workspace.purpose {
            let path = confined_existing_path(&workspace.root, Path::new(reference))?;
            snapshot.files.push(file_record(
                workspace,
                path.strip_prefix(&workspace.root).map_err(|_| {
                    snapshot_error(
                        "PATH_OUTSIDE_WORKSPACE",
                        "Purpose path escapes the workspace.",
                    )
                })?,
                "purpose",
                None,
            )?);
        }
        for source in SourceLibrary::new(workspace).list(true)? {
            let file = file_record(
                workspace,
                &source.path,
                "source",
                Some(source.manifest.id.to_string()),
            )?;
            if file.sha256 != sha256(source.original.as_bytes()) {
                return Err(file_changed());
            }
            snapshot.files.push(file);
            if source.manifest.storage != crate::domain::StorageMode::Referenced {
                for revision in &source.manifest.revisions {
                    let path = source
                        .path
                        .parent()
                        .ok_or_else(file_changed)?
                        .join(&revision.path);
                    let file = file_record(
                        workspace,
                        &path,
                        "source_blob",
                        Some(revision.id.to_string()),
                    )?;
                    if file.sha256 != revision.sha256 || file.byte_size != revision.byte_size {
                        return Err(snapshot_error(
                            "SOURCE_REVISION_CHANGED",
                            "A managed revision does not match its immutable content hash.",
                        ));
                    }
                    snapshot.files.push(file);
                }
            }
            snapshot.sources.push(SourceProjection {
                manifest: source.manifest,
                manifest_path: source.path,
            });
        }
        let mut links = Vec::new();
        let mut evidence = BTreeMap::new();
        for path in markdown_files(workspace, "knowledge/nodes")? {
            let bytes = read_bounded(&checked_path(&workspace.root, &path)?, 8 * 1024 * 1024)?;
            let text = std::str::from_utf8(&bytes).map_err(|_| {
                snapshot_error(
                    "INVALID_DOCUMENT_ENCODING",
                    "Canonical Markdown must use UTF-8.",
                )
            })?;
            let document = NodeDocument::parse(text)?;
            let file = file_record(
                workspace,
                &path,
                "node",
                Some(document.metadata.id.to_string()),
            )?;
            if file.sha256 != sha256(&bytes) {
                return Err(file_changed());
            }
            let node_id = document.metadata.id.clone();
            for link in document.links() {
                links.push((node_id.clone(), path.clone(), link));
            }
            snapshot.nodes.push(NodeProjection {
                metadata: document.metadata.clone(),
                summary: summary(document.body()),
                canonical_path: path.clone(),
                content_sha256: file.sha256.clone(),
            });
            for (ordinal, assertion) in document.claims.into_iter().enumerate() {
                collect_evidence(
                    &mut evidence,
                    &assertion.evidence,
                    &path,
                    document.metadata.created_at,
                )?;
                snapshot.claims.push(ClaimProjection {
                    semantic_sha256: assertion.semantic_hash(&node_id)?,
                    normalized_hash: assertion.normalized_hash()?,
                    claim: Claim {
                        subject_node_id: node_id.clone(),
                        assertion,
                    },
                    canonical_path: path.clone(),
                    canonical_order: ordinal,
                    created_at: document.metadata.created_at,
                    updated_at: document.metadata.updated_at,
                });
            }
            for (ordinal, assertion) in document.relations.into_iter().enumerate() {
                collect_evidence(
                    &mut evidence,
                    &assertion.evidence,
                    &path,
                    document.metadata.created_at,
                )?;
                snapshot.relations.push(RelationProjection {
                    semantic_sha256: assertion.semantic_hash(&node_id)?,
                    relation: Relation {
                        source_node_id: node_id.clone(),
                        assertion,
                    },
                    canonical_path: path.clone(),
                    canonical_order: ordinal,
                    created_at: document.metadata.created_at,
                    updated_at: document.metadata.updated_at,
                });
            }
            snapshot.files.push(file);
        }
        snapshot.evidence = evidence.into_values().collect();
        let available: BTreeSet<_> = snapshot
            .evidence
            .iter()
            .map(|e| e.evidence.id.clone())
            .collect();
        for path in markdown_files(workspace, "knowledge/syntheses")? {
            let bytes = read_bounded(&checked_path(&workspace.root, &path)?, 8 * 1024 * 1024)?;
            let text = std::str::from_utf8(&bytes).map_err(|_| {
                snapshot_error(
                    "INVALID_DOCUMENT_ENCODING",
                    "Canonical Markdown must use UTF-8.",
                )
            })?;
            let document = SynthesisDocument::parse(text)?;
            document.validate_citations(&available)?;
            let file = file_record(
                workspace,
                &path,
                "synthesis",
                Some(document.metadata.id.to_string()),
            )?;
            if file.sha256 != sha256(&bytes) {
                return Err(file_changed());
            }
            snapshot.syntheses.push(SynthesisProjection {
                metadata: document.metadata,
                body_markdown: document.body,
                canonical_path: path,
                content_sha256: file.sha256.clone(),
            });
            snapshot.files.push(file);
        }
        let mut names: BTreeMap<String, BTreeSet<NodeId>> = BTreeMap::new();
        let ids: BTreeSet<_> = snapshot
            .nodes
            .iter()
            .map(|n| n.metadata.id.clone())
            .collect();
        for node in &snapshot.nodes {
            for name in std::iter::once(&node.metadata.name).chain(&node.metadata.aliases) {
                names
                    .entry(normalize_name(name))
                    .or_default()
                    .insert(node.metadata.id.clone());
            }
        }
        for (source, path, link) in links {
            let matches = if let Ok(id) = link.target.parse::<NodeId>() {
                if ids.contains(&id) {
                    BTreeSet::from([id])
                } else {
                    BTreeSet::new()
                }
            } else {
                names
                    .get(&normalize_name(&link.target))
                    .cloned()
                    .unwrap_or_default()
            };
            if matches.len() == 1 {
                let target = matches.into_iter().next().ok_or_else(file_changed)?;
                let id = sha256(
                    format!("{source}:{target}:{}:{}", link.byte_start, link.byte_end).as_bytes(),
                );
                snapshot.mentions.push(MentionProjection {
                    id,
                    source_node_id: source,
                    target_node_id: target,
                    surface: link.display,
                    byte_start: link.byte_start,
                    byte_end: link.byte_end,
                });
            } else {
                snapshot.warnings.push(SnapshotWarning {
                    code: if matches.is_empty() {
                        "UNRESOLVED_NODE_LINK"
                    } else {
                        "AMBIGUOUS_NODE_LINK"
                    }
                    .into(),
                    message: "A wiki link could not be resolved to exactly one node.".into(),
                    path,
                });
            }
        }
        snapshot
            .sources
            .sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        snapshot
            .nodes
            .sort_by(|a, b| a.metadata.id.cmp(&b.metadata.id));
        snapshot
            .claims
            .sort_by(|a, b| a.claim.assertion.id.cmp(&b.claim.assertion.id));
        snapshot
            .relations
            .sort_by(|a, b| a.relation.assertion.id.cmp(&b.relation.assertion.id));
        snapshot
            .syntheses
            .sort_by(|a, b| a.metadata.id.cmp(&b.metadata.id));
        snapshot.mentions.sort_by(|a, b| a.id.cmp(&b.id));
        snapshot.files.sort_by(|a, b| a.path.cmp(&b.path));
        snapshot.files.dedup_by(|a, b| a.path == b.path);
        snapshot.content_sha256 = snapshot.digest()?;
        snapshot.validate()?;
        check_workspace(workspace)?;
        if Schema::load(workspace)?.hash != snapshot.schema_hash {
            return Err(file_changed());
        }
        for file in &snapshot.files {
            if file_hash(&checked_path(&workspace.root, &file.path)?)?.as_deref()
                != Some(&file.sha256)
            {
                return Err(file_changed());
            }
        }
        check_recovery(workspace, transaction_id)?;
        Ok(snapshot)
    }

    fn digest(&self) -> AppResult<String> {
        // File mtimes are scan hints; changing them alone must not advance generation.
        let files: Vec<_> = self
            .files
            .iter()
            .map(|file| {
                (
                    &file.path,
                    &file.kind,
                    &file.public_id,
                    file.byte_size,
                    &file.sha256,
                    file.format_version,
                )
            })
            .collect();
        Ok(sha256(
            &serde_json::to_vec(&(
                &self.workspace_id,
                &self.schema_hash,
                files,
                &self.sources,
                &self.nodes,
                &self.claims,
                &self.relations,
                &self.evidence,
                &self.syntheses,
                &self.mentions,
            ))
            .map_err(|_| file_changed())?,
        ))
    }

    pub fn validate(&self) -> AppResult<()> {
        if self.digest()? != self.content_sha256 || self.schema_hash != self.schema.hash {
            return Err(snapshot_error(
                "SNAPSHOT_DIGEST_MISMATCH",
                "The projection payload changed after canonical parsing.",
            ));
        }
        let mut sources = BTreeSet::new();
        let mut revisions = BTreeSet::new();
        for source in &self.sources {
            source.manifest.validate()?;
            if !sources.insert(&source.manifest.id) {
                return Err(snapshot_error(
                    "DUPLICATE_SOURCE_ID",
                    "Source IDs must be unique across the workspace.",
                ));
            }
            for revision in &source.manifest.revisions {
                if !revisions.insert(&revision.id) {
                    return Err(snapshot_error(
                        "DUPLICATE_SOURCE_REVISION",
                        "Revision IDs must be unique across the workspace.",
                    ));
                }
            }
        }
        let mut nodes = BTreeMap::new();
        for node in &self.nodes {
            node.metadata.validate()?;
            if nodes.insert(&node.metadata.id, &node.metadata).is_some() {
                return Err(snapshot_error(
                    "DUPLICATE_NODE_ID",
                    "Node IDs must be unique across the workspace.",
                ));
            }
            if !self
                .schema
                .packs
                .iter()
                .any(|pack| pack.key() == node.metadata.schema)
            {
                return Err(snapshot_error(
                    "SCHEMA_PACK_NOT_FOUND",
                    "A node refers to an inactive Schema Pack version.",
                ));
            }
            self.schema
                .validate_properties(&node.metadata.node_type, &node.metadata.properties)?;
        }
        for source in &self.sources {
            for node in &source.manifest.represented_nodes {
                if !nodes.contains_key(node) {
                    return Err(node_missing());
                }
            }
        }
        let mut evidence = BTreeMap::new();
        for item in &self.evidence {
            item.evidence.validate()?;
            if !revisions.contains(&item.evidence.source_revision_id) {
                return Err(snapshot_error(
                    "SOURCE_REVISION_NOT_FOUND",
                    "Evidence refers to a revision absent from source manifests.",
                ));
            }
            if evidence.insert(&item.evidence.id, &item.evidence).is_some() {
                return Err(snapshot_error(
                    "DUPLICATE_EVIDENCE_ID",
                    "Projected evidence identities must be unique.",
                ));
            }
        }
        let mut assertion_ids = BTreeSet::new();
        let mut active_claims = BTreeSet::new();
        for item in &self.claims {
            let claim = &item.claim;
            claim.assertion.validate()?;
            if !assertion_ids.insert(claim.assertion.id.as_str()) {
                return Err(assertion_duplicate());
            }
            if !nodes.contains_key(&claim.subject_node_id) {
                return Err(node_missing());
            }
            if claim.assertion.lifecycle_status == LifecycleStatus::Active
                && !active_claims
                    .insert((&claim.subject_node_id, claim.assertion.normalized_hash()?))
            {
                return Err(snapshot_error(
                    "DUPLICATE_ACTIVE_CLAIM",
                    "Identical active claims must merge their evidence.",
                ));
            }
            check_evidence(&claim.assertion.evidence, &evidence)?;
        }
        for item in &self.relations {
            let relation = &item.relation;
            relation.assertion.validate()?;
            if !assertion_ids.insert(relation.assertion.id.as_str()) {
                return Err(assertion_duplicate());
            }
            let source = nodes
                .get(&relation.source_node_id)
                .ok_or_else(node_missing)?;
            let target = nodes
                .get(&relation.assertion.target_node_id)
                .ok_or_else(node_missing)?;
            self.schema.validate_relation(
                &relation.assertion.predicate,
                &source.node_type,
                &target.node_type,
                !relation.assertion.evidence.is_empty()
                    || relation.assertion.evidence_status == EvidenceStatus::Unreviewed,
            )?;
            if self.schema.predicates[&relation.assertion.predicate].directed
                != relation.assertion.directed
            {
                return Err(snapshot_error(
                    "RELATION_DIRECTION_MISMATCH",
                    "Relation direction must match the schema definition.",
                ));
            }
            check_evidence(&relation.assertion.evidence, &evidence)?;
        }
        let mut synthesis_ids = BTreeSet::new();
        for item in &self.syntheses {
            item.metadata.validate()?;
            if !synthesis_ids.insert(&item.metadata.id) {
                return Err(snapshot_error(
                    "DUPLICATE_SYNTHESIS_ID",
                    "Synthesis IDs must be unique.",
                ));
            }
            for id in &item.metadata.evidence_ids {
                if !evidence.contains_key(id) {
                    return Err(snapshot_error(
                        "EVIDENCE_NOT_FOUND",
                        "Synthesis references missing evidence.",
                    ));
                }
            }
            for id in &item.metadata.related_nodes {
                if !nodes.contains_key(id) {
                    return Err(node_missing());
                }
            }
        }
        Ok(())
    }
}

fn collect_evidence(
    all: &mut BTreeMap<EvidenceId, EvidenceProjection>,
    items: &[Evidence],
    path: &Path,
    created_at: Timestamp,
) -> AppResult<()> {
    for item in items {
        if let Some(existing) = all.get(&item.id) {
            if existing.evidence != *item {
                return Err(snapshot_error(
                    "EVIDENCE_ID_CONFLICT",
                    "A shared Evidence ID contains inconsistent canonical content.",
                ));
            }
        } else {
            all.insert(
                item.id.clone(),
                EvidenceProjection {
                    evidence: item.clone(),
                    canonical_path: path.to_owned(),
                    created_at,
                },
            );
        }
    }
    Ok(())
}

fn check_evidence(
    items: &[Evidence],
    available: &BTreeMap<&EvidenceId, &Evidence>,
) -> AppResult<()> {
    for item in items {
        if available.get(&item.id).copied() != Some(item) {
            return Err(snapshot_error(
                "EVIDENCE_ID_CONFLICT",
                "An assertion's evidence must match the canonical evidence projection.",
            ));
        }
    }
    Ok(())
}

fn markdown_files(workspace: &Workspace, directory: &str) -> AppResult<Vec<PathBuf>> {
    let mut directories = vec![PathBuf::from(directory)];
    let mut files = Vec::new();
    while let Some(relative) = directories.pop() {
        if relative.components().count() > 64 || files.len() > 100_000 {
            return Err(snapshot_error(
                "WORKSPACE_LIMIT_EXCEEDED",
                "The workspace exceeds canonical traversal limits.",
            ));
        }
        let absolute = checked_path(&workspace.root, &relative)?;
        for entry in fs::read_dir(absolute).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let path = relative.join(entry.file_name());
            checked_path(&workspace.root, &path)?;
            let kind = entry.file_type().map_err(io_error)?;
            if kind.is_dir() {
                directories.push(path);
            } else if kind.is_file()
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
            {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn file_record(
    workspace: &Workspace,
    relative: &Path,
    kind: &str,
    public_id: Option<String>,
) -> AppResult<FileManifest> {
    let path = checked_path(&workspace.root, relative)?;
    let metadata = fs::metadata(&path).map_err(io_error)?;
    let mtime_ns = metadata
        .modified()
        .map_err(io_error)?
        .duration_since(UNIX_EPOCH)
        .map_err(|_| file_changed())?
        .as_nanos()
        .try_into()
        .map_err(|_| file_changed())?;
    Ok(FileManifest {
        path: relative.to_owned(),
        kind: kind.into(),
        public_id,
        byte_size: metadata.len(),
        mtime_ns,
        sha256: file_hash(&path)?.ok_or_else(file_changed)?,
        format_version: 1,
    })
}

fn summary(body: &str) -> String {
    let mut in_heading = false;
    let mut heading = String::new();
    let mut selected = false;
    let mut output = String::new();
    for event in Parser::new_ext(body, markdown_options()) {
        match event {
            Event::Start(Tag::Heading {
                level: HeadingLevel::H1 | HeadingLevel::H2,
                ..
            }) => {
                if selected {
                    break;
                }
                in_heading = true;
                heading.clear();
            }
            Event::End(TagEnd::Heading(_)) if in_heading => {
                selected = heading.eq_ignore_ascii_case("Summary");
                in_heading = false;
            }
            Event::Text(text) | Event::Code(text) if in_heading => heading.push_str(&text),
            Event::Text(text) | Event::Code(text) if selected => output.push_str(&text),
            Event::SoftBreak | Event::HardBreak | Event::End(TagEnd::Paragraph) if selected => {
                output.push('\n')
            }
            Event::Html(_) if selected => break,
            _ => {}
        }
    }
    output.trim().to_owned()
}

fn check_recovery(workspace: &Workspace, transaction_id: Option<&str>) -> AppResult<()> {
    let pending = pending(&workspace.root)?;
    if pending.iter().any(|tx| {
        Some(tx.id.as_str()) != transaction_id || tx.state != TransactionState::CanonicalCommitted
    }) {
        return Err(recovery_required());
    }
    Ok(())
}

fn check_workspace(workspace: &Workspace) -> AppResult<()> {
    let current = Workspace::load(&workspace.root)?;
    if serde_json::to_value(&current.config).map_err(|_| file_changed())?
        != serde_json::to_value(&workspace.config).map_err(|_| file_changed())?
        || current.purpose.as_ref().map(|p| &p.sha256)
            != workspace.purpose.as_ref().map(|p| &p.sha256)
    {
        return Err(file_changed());
    }
    Ok(())
}
fn snapshot_error(code: &str, message: &str) -> AppError {
    knowledge_error(code, message)
}
fn node_missing() -> AppError {
    snapshot_error(
        "NODE_NOT_FOUND",
        "A canonical reference points to a node absent from the workspace.",
    )
}
fn assertion_duplicate() -> AppError {
    snapshot_error(
        "DUPLICATE_ASSERTION_ID",
        "Assertion IDs must be unique across the workspace.",
    )
}
fn file_changed() -> AppError {
    AppError::new(
        ErrorType::Conflict,
        "CANONICAL_FILE_CONFLICT",
        "A canonical file changed during the scan.",
    )
    .retryable(true)
}
