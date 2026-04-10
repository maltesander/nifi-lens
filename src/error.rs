//! Top-level error type for nifi-lens.
//!
//! Sub-modules (config, client, intent) define their own snafu error types
//! that roll up into this enum via `#[snafu(source)]` variants. The goal is
//! to present one error type at the application edge while letting each
//! module keep its own local context selectors.

use std::path::PathBuf;

use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum NifiLensError {
    #[snafu(display(
        "write mode is not implemented in Phase 0; --allow-writes is reserved for v2"
    ))]
    WritesNotImplemented,

    #[snafu(display("no config file at {}; run `nifilens config init` to create a template", path.display()))]
    ConfigMissing { path: PathBuf },

    #[snafu(display("config file {} is world-readable; refusing to load (chmod 0600 and retry)", path.display()))]
    ConfigWorldReadable { path: PathBuf },

    #[snafu(display("failed to parse config at {}: {source}", path.display()))]
    ConfigParse {
        path: PathBuf,
        source: toml::de::Error,
    },

    #[snafu(display("config at {} already exists; pass --force to overwrite", path.display()))]
    ConfigAlreadyExists { path: PathBuf },

    #[snafu(display("failed to write config template at {}: {source}", path.display()))]
    ConfigWriteFailed {
        path: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display("unknown context {name:?} (available: {available:?})"))]
    UnknownContext {
        name: String,
        available: Vec<String>,
    },

    #[snafu(display(
        "context {context:?} uses password_env {var:?} but the environment variable is not set"
    ))]
    MissingPasswordEnv { context: String, var: String },

    #[snafu(display("CA cert file not found at {}", path.display()))]
    CaCertNotFound { path: PathBuf },

    #[snafu(display("failed to read CA cert at {}: {source}", path.display()))]
    CaCertReadFailed {
        path: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display("failed to build nifi-rust-client for context {context:?}: {source}"))]
    ClientBuildFailed {
        context: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[snafu(display("login failed for context {context:?}: {source}"))]
    LoginFailed {
        context: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[snafu(display("failed to fetch /flow/about for context {context:?}: {source}"))]
    AboutFailed {
        context: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[snafu(display("write intents are disabled in Phase 0 (intent: {intent_name})"))]
    WritesNotAllowed { intent_name: &'static str },

    #[snafu(display("failed to initialize the terminal: {source}"))]
    TerminalInit { source: std::io::Error },

    #[snafu(display("failed to initialize logging: {source}"))]
    LoggingInit {
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[snafu(display("I/O error: {source}"))]
    Io { source: std::io::Error },
}
