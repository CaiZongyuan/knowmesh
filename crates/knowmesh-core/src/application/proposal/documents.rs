use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

mod synthesis;

use super::{error, payload::Payload};
use crate::{
    canonical::{
        node::NodeDocument,
        schema::Schema,
        snapshot::CanonicalSnapshot,
        source::SourceFile,
        synthesis::SynthesisDocument,
        transaction::{checked_path, path_key},
        workspace::{Workspace, read_bounded},
    },
    domain::{
        ClaimId, Evidence, EvidenceId, EvidenceStance, EvidenceStatus, LifecycleStatus, NodeId,
        RelationId, SourceId, Timestamp,
        proposal::{ProposalItem, ProposalKind},
    },
    error::AppResult,
};

pub(super) struct Documents {
    nodes: BTreeMap<NodeId, (PathBuf, NodeDocument)>,
    sources: BTreeMap<SourceId, SourceFile>,
    syntheses: BTreeMap<crate::domain::SynthesisId, (PathBuf, SynthesisDocument)>,
    claim_owners: BTreeMap<ClaimId, NodeId>,
    relation_owners: BTreeMap<RelationId, NodeId>,
    originals: BTreeMap<PathBuf, Vec<u8>>,
    exclusive: BTreeSet<(String, &'static str)>,
}

impl Documents {
    pub fn load(workspace: &Workspace, before: &CanonicalSnapshot) -> AppResult<Self> {
        let mut this = Self {
            nodes: BTreeMap::new(),
            sources: BTreeMap::new(),
            syntheses: BTreeMap::new(),
            claim_owners: BTreeMap::new(),
            relation_owners: BTreeMap::new(),
            originals: BTreeMap::new(),
            exclusive: BTreeSet::new(),
        };
        for node in &before.nodes {
            let bytes = read_bounded(
                &checked_path(&workspace.root, &node.canonical_path)?,
                8 * 1024 * 1024,
            )?;
            if crate::domain::sha256(&bytes) != node.content_sha256 {
                return Err(error(
                    "CANONICAL_FILE_CONFLICT",
                    "A canonical document changed while preparing the Proposal.",
                ));
            }
            let doc = NodeDocument::parse(utf8(&bytes)?)?;
            for claim in &doc.claims {
                this.claim_owners
                    .insert(claim.id.clone(), doc.metadata.id.clone());
            }
            for relation in &doc.relations {
                this.relation_owners
                    .insert(relation.id.clone(), doc.metadata.id.clone());
            }
            this.nodes
                .insert(doc.metadata.id.clone(), (node.canonical_path.clone(), doc));
            this.originals.insert(node.canonical_path.clone(), bytes);
        }
        for source in &before.sources {
            let bytes = read_bounded(
                &checked_path(&workspace.root, &source.manifest_path)?,
                16 * 1024 * 1024,
            )?;
            let file = SourceFile::parse(source.manifest_path.clone(), &bytes)?;
            if file.manifest != source.manifest {
                return Err(error(
                    "CANONICAL_FILE_CONFLICT",
                    "Source metadata changed while preparing the Proposal.",
                ));
            }
            this.sources.insert(file.manifest.id.clone(), file);
            this.originals.insert(source.manifest_path.clone(), bytes);
        }
        for synthesis in &before.syntheses {
            let bytes = read_bounded(
                &checked_path(&workspace.root, &synthesis.canonical_path)?,
                8 * 1024 * 1024,
            )?;
            if crate::domain::sha256(&bytes) != synthesis.content_sha256 {
                return Err(error(
                    "CANONICAL_FILE_CONFLICT",
                    "A Synthesis changed while preparing the Proposal.",
                ));
            }
            let doc = SynthesisDocument::parse(utf8(&bytes)?)?;
            this.syntheses.insert(
                doc.metadata.id.clone(),
                (synthesis.canonical_path.clone(), doc),
            );
            this.originals
                .insert(synthesis.canonical_path.clone(), bytes);
        }
        Ok(this)
    }

