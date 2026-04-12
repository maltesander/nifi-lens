//! nifilens-fixture-seeder entry point.

mod cleanup;
mod cli;
mod entities;
mod error;
mod fixture;
mod marker;
mod state;

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

/// Returns Ok(()) if the context name is allowed for seeding. Production
/// context names must not be accepted by the seeder even if the user
/// somehow points it at a real cluster.
fn check_context_allowlisted(context: &str) -> Result<(), SeederError> {
    if context.starts_with("dev-nifi-") || context.starts_with("test-nifi-") {
        Ok(())
    } else {
        Err(SeederError::ContextNotAllowlisted {
            context: context.to_string(),
        })
    }
}

async fn run(args: Args) -> Result<(), SeederError> {
    tracing::info!(
        config = %args.config.display(),
        context = %args.context,
        skip_if_seeded = args.skip_if_seeded,
        "nifilens-fixture-seeder starting",
    );

    check_context_allowlisted(&args.context)?;

    // Construct a minimal nifi-lens Args so we can reuse its config loader
    // verbatim — same parsing, same env-var resolution, same error types.
    let lens_args = nifi_lens::cli::Args {
        config: Some(args.config.clone()),
        context: Some(args.context.clone()),
        debug: false,
        log_level: None,
        no_color: true,
        allow_writes: false,
        command: None,
    };
    let (_config, resolved) =
        nifi_lens::config::loader::load(&lens_args).map_err(|e| SeederError::ConfigLoad {
            path: args.config.clone(),
            source: Box::new(e),
        })?;

    tracing::info!(
        context = %resolved.name,
        url = %resolved.url,
        "connecting to NiFi",
    );

    let client = nifi_lens::client::NifiClient::connect(&resolved)
        .await
        .map_err(|e| SeederError::Connect {
            context: resolved.name.clone(),
            source: Box::new(e),
        })?;

    tracing::info!(
        version = %client.detected_version(),
        "connected successfully"
    );

    if args.skip_if_seeded
        && let Some(id) = marker::find_marker(&client).await?
    {
        tracing::info!(%id, "fixture marker already present; exiting early");
        return Ok(());
    }

    cleanup::nuke_and_repave(&client).await?;
    fixture::seed(&client, client.detected_version()).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_context_allowed() {
        assert!(check_context_allowlisted("dev-nifi-2-6-0").is_ok());
    }

    #[test]
    fn test_context_allowed() {
        assert!(check_context_allowlisted("test-nifi-2-8-0").is_ok());
    }

    #[test]
    fn prod_context_rejected() {
        assert!(check_context_allowlisted("prod-east").is_err());
        assert!(check_context_allowlisted("nifi-prod").is_err());
    }
}
