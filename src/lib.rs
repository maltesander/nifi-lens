//! nifi-lens — a keyboard-driven terminal UI for observing and debugging
//! Apache NiFi 2.x clusters.
//!
//! Read-only and multi-cluster (kubeconfig-style context switching), powered
//! by [`nifi-rust-client`](https://docs.rs/nifi-rust-client) with the
//! `dynamic` feature so one binary works against every supported NiFi version.

pub mod app;
pub mod bytes;
pub mod cli;
pub mod client;
pub mod cluster;
pub mod config;
pub mod error;
pub mod event;
pub mod input;
pub mod intent;
pub mod layout;
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
            println!("nifi-rust-client {}", env!("NIFI_RUST_CLIENT_VERSION"));
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
            let (config, resolved) = match config::loader::load(&args) {
                Ok(v) => v,
                Err(NifiLensError::ConfigMissing { path }) => {
                    match config::init::try_bootstrap(&path, args.config.is_some())? {
                        Some(msg) => {
                            eprintln!("no config file at {}", msg.missing_path.display());
                            eprintln!("wrote template to {}", msg.written_path.display());
                            eprintln!(
                                "edit it with your cluster URL and credentials, then re-run nifilens"
                            );
                            return Ok(ExitCode::SUCCESS);
                        }
                        None => return Err(NifiLensError::ConfigMissing { path }),
                    }
                }
                Err(e) => return Err(e),
            };
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|source| NifiLensError::Io { source })?;
            rt.block_on(async move {
                let client = client::NifiClient::connect(&resolved).await?;
                app::run(client, config, stderr_toggle.clone()).await?;
                Ok::<(), NifiLensError>(())
            })?;
            Ok(ExitCode::SUCCESS)
        }
    }
}
