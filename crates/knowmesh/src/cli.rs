use std::{
    io::{self, Write},
    path::PathBuf,
    time::Instant,
};

use clap::{Parser, Subcommand, ValueEnum, error::ErrorKind};
use knowmesh_core::{
    application::{
        operations,
        workspace::{self, InitInput},
    },
    domain::{RunId, WorkspaceId},
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
}

#[derive(Subcommand)]
enum SchemaCommand {
    List,
    Command { operation: String },
}

impl Command {
    fn operation_name(&self) -> &'static str {
        match self {
            Self::Version => "version",
            Self::Init { .. } => "init",
            Self::Schema {
                command: SchemaCommand::List,
            } => "schema.list",
            Self::Schema {
                command: SchemaCommand::Command { .. },
            } => "schema.command",
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
    let (data, workspace_id) = match execute(&cli.command, cli.workspace) {
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
) -> Result<(serde_json::Value, Option<WorkspaceId>), AppError> {
    operations::describe(command.operation_name())?;
    let mut workspace_id = None;
    let result = match command {
        Command::Version => serde_json::to_value(operations::version()),
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
    };
    result.map(|data| (data, workspace_id)).map_err(|_| {
        AppError::new(
            ErrorType::Internal,
            "ENCODE_FAILED",
            "Could not encode the operation result.",
        )
    })
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
