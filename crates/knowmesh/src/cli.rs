use std::{
    io::{self, Write},
    path::PathBuf,
    time::Instant,
};

use clap::{Parser, Subcommand, ValueEnum, error::ErrorKind};
use knowmesh_core::{
    application::{
        doctor::{self, IndexAccess, RepairInput},
        operations,
        schema::{self, PackInput},
        source, status, sync,
        workspace::{self, InitInput},
    },
    canonical::{source::ImportInput, workspace::Workspace},
    domain::{RunId, SourceId, StorageMode, WorkspaceId},
    error::{AppError, ErrorType},
    wire::{Failure, Metadata, Success},
};
use serde::Serialize;

#[derive(Parser)]
#[command(
    name = "knowmesh",
    about = "Evidence-backed knowledge workspace",
    disable_help_subcommand = true
)]
struct Cli {
    #[arg(long, global = true)]
    workspace: Option<PathBuf>,
    #[arg(long, global = true, value_enum, default_value = "json")]
    format: OutputFormat,
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
    Status,
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
    Add {
        path: PathBuf,
        #[arg(long)]
        source_id: Option<SourceId>,
        #[arg(long, value_enum)]
        storage: Option<StorageArg>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, default_value = "document")]
        kind: String,
        #[arg(long = "tag")]
        tags: Vec<String>,
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
    Command { operation: String },
    Pack { id: String },
}

impl Command {
    fn operation_name(&self) -> &'static str {
        match self {
            Self::Version => "version",
            Self::Status => "status",
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
            Self::Doctor { repair: true, .. } => "doctor.repair",
            Self::Doctor { .. } => "doctor",
            Self::Source {
                command: SourceCommand::Add { .. },
            } => "source.add",
            Self::Source {
                command: SourceCommand::Remove { .. },
            } => "source.remove",
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
    if !matches!(cli.format, OutputFormat::Json | OutputFormat::Pretty) {
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
    let envelope = Success::new(data, meta);
    let result = write_json(
        io::stdout().lock(),
        &envelope,
        matches!(cli.format, OutputFormat::Pretty),
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
        Command::Status => {
            let workspace = load_workspace(root)?;
            workspace_id = Some(workspace.config.workspace.id.clone());
            serde_json::to_value(status::get(
                &workspace,
                &mut crate::runtime::open_store(&workspace)?,
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
                sync::synchronize(&workspace, &mut crate::runtime::open_store(&workspace)?)?
            };
            serde_json::to_value(report)
        }
        Command::Source { command } => {
            let workspace = load_workspace(root)?;
            workspace_id = Some(workspace.config.workspace.id.clone());
            let report = match command {
                SourceCommand::Add {
                    path,
                    source_id,
                    storage,
                    title,
                    kind,
                    tags,
                    dry_run,
                } => {
                    let input = ImportInput {
                        path: path.clone(),
                        source_id: source_id.clone(),
                        storage: storage.map(Into::into),
                        title: title.clone(),
                        kind: kind.clone(),
                        tags: tags.clone(),
                        dry_run: *dry_run,
                    };
                    if *dry_run {
                        source::preview_add(&workspace, &input, None)?
                    } else {
                        source::add(
                            &workspace,
                            &mut crate::runtime::open_store(&workspace)?,
                            &input,
                            None,
                        )?
                    }
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
                    if *dry_run {
                        source::preview_remove(&workspace, &input)?
                    } else {
                        source::remove(
                            &workspace,
                            &mut crate::runtime::open_store(&workspace)?,
                            &input,
                        )?
                    }
                }
            };
            serde_json::to_value(report)
        }
        Command::Doctor {
            repair,
            dry_run,
            yes,
        } => {
            let workspace = load_workspace(root)?;
            workspace_id = Some(workspace.config.workspace.id.clone());
            let input = RepairInput {
                dry_run: *dry_run,
                yes: *yes,
            };
            if *repair {
                doctor::validate_repair(&input)?;
            }
            let report = if *repair && !*dry_run {
                doctor::repair(
                    &workspace,
                    &mut crate::runtime::open_store(&workspace)?,
                    &input,
                )?
            } else {
                let store = crate::runtime::inspect_store(&workspace);
                let access = match &store {
                    Ok(Some(store)) => IndexAccess::Ready(store),
                    Ok(None) => IndexAccess::Missing,
                    Err(error) => IndexAccess::Failed(error.clone()),
                };
                if *dry_run {
                    doctor::preview_repair(&workspace, access)?
                } else {
                    doctor::inspect(&workspace, access)?
                }
            };
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
    let environment = std::env::var_os("KNOWMESH_WORKSPACE").map(PathBuf::from);
    let cwd = std::env::current_dir().map_err(|_| {
        AppError::new(
            ErrorType::Io,
            "CURRENT_DIRECTORY_UNAVAILABLE",
            "Cannot resolve the current directory.",
        )
    })?;
    workspace::load(root.as_deref(), environment.as_deref(), &cwd)
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
