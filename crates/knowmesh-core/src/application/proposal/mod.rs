mod documents;
mod evidence;
pub mod payload;
mod record;
mod selection;

pub use record::{MAX_PROPOSAL_RECORD_BYTES, ProposalRecord};
pub use selection::{AcceptedPreview, prepare_accepted};

use std::{collections::BTreeMap, path::PathBuf};

use crate::{
    canonical::{
        schema::Schema,
        snapshot::{CanonicalPreview, CanonicalSnapshot},
        workspace::Workspace,
    },
    domain::{
        Timestamp,
        proposal::{Proposal, ProposalInput, ProposalIssue, ProposalItem, ProposalKind},
    },
    error::{AppError, AppResult, ErrorType},
};
use documents::Documents;
use payload::Payload;

#[derive(Debug)]
pub struct PreparedProposal {
    pub proposal: Proposal,
    pub base_snapshot_sha256: String,
    pub preview: Option<CanonicalPreview>,
    documents: BTreeMap<PathBuf, Vec<u8>>,
}

impl PreparedProposal {
    pub fn documents(&self) -> &BTreeMap<PathBuf, Vec<u8>> {
        &self.documents
    }
}

pub fn prepare(
    workspace: &Workspace,
    input: &ProposalInput,
    actor: &str,
    now: Timestamp,
) -> AppResult<PreparedProposal> {
    let before = CanonicalSnapshot::scan(workspace)?;
    prepare_snapshot(workspace, input, actor, now, before)
}

fn prepare_snapshot(
    workspace: &Workspace,
    input: &ProposalInput,
    actor: &str,
    now: Timestamp,
    before: CanonicalSnapshot,
) -> AppResult<PreparedProposal> {
    let schema = Schema::load(workspace)?;
    if input.schema_hash != before.schema_hash || schema.hash != before.schema_hash {
        return Err(error(
            "PROPOSAL_SCHEMA_MISMATCH",
            "The Proposal input must use the current workspace Schema.",
        ));
    }
    if matches!(input.kind, ProposalKind::Compile | ProposalKind::Refresh)
        && input.source_revision_id.is_none()
    {
        return Err(error(
            "PROPOSAL_SOURCE_REQUIRED",
            "Compiler Proposals must identify their source revision.",
        ));
    }
    if input.source_revision_id.as_ref().is_some_and(|id| {
        !before.sources.iter().any(|source| {
            source
                .manifest
                .revisions
                .iter()
                .any(|revision| &revision.id == id)
        })
    }) {
        return Err(error(
            "SOURCE_REVISION_NOT_FOUND",
            "The Proposal source revision is absent from the workspace.",
        ));
    }
    let mut proposal = Proposal::new(input.clone(), actor, now)?;
    let mut payloads = Vec::with_capacity(proposal.items.len());
    for item in &mut proposal.items {
        match Payload::decode(item) {
            Ok(payload) => payloads.push(Some(payload)),
            Err(error) => {
                issue(item, error);
                payloads.push(None);
            }
        }
    }
    evidence::verify(workspace, &before, &mut proposal, &payloads);
    let mut working = Documents::load(workspace, &before)?;
    let original_files: BTreeMap<_, _> = before
        .files
        .iter()
        .map(|file| (&file.path, &file.sha256))
        .collect();
    let mut order: Vec<_> = payloads
        .iter()
        .enumerate()
        .filter_map(|(index, payload)| payload.as_ref().map(|payload| (payload.phase(), index)))
        .collect();
    order.sort();
    for (_, index) in order {
        if proposal.items[index]
            .issues
            .iter()
            .any(|issue| issue.blocking)
        {
            continue;
        }
        let payload = payloads[index].as_ref().expect("decoded payload");
        let result = (|| {
            let path = working.path(&proposal.items[index], payload)?;
            let before_hash = original_files.get(&path).map(|hash| (*hash).clone());
            if proposal.items[index].before_sha256.is_some()
                && proposal.items[index].before_sha256 != before_hash
            {
                return Err(error(
                    "CANONICAL_FILE_CONFLICT",
                    "A supplied before hash does not match the target file.",
                ));
            }
            proposal.items[index].before_sha256 = before_hash;
            working.apply(&proposal.items[index], payload, &schema, proposal.kind, now)
        })();
        if let Err(error) = result {
            issue(&mut proposal.items[index], error);
        }
    }
    let mut documents = working.rendered()?;
    let preview = match before.preview_documents(workspace, &documents) {
        Ok(preview) => Some(preview),
        Err(error) => {
            for item in &mut proposal.items {
                if !item.issues.iter().any(|issue| issue.blocking) {
                    issue(item, error.clone());
                }
            }
            documents.clear();
            None
        }
    };
    proposal.validate()?;
    Ok(PreparedProposal {
        proposal,
        base_snapshot_sha256: before.content_sha256,
        preview,
        documents,
    })
}

fn issue(item: &mut ProposalItem, error: AppError) {
    if !item
        .issues
        .iter()
        .any(|issue| issue.code == error.code && issue.message == error.message && issue.blocking)
    {
        let issue = ProposalIssue {
            code: error.code,
            message: error.message,
            blocking: true,
        };
        if item.issues.len() < 128 {
            item.issues.push(issue);
        } else {
            item.issues[127] = ProposalIssue {
                code: "PROPOSAL_ISSUE_LIMIT".into(),
                message: "Additional validation failures exceeded the diagnostic limit. Fix the payload and prepare a new revision.".into(),
                blocking: true,
            };
        }
    }
}

fn error(code: &str, message: &str) -> AppError {
    AppError::new(ErrorType::Validation, code, message)
}
