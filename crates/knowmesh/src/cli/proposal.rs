use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use clap::Subcommand;
use knowmesh_core::{
    application::proposal::{
        MAX_PROPOSAL_RECORD_BYTES,
        apply::{self, ApplyInput},
        workflow::{
            self, CreateInput, EditInput, GetInput, RejectInput, RevalidateInput, ReviewRequest,
        },
    },
    canonical::workspace::Workspace,
    domain::{ProposalId, Timestamp},
    error::{AppError, AppResult, ErrorType},
};
use serde::{Serialize, de::DeserializeOwned};

#[derive(Subcommand)]
pub(super) enum ProposalCommand {
    /// Create a Proposal from the proposal.create JSON contract.
    Create {
        #[arg(long, value_name = "FILE")]
        input: PathBuf,
        #[arg(long)]
        dry_run: bool,
    },
    /// Read current or historical Proposal metadata.
    Get {
        proposal_id: ProposalId,
        #[arg(long)]
        revision: Option<u32>,
    },
    /// Review items using the proposal.review JSON contract.
    Review {
        #[arg(long, value_name = "FILE")]
        input: PathBuf,
        #[arg(long)]
        dry_run: bool,
    },
    /// Replace the draft using the proposal.edit JSON contract.
    Edit {
        #[arg(long, value_name = "FILE")]
        input: PathBuf,
        #[arg(long)]
        dry_run: bool,
    },
    /// Revalidate a Proposal against current canonical content.
    Revalidate {
        proposal_id: ProposalId,
        #[arg(long)]
        expected_revision: u32,
        #[arg(long)]
        dry_run: bool,
    },
    /// Reject a Proposal while preserving its history.
    Reject {
        proposal_id: ProposalId,
        #[arg(long)]
        expected_revision: u32,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Apply an approved Proposal through the canonical transaction coordinator.
    Apply {
        proposal_id: ProposalId,
        #[arg(long)]
        expected_revision: u32,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        yes: bool,
    },
}

pub(super) fn execute(
    command: &ProposalCommand,
    workspace: &Workspace,
) -> AppResult<serde_json::Value> {
    let now = Timestamp::now();
    let actor = "human_cli";
    match command {
        ProposalCommand::Create { input, dry_run } => {
            let mut input: CreateInput = read_input(input)?;
            input.dry_run |= dry_run;
            encode(workflow::create(
                workspace,
                crate::runtime::open_proposal_store(workspace, !input.dry_run)?.as_mut(),
                &input,
                actor,
                now,
            )?)
        }
        ProposalCommand::Get {
            proposal_id,
            revision,
        } => encode(workflow::get(
            workspace,
            crate::runtime::open_proposal_store(workspace, false)?.as_ref(),
            &GetInput {
                proposal_id: proposal_id.clone(),
                revision: *revision,
            },
        )?),
        ProposalCommand::Review { input, dry_run } => {
            let mut input: ReviewRequest = read_input(input)?;
            input.dry_run |= dry_run;
            encode(workflow::review(
                workspace,
                crate::runtime::open_proposal_store(workspace, !input.dry_run)?.as_mut(),
                &input,
                actor,
                now,
            )?)
        }
        ProposalCommand::Edit { input, dry_run } => {
            let mut input: EditInput = read_input(input)?;
            input.dry_run |= dry_run;
            encode(workflow::edit(
                workspace,
                crate::runtime::open_proposal_store(workspace, !input.dry_run)?.as_mut(),
                &input,
                actor,
                now,
            )?)
        }
        ProposalCommand::Revalidate {
            proposal_id,
            expected_revision,
            dry_run,
        } => encode(workflow::revalidate(
            workspace,
            crate::runtime::open_proposal_store(workspace, !dry_run)?.as_mut(),
            &RevalidateInput {
                proposal_id: proposal_id.clone(),
                expected_revision: *expected_revision,
                dry_run: *dry_run,
            },
            actor,
            now,
        )?),
        ProposalCommand::Reject {
            proposal_id,
            expected_revision,
            reason,
            dry_run,
        } => encode(workflow::reject(
            workspace,
            crate::runtime::open_proposal_store(workspace, !dry_run)?.as_mut(),
            &RejectInput {
                proposal_id: proposal_id.clone(),
                expected_revision: *expected_revision,
                reason: reason.clone(),
                dry_run: *dry_run,
            },
            actor,
            now,
        )?),
        ProposalCommand::Apply {
            proposal_id,
            expected_revision,
            dry_run,
            yes,
        } => {
            let input = ApplyInput {
                proposal_id: proposal_id.clone(),
                expected_revision: *expected_revision,
                dry_run: *dry_run,
                yes: *yes,
            };
            apply::validate_input(&input)?;
            encode(apply::execute(
                workspace,
                crate::runtime::open_proposal_store(workspace, !dry_run)?.as_mut(),
                &input,
                actor,
                now,
            )?)
        }
    }
}

fn read_input<T: DeserializeOwned>(path: &Path) -> AppResult<T> {
    let mut bytes = Vec::new();
    let mut reader: Box<dyn Read> = if path == Path::new("-") {
        Box::new(std::io::stdin().lock())
    } else {
        let path = path.canonicalize().map_err(|_| unreadable())?;
        if !std::fs::metadata(&path)
            .map_err(|_| unreadable())?
            .is_file()
        {
            return Err(unreadable());
        }
        Box::new(File::open(path).map_err(|_| unreadable())?)
    };
    reader
        .by_ref()
        .take(MAX_PROPOSAL_RECORD_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unreadable())?;
    if bytes.len() > MAX_PROPOSAL_RECORD_BYTES {
        return Err(AppError::new(
            ErrorType::Validation,
            "INPUT_TOO_LARGE",
            "Proposal JSON input must not exceed 20 MiB.",
        )
        .with_param("input"));
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        AppError::new(
            ErrorType::Validation,
            "INVALID_INPUT_JSON",
            "The input does not match the operation JSON contract.",
        )
        .with_param("input")
        .with_hint("Inspect the request using `knowmesh schema command <operation>`.")
    })
}

fn unreadable() -> AppError {
    AppError::new(
        ErrorType::Io,
        "INPUT_UNREADABLE",
        "Could not read the JSON input.",
    )
    .with_param("input")
}
fn encode(value: impl Serialize) -> AppResult<serde_json::Value> {
    serde_json::to_value(value).map_err(|_| {
        AppError::new(
            ErrorType::Internal,
            "OUTPUT_ENCODE_FAILED",
            "Could not encode the operation result.",
        )
    })
}
