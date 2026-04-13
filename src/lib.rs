//! nifi-lens — a keyboard-driven terminal UI for observing and debugging
//! Apache NiFi 2.x clusters.
//!
//! Read-only and multi-cluster (kubeconfig-style context switching), powered
//! by [`nifi-rust-client`](https://docs.rs/nifi-rust-client) with the
//! `dynamic` feature so one binary works against every supported NiFi version.

pub mod app;
pub mod cli;
pub mod client;
pub mod config;
pub mod error;
pub mod event;
pub mod intent;
pub mod logging;
pub mod theme;
pub mod timestamp;
pub mod view;
pub mod widget;

#[cfg(test)]
mod test_support;

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
    // writer. `stderr_toggle` is shared with the TUI run loop, TerminalGuard,
    // and the panic hook via Arc-based Clone.
    let (log_guard, stderr_toggle) = logging::init(&args)?;
    let _ = &log_guard; // keep log_guard alive for the duration of run_inner

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
            let (config, resolved) = config::loader::load(&args)?;
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|source| NifiLensError::Io { source })?;
            let local = tokio::task::LocalSet::new();
            rt.block_on(local.run_until(async move {
                let client = client::NifiClient::connect(&resolved).await?;
                app::run(client, config, stderr_toggle.clone()).await?;
                Ok::<(), NifiLensError>(())
            }))?;
            Ok(ExitCode::SUCCESS)
        }
    }
}
