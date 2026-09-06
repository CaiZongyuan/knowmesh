use std::{
    io::{self, Write},
    path::PathBuf,
    time::Instant,
};

use clap::{Parser, Subcommand, ValueEnum, error::ErrorKind};
use knowmesh_core::{
    application::{
        doctor::{self, IndexAccess, RepairInput},
        impact::{self, ImpactInput, ImpactKind},
        lexical::{QuerySyntax, RecordType},
        operations,
        rebuild::{self, RebuildInput},
        schema::{self, PackInput},
        search::{self, SearchInput},
        source,
        source_read::{self, ContentId, ContentInput},
        status, sync,
        workspace::{self, InitInput},
    },
    canonical::{source::ImportInput, workspace::Workspace},
    domain::{RunId, SourceId, SourceRevisionId, StorageMode, TextEncoding, WorkspaceId},
    error::{AppError, ErrorType},
    wire::{Failure, Metadata, Success},
};
use serde::Serialize;

mod proposal;
use proposal::ProposalCommand;

#[derive(Parser)]
#[command(
    name = "knowmesh",
    about = "Evidence-backed knowledge workspace",
    disable_help_subcommand = true
)]
struct Cli {
    #[arg(long, global = true)]
    workspace: Option<PathBuf>,
    #[arg(long, global = true, value_enum)]
    format: Option<OutputFormat>,
    #[arg(long, global = true)]
    trace_id: Option<RunId>,
    #[arg(long, global = true)]
    no_sync: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum OutputFormat {
    Json,
    Pretty,
    Table,
    Ndjson,
    Csv,
}

#[derive(Subcommand)]
enum Command {
    Version,
    Proposal {
        #[command(subcommand)]
        command: ProposalCommand,
    },
    Status,
    Search {
        query: String,
        #[arg(long, default_value = "literal")]
        query_syntax: QuerySyntax,
        #[arg(long = "record-type")]
        record_types: Vec<RecordType>,
        #[arg(long = "node-type")]
        node_types: Vec<String>,
        #[arg(long = "source-id")]
        source_ids: Vec<SourceId>,
        #[arg(long = "tag")]
        tags: Vec<String>,
        #[arg(long = "status", default_value = "active")]
        statuses: Vec<String>,
        #[arg(long)]
        limit: Option<u32>,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        include_graph_paths: bool,
        #[arg(long)]
        explain: bool,
    },
    Init {
        path: Option<PathBuf>,
        #[arg(long, default_value = "Knowledge Space")]
        name: String,
        #[arg(long, default_value = "research")]
        template: String,
        #[arg(long)]
        dry_run: bool,
    },
    Schema {
        #[command(subcommand)]
        command: SchemaCommand,
    },
    Sync {
        #[arg(long)]
        dry_run: bool,
    },
    Rebuild {
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        discard_runtime: bool,
        #[arg(long, default_value_t = 3)]
        keep_backups: usize,
    },
    Source {
        #[command(subcommand)]
        command: SourceCommand,
    },
    Doctor {
        #[arg(long)]
        repair: bool,
        #[arg(long, requires = "repair")]
        dry_run: bool,
        #[arg(long, requires = "repair")]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum SourceCommand {
    List {
        #[arg(long)]
        include_removed: bool,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: u32,
        #[arg(long)]
        cursor: Option<String>,
    },
    Get {
        source_id: SourceId,
    },
    Content {
        id: ContentId,
        #[arg(long, conflicts_with = "format")]
        raw: bool,
    },
    Impact {
        source_id: SourceId,
        #[arg(long)]
        revision: Option<SourceRevisionId>,
        #[arg(long)]
        kind: Option<ImpactKind>,
        #[arg(long, default_value_t = 20)]
        limit: u32,
        #[arg(long)]
        cursor: Option<String>,
    },
    Add {
        path: PathBuf,
        #[arg(long)]
        source_id: Option<SourceId>,
        #[arg(long, value_enum)]
        storage: Option<StorageArg>,
        #[arg(long)]
        encoding: Option<TextEncoding>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, default_value = "document")]
        kind: String,
        #[arg(long = "tag")]
        tags: Vec<String>,
        #[arg(long)]
        allow_private_network: bool,
        #[arg(long)]
        dry_run: bool,
    },
    Remove {
        source_id: SourceId,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum StorageArg {
    Managed,
    Referenced,
    SnapshotUrl,
}

impl From<StorageArg> for StorageMode {
    fn from(value: StorageArg) -> Self {
        match value {
            StorageArg::Managed => Self::Managed,
            StorageArg::Referenced => Self::Referenced,
            StorageArg::SnapshotUrl => Self::SnapshotUrl,
        }
    }
}

#[derive(Subcommand)]
enum SchemaCommand {
    List,
    Command {
        operation: String,
    },
    Pack {
        id: String,
    },
    Patch {
        op: knowmesh_core::domain::proposal::PatchOp,
    },
}

impl Command {
    fn operation_name(&self) -> &'static str {
        match self {
            Self::Version => "version",
            Self::Proposal {
                command: ProposalCommand::Create { .. },
            } => "proposal.create",
            Self::Proposal {
                command: ProposalCommand::Get { .. },
            } => "proposal.get",
            Self::Proposal {
                command: ProposalCommand::Review { .. },
            } => "proposal.review",
            Self::Proposal {
                command: ProposalCommand::Edit { .. },
            } => "proposal.edit",
            Self::Proposal {
                command: ProposalCommand::Revalidate { .. },
            } => "proposal.revalidate",
            Self::Proposal {
                command: ProposalCommand::Reject { .. },
            } => "proposal.reject",
            Self::Proposal {
                command: ProposalCommand::Apply { .. },
            } => "proposal.apply",
            Self::Schema {
                command: SchemaCommand::Patch { .. },
            } => "schema.patch",
            Self::Status => "status",
            Self::Search { .. } => "knowledge.search",
            Self::Init { .. } => "init",
            Self::Schema {
                command: SchemaCommand::List,
            } => "schema.list",
            Self::Schema {
                command: SchemaCommand::Command { .. },
            } => "schema.command",
            Self::Schema {
                command: SchemaCommand::Pack { .. },
            } => "schema.pack",
            Self::Sync { .. } => "sync",
            Self::Rebuild { .. } => "rebuild",
            Self::Doctor { repair: true, .. } => "doctor.repair",
            Self::Doctor { .. } => "doctor",
            Self::Source {
                command: SourceCommand::List { .. },
            } => "source.list",
            Self::Source {
                command: SourceCommand::Get { .. },
            } => "source.get",
            Self::Source {
                command: SourceCommand::Content { .. },
            } => "source.content",
            Self::Source {
                command: SourceCommand::Add { .. },
            } => "source.add",
            Self::Source {
                command: SourceCommand::Remove { .. },
            } => "source.remove",
            Self::Source {
                command: SourceCommand::Impact { .. },
            } => "source.impact",
        }
    }
}

pub fn run() -> u8 {
    let start = Instant::now();
    let trace = RunId::new();
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            return if error.print().is_ok() { 0 } else { 4 };
        }
        Err(error) => {
            return fail(
                AppError::new(ErrorType::Validation, "INVALID_ARGUMENT", error.to_string())
                    .with_hint("Run `knowmesh --help` for valid commands and arguments."),
                "parse",
                trace,
                start,
            );
        }
    };
    let trace = cli.trace_id.unwrap_or(trace);
    let command = cli.command.operation_name();
    if let Command::Source {
        command: SourceCommand::Content { id, raw: true },
    } = &cli.command
    {
        if cli.format.is_some() {
            return fail(
                AppError::new(
                    ErrorType::Validation,
                    "INVALID_ARGUMENT",
                    "--raw and --format cannot be combined.",
                )
                .with_param("format"),
                command,
                trace,
                start,
            );
        }
        let result = read_content(cli.workspace, id.clone(), cli.no_sync).and_then(|content| {
            io::stdout().lock().write_all(&content.bytes).map_err(|_| {
                AppError::new(
                    ErrorType::Io,
                    "OUTPUT_WRITE_FAILED",
                    "Could not write source content.",
                )
            })
        });
        return match result {
            Ok(()) => 0,
            Err(error) => fail(error, command, trace, start),
        };
    }
    let format = cli.format.unwrap_or(OutputFormat::Json);
    if !matches!(format, OutputFormat::Json | OutputFormat::Pretty) {
        return fail(
            AppError::new(
                ErrorType::Validation,
                "UNSUPPORTED_FORMAT",
                "This command supports json and pretty output.",
            )
            .with_hint("Use --format json or --format pretty.")
            .with_param("format"),
            command,
            trace,
            start,
        );
    }
    let (data, workspace_id) = match execute(&cli.command, cli.workspace, cli.no_sync) {
        Ok(data) => data,
        Err(error) => return fail(error, command, trace, start),
    };
    let mut meta = Metadata::new(command, trace.clone(), elapsed_ms(start));
    meta.workspace_id = workspace_id;
    meta.next_cursor = data
        .get("next_cursor")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let envelope = Success::new(data, meta);
    let result = write_json(
        io::stdout().lock(),
        &envelope,
        matches!(format, OutputFormat::Pretty),
    );
    match result {
        Ok(()) => 0,
        Err(_) => fail(
            AppError::new(
                ErrorType::Io,
                "OUTPUT_WRITE_FAILED",
                "Could not write command output.",
            ),
            command,
            trace,
            start,
        ),
    }
}

