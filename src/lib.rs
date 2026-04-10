//! nifi-lens — a keyboard-driven TUI lens into Apache NiFi 2.x.
//!
//! The library crate holds every module except the `main` entry point.
//! Integration tests can `use nifi_lens::...` without spawning a binary.

pub mod cli;
pub mod error;
pub mod logging;

pub use error::NifiLensError;

use clap::Parser;
use std::process::ExitCode;

/// Run nifi-lens. The binary calls this and maps the result to a process
/// exit code; integration tests can call into the library directly without
/// going through `main`.
pub fn run() -> ExitCode {
    let args = cli::Args::parse();
    match run_inner(args) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run_inner(args: cli::Args) -> Result<ExitCode, NifiLensError> {
    if args.allow_writes {
        return Err(NifiLensError::WritesNotImplemented);
    }

    // Initialize logging. The WorkerGuard must stay alive for the whole run.
    let (_log_guard, _stderr_toggle) = logging::init(&args)?;

    match args.command {
        Some(cli::Command::Version) => {
            println!("nifilens {}", env!("CARGO_PKG_VERSION"));
            println!("nifi-rust-client 0.5.0");
            Ok(ExitCode::SUCCESS)
        }
        Some(cli::Command::Config { .. }) => {
            // Filled in by Tasks 5 and 6.
            eprintln!("error: config subcommands not yet implemented");
            Ok(ExitCode::FAILURE)
        }
        None => {
            // Filled in by Task 12.
            println!("nifilens {}", env!("CARGO_PKG_VERSION"));
            println!("(TUI not yet wired — see Phase 0 Task 12)");
            Ok(ExitCode::SUCCESS)
        }
    }
}
