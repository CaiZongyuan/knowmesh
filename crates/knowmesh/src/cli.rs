use std::{
    io::{self, Write},
    path::PathBuf,
    time::Instant,
};

use clap::{Parser, Subcommand, ValueEnum, error::ErrorKind};
use knowmesh_core::{
    domain::RunId,
    error::{AppError, ErrorType},
    wire::{API_CONTRACT_VERSION, Failure, Metadata, Success},
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
}

#[derive(Serialize)]
struct VersionInfo {
    version: &'static str,
    api_contract_version: &'static str,
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
    let command = match cli.command {
        Command::Version => "version",
    };
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
    let data = VersionInfo {
        version: env!("CARGO_PKG_VERSION"),
        api_contract_version: API_CONTRACT_VERSION,
    };
    let envelope = Success::new(
        data,
        Metadata::new(command, trace.clone(), elapsed_ms(start)),
    );
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