    pub fn path(&self, item: &ProposalItem, payload: &Payload) -> AppResult<PathBuf> {
        match payload {
            Payload::CreateNode(value) => Ok(new_path(
                "nodes",
                &value.metadata.name,
                value.metadata.id.as_str(),
            )),
            Payload::CreateSynthesis(value) => Ok(new_path(
                "syntheses",
                &value.metadata.title,
                value.metadata.id.as_str(),
            )),
            Payload::SourceMetadata(_) => Ok(self
                .sources
                .get(&item.target_id.parse()?)
                .ok_or_else(|| error("SOURCE_NOT_FOUND", "The target source is absent."))?
                .path
                .clone()),
            _ => {
                let owner = self.owner(item)?;
                Ok(self
                    .nodes
                    .get(&owner)
                    .ok_or_else(|| error("NODE_NOT_FOUND", "The target Node is absent."))?
                    .0
                    .clone())
            }
        }
    }

    fn owner(&self, item: &ProposalItem) -> AppResult<NodeId> {
        if let Ok(id) = item.target_id.parse::<NodeId>() {
            return Ok(id);
        }
        if let Ok(id) = item.target_id.parse::<ClaimId>() {
            return self
                .claim_owners
                .get(&id)
                .cloned()
                .ok_or_else(|| error("CLAIM_NOT_FOUND", "The target Claim is absent."));
        }
        if let Ok(id) = item.target_id.parse::<RelationId>() {
            return self
                .relation_owners
                .get(&id)
                .cloned()
                .ok_or_else(|| error("RELATION_NOT_FOUND", "The target Relation is absent."));
        }
        Err(error(
            "INVALID_PROPOSAL_TARGET",
            "The operation target has no canonical owner.",
        ))
    }

    fn mutate_node(
        &mut self,
        owner: &NodeId,
        now: Timestamp,
        apply: impl FnOnce(&mut NodeDocument) -> AppResult<()>,
    ) -> AppResult<()> {
        let (_, original) = self
            .nodes
            .get(owner)
            .ok_or_else(|| error("NODE_NOT_FOUND", "The target Node is absent."))?;
        let mut next = original.clone();
        apply(&mut next)?;
        next.validate()?;
        if next.render()? != original.render()? {
            next.metadata.updated_at = now.max(next.metadata.updated_at);
            self.nodes.get_mut(owner).expect("validated Node").1 = next;
        }
        Ok(())
    }

