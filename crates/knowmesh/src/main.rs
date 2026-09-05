mod cli;
mod runtime;

fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(cli::run())
}
