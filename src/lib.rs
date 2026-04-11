//! nifi-lens — a keyboard-driven TUI lens into Apache NiFi 2.x.
//!
//! The library crate holds every module except the `main` entry point.
//! Integration tests can `use nifi_lens::...` without spawning a binary.

pub mod cli;
pub mod client;
pub mod config;
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
    // `log_guard` intentionally held here — dropping it flushes the async log
    // writer. `_stderr_toggle` is consumed by Task 12's TUI run loop.
    let (log_guard, _stderr_toggle) = logging::init(&args)?;
    // Keep `log_guard` alive for the whole `run_inner` scope.
    let _ = &log_guard;

    match args.command {
        Some(cli::Command::Version) => {
            println!("nifilens {}", env!("CARGO_PKG_VERSION"));
            println!("nifi-rust-client 0.5.0");
            Ok(ExitCode::SUCCESS)
        }
        Some(cli::Command::Config { ref action }) => match action {
            cli::ConfigAction::Init { force } => {
                let path = config::init::write_template(*force)?;
                eprintln!("wrote template to {}", path.display());
                Ok(ExitCode::SUCCESS)
            }
            cli::ConfigAction::Validate => {
                let (config, resolved) = config::loader::load(&args)?;
                eprintln!(
                    "OK — {} context(s), active: {}",
                    config.contexts.len(),
                    resolved.name
                );
                Ok(ExitCode::SUCCESS)
            }
        },
        None => {
            // Filled in by Task 12.
            println!("nifilens {}", env!("CARGO_PKG_VERSION"));
            println!("(TUI not yet wired — see Phase 0 Task 12)");
            Ok(ExitCode::SUCCESS)
        }
    }
}
