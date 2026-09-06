use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

use pulldown_cmark::{Event, LinkType, Parser, Tag, TagEnd};
use schemars::JsonSchema;
use serde::Serialize;

use super::markdown::{
    MarkdownFile, managed_ranges, managed_yaml, markdown_options, render_managed,
};
use crate::{
    domain::{ClaimRecord, NodeMetadata, RelationRecord, claim_conflict_groups, knowledge_error},
    error::AppResult,
};

mod summary;

#[derive(Debug)]
pub struct NodeDocument {
    pub metadata: NodeMetadata,
    pub claims: Vec<ClaimRecord>,
    pub relations: Vec<RelationRecord>,
    file: MarkdownFile,
    before_metadata: NodeMetadata,
    before_claims: Vec<ClaimRecord>,
    before_relations: Vec<RelationRecord>,
    ranges: BTreeMap<&'static str, Range<usize>>,
    summary_edit: Option<(Range<usize>, String)>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct NodeLink {
    pub target: String,
    pub display: String,
    pub byte_start: usize,
    pub byte_end: usize,
}

impl NodeDocument {
    pub fn parse(text: &str) -> AppResult<Self> {
        let file = MarkdownFile::parse(text)?;
        let metadata: NodeMetadata = file.metadata()?;
        let ranges = managed_ranges(&file)?;
        let claims: Vec<ClaimRecord> = managed_yaml(&text[ranges["claims"].clone()])?;
        let relations: Vec<RelationRecord> = managed_yaml(&text[ranges["relations"].clone()])?;
        let document = Self {
            before_metadata: metadata.clone(),
            before_claims: claims.clone(),
            before_relations: relations.clone(),
            metadata,
            claims,
            relations,
            file,
            ranges,
            summary_edit: None,
        };
        document.validate()?;
        Ok(document)
    }

    pub fn create(metadata: NodeMetadata, body: &str) -> AppResult<Self> {
        metadata.validate()?;
        let frontmatter = serde_yaml::to_string(&metadata).map_err(|_| {
            knowledge_error("NODE_ENCODE_FAILED", "Could not encode node metadata.")
        })?;
        let text = format!(
            "---\n{frontmatter}---\n\n{body}\n\n<!-- knowmesh:claims:begin -->\n{}<!-- knowmesh:claims:end -->\n\n<!-- knowmesh:relations:begin -->\n{}<!-- knowmesh:relations:end -->\n",
            render_managed::<ClaimRecord>(&[], "\n")?,
            render_managed::<RelationRecord>(&[], "\n")?
        );
        Self::parse(&text)
    }

    pub fn validate(&self) -> AppResult<()> {
        self.metadata.validate()?;
        if self.metadata.id != self.before_metadata.id
            || self.metadata.created_at != self.before_metadata.created_at
        {
            return Err(knowledge_error(
                "NODE_IDENTITY_CHANGED",
                "A node writer cannot replace an existing node identity or creation timestamp.",
            ));
        }
        let mut ids = BTreeSet::new();
        for claim in &self.claims {
            claim.validate()?;
            if !ids.insert(claim.id.as_str()) {
                return Err(knowledge_error(
                    "DUPLICATE_ASSERTION_ID",
                    "Assertion IDs must be unique.",
                ));
            }
        }
        for relation in &self.relations {
            relation.validate()?;
            if !ids.insert(relation.id.as_str()) {
                return Err(knowledge_error(
                    "DUPLICATE_ASSERTION_ID",
                    "Assertion IDs must be unique.",
                ));
            }
        }
        claim_conflict_groups(&self.claims)?;
        let mut evidence_ids = BTreeMap::new();
        for evidence in self.claims.iter().flat_map(|claim| &claim.evidence).chain(
            self.relations
                .iter()
                .flat_map(|relation| &relation.evidence),
        ) {
            if let Some(previous) = evidence_ids.insert(&evidence.id, evidence)
                && previous != evidence
            {
                return Err(knowledge_error(
                    "EVIDENCE_ID_CONFLICT",
                    "A shared Evidence ID must have identical content in every assertion.",
                ));
            }
        }
        Ok(())
    }

    pub fn render(&self) -> AppResult<String> {
        self.validate()?;
        let mut replacements = Vec::new();
        if let Some(change) = &self.summary_edit {
            replacements.push(change.clone());
        }
        if self.metadata != self.before_metadata {
            replacements.push((
                self.file.header.clone(),
                self.file
                    .render_metadata(&self.before_metadata, &self.metadata)?,
            ));
        }
        if self.claims != self.before_claims {
            let mut claims = self.claims.clone();
            claims.sort_by(|a, b| a.id.cmp(&b.id));
            replacements.push((
                self.ranges["claims"].clone(),
                render_managed(&claims, self.file.newline)?,
            ));
        }
        if self.relations != self.before_relations {
            let mut relations = self.relations.clone();
            relations.sort_by(|a, b| a.id.cmp(&b.id));
            replacements.push((
                self.ranges["relations"].clone(),
                render_managed(&relations, self.file.newline)?,
            ));
        }
        self.file.render(replacements)
    }

    pub fn body(&self) -> &str {
        self.file.body()
    }

    pub fn links(&self) -> Vec<NodeLink> {
        let mut result = Vec::new();
        let mut current: Option<NodeLink> = None;
        for (event, span) in
            Parser::new_ext(self.file.body(), markdown_options()).into_offset_iter()
        {
            match event {
                Event::Start(Tag::Link {
                    link_type: LinkType::WikiLink { .. },
                    dest_url,
                    ..
                }) => {
                    current = Some(NodeLink {
                        target: dest_url.to_string(),
                        display: String::new(),
                        byte_start: self.file.body_start + span.start,
                        byte_end: self.file.body_start + span.end,
                    });
                }
                Event::Text(text) | Event::Code(text) => {
                    if let Some(link) = current.as_mut() {
                        link.display.push_str(&text);
                    }
                }
                Event::End(TagEnd::Link) => {
                    if let Some(link) = current.take() {
                        result.push(link);
                    }
                }
                _ => {}
            }
        }
        result
    }
}