fn execute(
    command: &Command,
    root: Option<PathBuf>,
    no_sync: bool,
) -> Result<(serde_json::Value, Option<WorkspaceId>), AppError> {
    operations::describe(command.operation_name())?;
    let mut workspace_id = None;
    let result = match command {
        Command::Version => serde_json::to_value(operations::version()),
        Command::Proposal { command } => {
            let workspace = load_workspace(root)?;
            workspace_id = Some(workspace.config.workspace.id.clone());
            Ok(proposal::execute(command, &workspace)?)
        }
        Command::Schema {
            command: SchemaCommand::Patch { op },
        } => serde_json::to_value(knowmesh_core::application::proposal::payload::schema(*op)),
        Command::Search {
            query,
            query_syntax,
            record_types,
            node_types,
            source_ids,
            tags,
            statuses,
            limit,
            cursor,
            include_graph_paths,
            explain,
        } => {
            let workspace = load_workspace(root)?;
            workspace_id = Some(workspace.config.workspace.id.clone());
            serde_json::to_value(search::execute(
                &workspace,
                crate::runtime::open_search_store(&workspace)?.as_mut(),
                &SearchInput {
                    query: query.clone(),
                    query_syntax: *query_syntax,
                    record_types: if record_types.is_empty() {
                        SearchInput::default().record_types
                    } else {
                        record_types.clone()
                    },
                    node_types: node_types.clone(),
                    source_ids: source_ids.clone(),
                    tags: tags.clone(),
                    statuses: statuses.clone(),
                    limit: *limit,
                    cursor: cursor.clone(),
                    include_graph_paths: *include_graph_paths,
                    explain: *explain,
                    no_sync,
                },
            )?)
        }
        Command::Status => {
            let workspace = load_workspace(root)?;
            workspace_id = Some(workspace.config.workspace.id.clone());
            serde_json::to_value(status::get(
                &workspace,
                crate::runtime::open_store(&workspace)?.as_mut(),
                &status::StatusInput { no_sync },
            )?)
        }
        Command::Init {
            path,
            name,
            template,
            dry_run,
        } => {
            let report = workspace::init(&InitInput {
                path: path.clone().or(root).unwrap_or_else(|| PathBuf::from(".")),
                name: name.clone(),
                template: template.clone(),
                dry_run: *dry_run,
            })?;
            workspace_id = Some(report.workspace_id.clone());
            serde_json::to_value(report)
        }
        Command::Schema {
            command: SchemaCommand::List,
        } => serde_json::to_value(operations::descriptors()),
        Command::Schema {
            command: SchemaCommand::Command { operation },
        } => serde_json::to_value(operations::describe(operation)?),
        Command::Schema {
            command: SchemaCommand::Pack { id },
        } => {
            let workspace = load_workspace(root)?;
            workspace_id = Some(workspace.config.workspace.id.clone());
            serde_json::to_value(schema::pack(&workspace, &PackInput { id: id.clone() })?)
        }
        Command::Sync { dry_run } => {
            let workspace = load_workspace(root)?;
            workspace_id = Some(workspace.config.workspace.id.clone());
            let report = if *dry_run {
                sync::preview(&workspace)?
            } else {
                sync::synchronize(&workspace, crate::runtime::open_store(&workspace)?.as_mut())?
            };
            serde_json::to_value(report)
        }
        Command::Source { command } => {
            let workspace = load_workspace(root)?;
            workspace_id = Some(workspace.config.workspace.id.clone());
            match command {
                SourceCommand::List {
                    include_removed,
                    kind,
                    tag,
                    limit,
                    cursor,
                } => serde_json::to_value(source_read::list(
                    &workspace,
                    crate::runtime::open_source_store(&workspace)?.as_mut(),
                    &source_read::ListInput {
                        include_removed: *include_removed,
                        kind: kind.clone(),
                        tag: tag.clone(),
                        limit: *limit,
                        cursor: cursor.clone(),
                        no_sync,
                    },
                )?),
                SourceCommand::Get { source_id } => serde_json::to_value(source_read::get(
                    &workspace,
                    crate::runtime::open_source_store(&workspace)?.as_mut(),
                    &source_read::GetInput {
                        source_id: source_id.clone(),
                        no_sync,
                    },
                )?),
                SourceCommand::Content { id, .. } => serde_json::to_value(
                    source_read::content(
                        &workspace,
                        crate::runtime::open_source_store(&workspace)?.as_mut(),
                        &ContentInput {
                            id: id.clone(),
                            no_sync,
                        },
                    )?
                    .into_report()?,
                ),
                SourceCommand::Add {
                    path,
                    source_id,
                    storage,
                    encoding,
                    title,
                    kind,
                    tags,
                    allow_private_network,
                    dry_run,
                } => {
                    let input = ImportInput {
                        path: path.clone(),
                        source_id: source_id.clone(),
                        storage: storage.map(Into::into),
                        encoding: encoding.clone(),
                        title: title.clone(),
                        kind: kind.clone(),
                        tags: tags.clone(),
                        dry_run: *dry_run,
                    };
                    let imported = knowmesh_core::application::source_fetch::fetch(
                        &workspace,
                        &input,
                        *allow_private_network,
                        &crate::source_fetch::HttpSourceFetcher,
                    )?;
                    let report = if *dry_run {
                        source::preview_add(&workspace, &input, imported)?
                    } else {
                        source::add(
                            &workspace,
                            crate::runtime::open_store(&workspace)?.as_mut(),
                            &input,
                            imported,
                        )?
                    };
                    serde_json::to_value(report)
                }
                SourceCommand::Remove {
                    source_id,
                    dry_run,
                    yes,
                } => {
                    let input = source::RemoveInput {
                        source_id: source_id.clone(),
                        dry_run: *dry_run,
                        yes: *yes,
                    };
                    let report = if *dry_run {
                        source::preview_remove_with_impact(
                            &workspace,
                            &input,
                            &crate::runtime::impact_preview_backend(&workspace)?,
                        )?
                    } else {
                        source::remove(
                            &workspace,
                            crate::runtime::open_store(&workspace)?.as_mut(),
                            &input,
                        )?
                    };
                    serde_json::to_value(report)
                }
                SourceCommand::Impact {
                    source_id,
                    revision,
                    kind,
                    limit,
                    cursor,
                } => serde_json::to_value(impact::execute(
                    &workspace,
                    crate::runtime::open_store(&workspace)?.as_mut(),
                    &ImpactInput {
                        source_id: source_id.clone(),
                        revision: revision.clone(),
                        kind: *kind,
                        limit: *limit,
                        cursor: cursor.clone(),
                        no_sync,
                    },
                )?),
            }
        }
        Command::Rebuild {
            dry_run,
            yes,
            discard_runtime,
            keep_backups,
        } => {
            let workspace = load_workspace(root)?;
            workspace_id = Some(workspace.config.workspace.id.clone());
            let backend = crate::runtime::rebuild_backend(&workspace)?;
            serde_json::to_value(rebuild::execute(
                &workspace,
                &backend,
                &RebuildInput {
                    dry_run: *dry_run,
                    yes: *yes,
                    discard_runtime: *discard_runtime,
                    keep_backups: *keep_backups,
                },
            )?)
        }
        Command::Doctor {
            repair,
            dry_run,
            yes,
        } => {
            let input = RepairInput {
                dry_run: *dry_run,
                yes: *yes,
            };
            if *repair {
                doctor::validate_repair(&input)?;
            }
            let root = workspace_root(root, true)?;
            let store = crate::runtime::inspect_store_at(&root);
            let access = match &store {
                Ok(Some(store)) => IndexAccess::Ready(store.as_ref()),
                Ok(None) => IndexAccess::Missing,
                Err(error) => IndexAccess::Failed(error.clone()),
            };
            let report = if *repair {
                doctor::repair_root(&root, access, &input, |workspace| {
                    Ok(crate::runtime::open_store(workspace)?)
                })?
            } else {
                doctor::inspect_root(&root, access)?
            };
            workspace_id = report.workspace_id.clone();
            serde_json::to_value(report)
        }
    };
    result.map(|data| (data, workspace_id)).map_err(|_| {
        AppError::new(
            ErrorType::Internal,
            "ENCODE_FAILED",
            "Could not encode the operation result.",
        )
    })
}

