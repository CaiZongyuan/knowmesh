mod cli;
mod runtime;
mod source_fetch;

fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(cli::run())
}
