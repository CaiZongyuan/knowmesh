use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use super::{
    CanonicalSnapshot, ClaimProjection, EvidenceProjection, FileManifest, MentionProjection,
    NodeProjection, RelationProjection, SnapshotWarning, collect_evidence, file_changed,
};
use crate::{
    canonical::node::{NodeDocument, NodeLink},
    domain::{Claim, EvidenceId, NodeId, Relation, normalize_name, sha256},
    error::AppResult,
};

pub(super) type PendingLink = (NodeId, PathBuf, NodeLink);

impl CanonicalSnapshot {
    pub(super) fn project_node(
        &mut self,
        document: NodeDocument,
        file: FileManifest,
        evidence: &mut BTreeMap<EvidenceId, EvidenceProjection>,
        links: &mut Vec<PendingLink>,
    ) -> AppResult<()> {
        let path = file.path.clone();
        let node_id = document.metadata.id.clone();
        for link in document.links() {
            links.push((node_id.clone(), path.clone(), link));
        }
        self.nodes.push(NodeProjection {
            metadata: document.metadata.clone(),
            summary: document.summary(),
            canonical_path: path.clone(),
            content_sha256: file.sha256.clone(),
        });
        for (ordinal, assertion) in document.claims.into_iter().enumerate() {
            collect_evidence(
                evidence,
                &assertion.evidence,
                &path,
                document.metadata.created_at,
            )?;
            self.claims.push(ClaimProjection {
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
                evidence,
                &assertion.evidence,
                &path,
                document.metadata.created_at,
            )?;
            self.relations.push(RelationProjection {
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
        self.files.push(file);
        Ok(())
    }

    pub(super) fn resolve_links(&mut self, links: Vec<PendingLink>) -> AppResult<()> {
        self.mentions.clear();
        self.warnings.retain(|warning| {
            !matches!(
                warning.code.as_str(),
                "UNRESOLVED_NODE_LINK" | "AMBIGUOUS_NODE_LINK"
            )
        });
        let mut names: BTreeMap<String, BTreeSet<NodeId>> = BTreeMap::new();
        let ids: BTreeSet<_> = self
            .nodes
            .iter()
            .map(|node| node.metadata.id.clone())
            .collect();
        for node in &self.nodes {
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
                self.mentions.push(MentionProjection {
                    id,
                    source_node_id: source,
                    target_node_id: target,
                    surface: link.display,
                    byte_start: link.byte_start,
                    byte_end: link.byte_end,
                });
            } else {
                self.warnings.push(SnapshotWarning {
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
        Ok(())
    }

    pub(super) fn finish_projection(&mut self) -> AppResult<()> {
        self.sources
            .sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        self.nodes.sort_by(|a, b| a.metadata.id.cmp(&b.metadata.id));
        self.claims
            .sort_by(|a, b| a.claim.assertion.id.cmp(&b.claim.assertion.id));
        self.relations
            .sort_by(|a, b| a.relation.assertion.id.cmp(&b.relation.assertion.id));
        self.syntheses
            .sort_by(|a, b| a.metadata.id.cmp(&b.metadata.id));
        self.mentions.sort_by(|a, b| a.id.cmp(&b.id));
        self.files.sort_by(|a, b| a.path.cmp(&b.path));
        self.files.dedup_by(|a, b| a.path == b.path);
        self.content_sha256 = self.digest()?;
        self.validated_sha256 = self.content_sha256.clone();
        self.validate()
    }
}
