//! nifilens-fixture-seeder entry point.

mod cli;
mod error;

use clap::Parser as _;
use tracing_subscriber::EnvFilter;

use crate::cli::Args;
use crate::error::SeederError;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args = Args::parse();
    init_tracing(&args.log_level);

    match run(args).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(error = %e, "seeder failed");
            std::process::ExitCode::from(1)
        }
    }
}

fn init_tracing(level: &str) {
    let filter = EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

async fn run(args: Args) -> Result<(), SeederError> {
    tracing::info!(
        config = %args.config.display(),
        context = %args.context,
        skip_if_seeded = args.skip_if_seeded,
        "nifilens-fixture-seeder starting",
    );
    // Task 8 wires in config loading and connect; for now we just return Ok.
    Ok(())
}