fn load_workspace(root: Option<PathBuf>) -> Result<Workspace, AppError> {
    Workspace::load(&workspace_root(root, false)?)
}

fn read_content(
    root: Option<PathBuf>,
    id: ContentId,
    no_sync: bool,
) -> Result<source_read::SourceContent, AppError> {
    let workspace = load_workspace(root)?;
    source_read::content(
        &workspace,
        crate::runtime::open_source_store(&workspace)?.as_mut(),
        &ContentInput { id, no_sync },
    )
}

fn workspace_root(root: Option<PathBuf>, recovery: bool) -> Result<PathBuf, AppError> {
    let environment = std::env::var_os("KNOWMESH_WORKSPACE").map(PathBuf::from);
    let cwd = std::env::current_dir().map_err(|_| {
        AppError::new(
            ErrorType::Io,
            "CURRENT_DIRECTORY_UNAVAILABLE",
            "Cannot resolve the current directory.",
        )
    })?;
    let resolve = if recovery {
        doctor::resolve_root
    } else {
        knowmesh_core::canonical::workspace::resolve_workspace
    };
    resolve(root.as_deref(), environment.as_deref(), &cwd)
}

fn fail(error: AppError, command: &str, trace: RunId, start: Instant) -> u8 {
    let code = error.exit_code();
    let envelope = Failure::new(error, Metadata::new(command, trace, elapsed_ms(start)));
    let _ = write_json(io::stderr().lock(), &envelope, false);
    code
}

fn elapsed_ms(start: Instant) -> u64 {
    start.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

fn write_json(mut writer: impl Write, value: &impl Serialize, pretty: bool) -> io::Result<()> {
    if pretty {
        serde_json::to_writer_pretty(&mut writer, value)?;
    } else {
        serde_json::to_writer(&mut writer, value)?;
    }
    writer.write_all(b"\n")?;
    writer.flush()
}