    pub fn apply(
        &mut self,
        item: &ProposalItem,
        payload: &Payload,
        schema: &Schema,
        kind: ProposalKind,
        now: Timestamp,
    ) -> AppResult<()> {
        let exclusive = match payload {
            Payload::Summary(_) => Some("summary"),
            Payload::SourceMetadata(_) => Some("source_metadata"),
            Payload::ReplaceClaim(_)
            | Payload::RetractClaim(_)
            | Payload::ReplaceRelation(_)
            | Payload::RetractRelation(_) => Some("lifecycle"),
            _ => None,
        };
        if let Some(key) = exclusive
            && self.exclusive.contains(&(item.target_id.clone(), key))
        {
            return Err(error(
                "PROPOSAL_TARGET_CONFLICT",
                "Multiple items replace the same target state.",
            ));
        }
        match payload {
            Payload::CreateNode(value) => {
                if value.metadata.id.as_str() != item.target_id {
                    return Err(error(
                        "PROPOSAL_TARGET_MISMATCH",
                        "Node metadata must identify the operation target.",
                    ));
                }
                if self.nodes.contains_key(&value.metadata.id) {
                    return Err(error("NODE_ALREADY_EXISTS", "The Node ID already exists."));
                }
                value.metadata.validate()?;
                schema
                    .validate_properties(&value.metadata.node_type, &value.metadata.properties)?;
                if !schema
                    .packs
                    .iter()
                    .any(|pack| pack.key() == value.metadata.schema)
                {
                    return Err(error(
                        "SCHEMA_PACK_NOT_FOUND",
                        "The Node references an unavailable Schema Pack.",
                    ));
                }
                let path = new_path("nodes", &value.metadata.name, value.metadata.id.as_str());
                self.check_new_path(&path)?;
                let mut doc = NodeDocument::create(
                    value.metadata.clone(),
                    &format!("# {}", markdown_title(&value.metadata.name)),
                )?;
                doc.set_summary(&value.summary)?;
                doc.validate()?;
                self.nodes.insert(value.metadata.id.clone(), (path, doc));
            }
            Payload::Summary(value) => {
                let owner = self.owner(item)?;
                self.mutate_node(&owner, now, |doc| {
                    doc.set_summary(&value.summary)?;
                    Ok(())
                })?;
            }
            Payload::Alias(value) => {
                if value.alias.trim().is_empty()
                    || value.alias.len() > 2048
                    || value.alias.contains('\0')
                {
                    return Err(error(
                        "INVALID_ALIAS",
                        "Aliases require bounded nonempty text.",
                    ));
                }
                let owner = self.owner(item)?;
                self.mutate_node(&owner, now, |doc| {
                    if !doc.metadata.aliases.contains(&value.alias) {
                        doc.metadata.aliases.push(value.alias.clone());
                    }
                    Ok(())
                })?;
            }
            Payload::AddClaim(value) => {
                value.claim.validate()?;
                if value.claim.lifecycle_status != LifecycleStatus::Active
                    || !value.claim.conflict_groups.is_empty()
                {
                    return Err(error(
                        "INVALID_PROPOSAL_CLAIM",
                        "Create active Claims first and record conflicts in a separate item.",
                    ));
                }
                require_evidence(kind, &value.claim.evidence)?;
                if self.claim_owners.contains_key(&value.claim.id) {
                    return Err(error(
                        "CLAIM_ALREADY_EXISTS",
                        "The Claim ID already exists.",
                    ));
                }
                let owner = self.owner(item)?;
                self.mutate_node(&owner, now, |doc| {
                    doc.claims.push(value.claim.clone());
                    Ok(())
                })?;
                self.claim_owners.insert(value.claim.id.clone(), owner);
            }
            Payload::AddRelation(value) => {
                value.relation.validate()?;
                if value.relation.lifecycle_status != LifecycleStatus::Active {
                    return Err(error(
                        "INVALID_PROPOSAL_RELATION",
                        "New Relations must be active.",
                    ));
                }
                require_evidence(kind, &value.relation.evidence)?;
                if self.relation_owners.contains_key(&value.relation.id) {
                    return Err(error(
                        "RELATION_ALREADY_EXISTS",
                        "The Relation ID already exists.",
                    ));
                }
                let owner = self.owner(item)?;
                let source = &self
                    .nodes
                    .get(&owner)
                    .ok_or_else(|| error("NODE_NOT_FOUND", "The source Node is absent."))?
                    .1
                    .metadata;
                let target = &self
                    .nodes
                    .get(&value.relation.target_node_id)
                    .ok_or_else(|| error("NODE_NOT_FOUND", "The target Node is absent."))?
                    .1
                    .metadata;
                schema.validate_relation(
                    &value.relation.predicate,
                    &source.node_type,
                    &target.node_type,
                    !value.relation.evidence.is_empty()
                        || value.relation.evidence_status == EvidenceStatus::Unreviewed,
                )?;
                if schema.predicates[&value.relation.predicate].directed != value.relation.directed
                {
                    return Err(error(
                        "RELATION_DIRECTION_MISMATCH",
                        "Relation direction must match its Schema.",
                    ));
                }
                self.mutate_node(&owner, now, |doc| {
                    doc.relations.push(value.relation.clone());
                    Ok(())
                })?;
                self.relation_owners
                    .insert(value.relation.id.clone(), owner);
            }
            Payload::ReplaceClaim(value) => {
                let id: ClaimId = item.target_id.parse()?;
                let owner = self.owner(item)?;
                if value.replacement_id == id
                    || self.claim_owners.get(&value.replacement_id) != Some(&owner)
                {
                    return Err(error(
                        "INVALID_CLAIM_REPLACEMENT",
                        "The replacement must be another Claim on the same subject.",
                    ));
                }
                self.mutate_node(&owner, now, |doc| {
                    if doc
                        .claims
                        .iter()
                        .find(|claim| claim.id == value.replacement_id)
                        .is_none_or(|claim| claim.lifecycle_status != LifecycleStatus::Active)
                    {
                        return Err(error(
                            "INVALID_CLAIM_REPLACEMENT",
                            "The replacement Claim must be active.",
                        ));
                    }
                    doc.claims
                        .iter_mut()
                        .find(|claim| claim.id == id)
                        .expect("indexed Claim")
                        .lifecycle_status = LifecycleStatus::Superseded;
                    Ok(())
                })?;
            }
            Payload::RetractClaim(value) => {
                reason(&value.reason)?;
                let id: ClaimId = item.target_id.parse()?;
                let owner = self.owner(item)?;
                self.mutate_node(&owner, now, |doc| {
                    doc.claims
                        .iter_mut()
                        .find(|claim| claim.id == id)
                        .expect("indexed Claim")
                        .lifecycle_status = LifecycleStatus::Retracted;
                    Ok(())
                })?;
            }
            Payload::ReplaceRelation(value) => {
                let id: RelationId = item.target_id.parse()?;
                let owner = self.owner(item)?;
                if value.replacement_id == id
                    || self.relation_owners.get(&value.replacement_id) != Some(&owner)
                {
                    return Err(error(
                        "INVALID_RELATION_REPLACEMENT",
                        "The replacement must be another Relation owned by the same Node.",
                    ));
                }
                self.mutate_node(&owner, now, |doc| {
                    if doc
                        .relations
                        .iter()
                        .find(|relation| relation.id == value.replacement_id)
                        .is_none_or(|relation| relation.lifecycle_status != LifecycleStatus::Active)
                    {
                        return Err(error(
                            "INVALID_RELATION_REPLACEMENT",
                            "The replacement Relation must be active.",
                        ));
                    }
                    doc.relations
                        .iter_mut()
                        .find(|relation| relation.id == id)
                        .expect("indexed Relation")
                        .lifecycle_status = LifecycleStatus::Superseded;
                    Ok(())
                })?;
            }
            Payload::RetractRelation(value) => {
                reason(&value.reason)?;
                let id: RelationId = item.target_id.parse()?;
                let owner = self.owner(item)?;
                self.mutate_node(&owner, now, |doc| {
                    doc.relations
                        .iter_mut()
                        .find(|relation| relation.id == id)
                        .expect("indexed Relation")
                        .lifecycle_status = LifecycleStatus::Retracted;
                    Ok(())
                })?;
            }
            Payload::AddEvidence(value) => {
                if value.evidence.is_empty() {
                    return Err(error("EVIDENCE_REQUIRED", "Provide evidence to append."));
                }
                let owner = self.owner(item)?;
                self.mutate_node(&owner, now, |doc| {
                    if let Ok(id) = item.target_id.parse::<ClaimId>() {
                        let claim = doc
                            .claims
                            .iter_mut()
                            .find(|claim| claim.id == id)
                            .expect("indexed Claim");
                        append_evidence(
                            &mut claim.evidence,
                            &value.evidence,
                            &mut claim.evidence_status,
                        )?;
                    } else {
                        let id: RelationId = item.target_id.parse()?;
                        let relation = doc
                            .relations
                            .iter_mut()
                            .find(|relation| relation.id == id)
                            .expect("indexed Relation");
                        append_evidence(
                            &mut relation.evidence,
                            &value.evidence,
                            &mut relation.evidence_status,
                        )?;
                    }
                    Ok(())
                })?;
            }
            Payload::RecordConflict(value) => {
                value.group.validate()?;
                let owner = self.owner(item)?;
                for (other_id, (_, doc)) in &self.nodes {
                    if other_id != &owner
                        && doc.claims.iter().any(|claim| {
                            claim
                                .conflict_groups
                                .iter()
                                .any(|group| group.id == value.group.id)
                        })
                    {
                        return Err(error(
                            "CONFLICT_GROUP_ID_CONFLICT",
                            "The conflict group belongs to another subject.",
                        ));
                    }
                }
                self.mutate_node(&owner, now, |doc| {
                    if let Some(previous) = doc.claims.iter().flat_map(|claim| &claim.conflict_groups).find(|group| group.id == value.group.id)
                        && (previous.created_at != value.group.created_at
                            || (previous.status != crate::domain::ConflictGroupStatus::Open && previous != &value.group))
                    {
                        return Err(error(
                            "CONFLICT_HISTORY_IMMUTABLE",
                            "Conflict creation time and closed group history cannot be rewritten. Record a new group instead.",
                        ));
                    }
                    let mut affected: BTreeSet<_> = value.group.claim_ids.iter().cloned().collect();
                    for claim in &doc.claims {
                        if claim
                            .conflict_groups
                            .iter()
                            .any(|group| group.id == value.group.id)
                        {
                            affected.insert(claim.id.clone());
                        }
                    }
                    if value
                        .member_statuses
                        .keys()
                        .any(|id| !affected.contains(id))
                    {
                        return Err(error(
                            "INVALID_CONFLICT_MEMBERSHIP",
                            "Status changes must belong to affected group members.",
                        ));
                    }
                    for id in &value.group.claim_ids {
                        if !doc.claims.iter().any(|claim| &claim.id == id) {
                            return Err(error(
                                "CONFLICT_CLAIM_MISSING",
                                "A conflict member is missing from this Node.",
                            ));
                        }
                    }
                    for claim in &mut doc.claims {
                        claim
                            .conflict_groups
                            .retain(|group| group.id != value.group.id);
                        if value.group.claim_ids.contains(&claim.id) {
                            claim.conflict_groups.push(value.group.clone());
                            if value.group.status == crate::domain::ConflictGroupStatus::Open {
                                claim.evidence_status = EvidenceStatus::Conflicting;
                            }
                        }
                        if let Some(status) = value.member_statuses.get(&claim.id) {
                            claim.evidence_status = *status;
                        }
                    }
                    Ok(())
                })?;
            }
            Payload::CreateSynthesis(value) => {
                if value.metadata.id.as_str() != item.target_id {
                    return Err(error(
                        "PROPOSAL_TARGET_MISMATCH",
                        "Synthesis metadata must identify the operation target.",
                    ));
                }
                if self.syntheses.contains_key(&value.metadata.id) {
                    return Err(error(
                        "SYNTHESIS_ALREADY_EXISTS",
                        "The Synthesis ID already exists.",
                    ));
                }
                let doc = SynthesisDocument::create(value.metadata.clone(), &value.body)?;
                let evidence = self.evidence()?;
                self.validate_synthesis(&value.metadata, schema, &evidence)?;
                doc.validate_citations(&evidence.keys().cloned().collect())?;
                if schema.policies.synthesis_requires_citation && doc.citations()?.is_empty() {
                    return Err(error(
                        "CITATION_REQUIRED",
                        "The current Schema requires a Synthesis citation.",
                    ));
                }
                for id in &value.metadata.related_nodes {
                    if !self.nodes.contains_key(id) {
                        return Err(error("NODE_NOT_FOUND", "A related Node is absent."));
                    }
                }
                let path = new_path(
                    "syntheses",
                    &value.metadata.title,
                    value.metadata.id.as_str(),
                );
                self.check_new_path(&path)?;
                self.syntheses
                    .insert(value.metadata.id.clone(), (path, doc));
            }
            Payload::SourceMetadata(value) => {
                for id in &value.represented_nodes {
                    if !self.nodes.contains_key(id) {
                        return Err(error("NODE_NOT_FOUND", "A represented Node is absent."));
                    }
                }
                let id: SourceId = item.target_id.parse()?;
                let original = self
                    .sources
                    .get(&id)
                    .ok_or_else(|| error("SOURCE_NOT_FOUND", "The target source is absent."))?;
                let mut file = original.clone();
                file.manifest.title = value.title.clone();
                file.manifest.kind = value.kind.clone();
                file.manifest.authors = value.authors.clone();
                file.manifest.identifiers = value.identifiers.clone();
                file.manifest.language = value.language.clone();
                file.manifest.tags = value.tags.clone();
                file.manifest.represented_nodes = value.represented_nodes.clone();
                if file.manifest != original.manifest {
                    file.manifest.updated_at = now.max(file.manifest.updated_at);
                }
                file.render()?;
                self.sources.insert(id, file);
            }
        }
        if let Some(key) = exclusive {
            self.exclusive.insert((item.target_id.clone(), key));
        }
        Ok(())
    }

