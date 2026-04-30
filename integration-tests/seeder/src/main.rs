//! nifilens-fixture-seeder entry point.

mod cleanup;
mod cli;
mod entities;
mod error;
mod fixture;
mod marker;
mod state;

use std::time::{Duration, Instant};

use clap::Parser as _;
use nifi_rust_client::NifiError;
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

    wait_for_cluster_ready(&client).await?;

    if args.skip_if_seeded
        && let Some(id) = marker::find_marker(&client).await?
    {
        tracing::info!(%id, "fixture marker already present; exiting early");
        return Ok(());
    }

    cleanup::nuke_and_repave(&client).await?;
    fixture::seed(&client, client.detected_version(), args.break_after).await?;

    Ok(())
}

/// Polls `/flow/cluster/summary` until the deployment is fully ready.
///
/// Earlier iterations of this gate tried to whack-a-mole the various
/// transient 4xx/5xx responses NiFi emits during cluster startup —
/// "is initializing", "Cannot replicate", "no nodes are connected" —
/// each on a different status code. `/flow/cluster/summary` collapses
/// that into one deterministic signal:
///
///   - standalone (`clustered: false`)             → ready immediately
///   - cluster fully formed (`connected == total`) → ready
///   - cluster forming (counts mismatch)           → wait
///   - FC still initializing (409)                 → wait (transient)
///
/// Other errors (auth, network, unexpected status) propagate immediately.
async fn wait_for_cluster_ready(client: &nifi_lens::client::NifiClient) -> Result<(), SeederError> {
    const TIMEOUT: Duration = Duration::from_secs(180);
    const POLL: Duration = Duration::from_secs(2);

    let started = Instant::now();
    let mut announced = false;
    let mut last_announce = String::new();

    loop {
        let elapsed = started.elapsed();
        match client.flow().get_cluster_summary().await {
            Ok(dto) => {
                let clustered = dto.clustered.unwrap_or(false);
                if !clustered {
                    if announced {
                        tracing::info!(elapsed_secs = elapsed.as_secs(), "NiFi ready");
                    }
                    return Ok(());
                }
                let connected = dto.connected_node_count.unwrap_or(0);
                let total = dto.total_node_count.unwrap_or(0);
                if total > 0 && connected == total {
                    if announced {
                        tracing::info!(
                            elapsed_secs = elapsed.as_secs(),
                            connected,
                            total,
                            "cluster fully formed",
                        );
                    }
                    return Ok(());
                }
                let reason = format!("cluster forming: {connected}/{total} nodes connected");
                if reason != last_announce {
                    tracing::info!("{reason}; waiting...");
                    last_announce = reason;
                    announced = true;
                }
            }
            Err(NifiError::Conflict { message }) => {
                if message.as_str() != last_announce {
                    tracing::info!(reason = %message, "cluster not ready; waiting...");
                    last_announce = message;
                    announced = true;
                }
            }
            Err(other) => {
                return Err(SeederError::Api {
                    message: "probing /flow/cluster/summary".into(),
                    source: Box::new(other),
                });
            }
        }

        if started.elapsed() >= TIMEOUT {
            return Err(SeederError::Api {
                message: format!(
                    "timed out after {}s waiting for cluster readiness",
                    TIMEOUT.as_secs()
                ),
                source: Box::new(std::io::Error::other(last_announce)),
            });
        }
        tokio::time::sleep(POLL).await;
    }
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
