//! SeederError — snafu-derived top-level error type for the seeder.

use std::path::PathBuf;

use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum SeederError {
    #[snafu(display("failed to load config at {}: {source}", path.display()))]
    ConfigLoad {
        path: PathBuf,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[snafu(display(
        "refused to seed context {context:?}: name does not match allowlist \
         (must start with 'dev-nifi-' or 'test-nifi-')"
    ))]
    ContextNotAllowlisted { context: String },

    #[snafu(display("failed to connect to NiFi for context {context}: {source}"))]
    Connect {
        context: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[snafu(display("NiFi API call failed: {message}: {source}"))]
    Api {
        message: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[snafu(display("timed out after {elapsed_secs}s waiting for {what} to reach {target_state}"))]
    StateTimeout {
        what: String,
        target_state: String,
        elapsed_secs: u64,
    },

    #[snafu(display("fixture invariant violated: {message}"))]
    Invariant { message: String },
}

pub type Result<T, E = SeederError> = std::result::Result<T, E>;