    fn check_new_path(&self, path: &std::path::Path) -> AppResult<()> {
        let key = path_key(path);
        if self
            .originals
            .keys()
            .chain(self.nodes.values().map(|(path, _)| path))
            .chain(self.syntheses.values().map(|(path, _)| path))
            .any(|path| path_key(path) == key)
        {
            return Err(error(
                "CANONICAL_FILE_CONFLICT",
                "The generated document path is already occupied.",
            ));
        }
        Ok(())
    }

    pub fn evidence(&self) -> AppResult<BTreeMap<EvidenceId, Evidence>> {
        let mut values = BTreeMap::new();
        for (_, doc) in self.nodes.values() {
            for evidence in doc
                .claims
                .iter()
                .flat_map(|claim| &claim.evidence)
                .chain(doc.relations.iter().flat_map(|relation| &relation.evidence))
            {
                if let Some(previous) = values.insert(evidence.id.clone(), evidence.clone())
                    && previous != *evidence
                {
                    return Err(error(
                        "EVIDENCE_ID_CONFLICT",
                        "Shared Evidence payloads must be identical.",
                    ));
                }
            }
        }
        Ok(values)
    }

    pub fn rendered(&self) -> AppResult<BTreeMap<PathBuf, Vec<u8>>> {
        let mut changes = BTreeMap::new();
        for (path, doc) in self.nodes.values() {
            self.add_change(&mut changes, path, doc.render()?.into_bytes());
        }
        for (path, doc) in self.syntheses.values() {
            self.add_change(&mut changes, path, doc.render()?.into_bytes());
        }
        for source in self.sources.values() {
            self.add_change(&mut changes, &source.path, source.render()?.into_bytes());
        }
        Ok(changes)
    }
    fn add_change(
        &self,
        changes: &mut BTreeMap<PathBuf, Vec<u8>>,
        path: &std::path::Path,
        bytes: Vec<u8>,
    ) {
        if self.originals.get(path) != Some(&bytes) {
            changes.insert(path.to_owned(), bytes);
        }
    }
}

fn append_evidence(
    existing: &mut Vec<Evidence>,
    incoming: &[Evidence],
    status: &mut EvidenceStatus,
) -> AppResult<()> {
    for evidence in incoming {
        evidence.validate()?;
        if let Some(old) = existing.iter().find(|old| old.id == evidence.id) {
            if old != evidence {
                return Err(error(
                    "EVIDENCE_ID_CONFLICT",
                    "Existing Evidence cannot be silently overwritten.",
                ));
            }
        } else {
            existing.push(evidence.clone());
        }
    }
    if existing
        .iter()
        .any(|evidence| evidence.stance == EvidenceStance::Supports)
        && existing
            .iter()
            .any(|evidence| evidence.stance == EvidenceStance::Contradicts)
    {
        *status = EvidenceStatus::Conflicting;
    }
    Ok(())
}

fn require_evidence(kind: ProposalKind, evidence: &[Evidence]) -> AppResult<()> {
    if matches!(kind, ProposalKind::Compile | ProposalKind::Refresh) && evidence.is_empty() {
        return Err(error(
            "EVIDENCE_REQUIRED",
            "Compiler assertions require verified Evidence.",
        ));
    }
    Ok(())
}
fn reason(value: &str) -> AppResult<()> {
    if value.trim().is_empty() || value.len() > 4096 {
        Err(error(
            "INVALID_PROPOSAL_PAYLOAD",
            "Provide a bounded nonempty reason.",
        ))
    } else {
        Ok(())
    }
}
fn utf8(bytes: &[u8]) -> AppResult<&str> {
    std::str::from_utf8(bytes).map_err(|_| {
        error(
            "INVALID_DOCUMENT_ENCODING",
            "Canonical documents must use UTF-8.",
        )
    })
}
fn new_path(directory: &str, title: &str, id: &str) -> PathBuf {
    let mut slug = String::new();
    for character in title.chars() {
        if slug.len() >= 48 {
            break;
        }
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    let slug = if slug.is_empty() { "knowledge" } else { slug };
    PathBuf::from(format!("knowledge/{directory}"))
        .join(format!("{slug}--{}.md", &id[id.len() - 8..]))
}

fn markdown_title(value: &str) -> String {
    let mut title = String::new();
    for character in value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
    {
        if character.is_ascii_punctuation() {
            title.push('\\');
        }
        title.push(character);
    }
    title
}
