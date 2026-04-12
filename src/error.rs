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

    #[snafu(display("config is invalid: {detail}"))]
    ConfigInvalid { detail: String },

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

    /// The detected NiFi version is not supported by the current
    /// `nifi-rust-client` build, and `version_strategy = "strict"` refuses
    /// to fall back.
    #[snafu(display(
        "context {context:?}: NiFi {detected} is not supported by nifi-rust-client\n\
         \n\
         hint: set `version_strategy = \"closest\"` (or `\"latest\"`) in the context\n\
         config to fall back to the nearest supported version."
    ))]
    UnsupportedNifiVersion { context: String, detected: String },

    /// TLS certificate verification failed — usually a self-signed cluster
    /// without a trust anchor configured.
    #[snafu(display(
        "context {context:?}: TLS certificate verification failed: {source}\n\
         \n\
         hints:\n\
         - set `ca_cert_path = \"/path/to/ca.crt\"` to a PEM containing the cluster's CA\n\
         - or set `insecure_tls = true` to skip verification entirely (dev only)"
    ))]
    TlsCertInvalid {
        context: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// NiFi rejected the credentials (401 / auth failure).
    #[snafu(display(
        "context {context:?}: NiFi rejected the credentials\n\
         \n\
         hints:\n\
         - double-check `username` in the config\n\
         - verify the password: either the `password` field, or the environment\n\
           variable named by `password_env`"
    ))]
    NifiUnauthorized { context: String },

    /// `nifi-rust-client` 0.5.0's error type is not yet audited (see Phase 0
    /// Task 7). We box the source as a trait object so this variant compiles
    /// before that audit, at the cost of losing snafu's automatic `From`
    /// conversion — call sites must box the source explicitly.
    #[snafu(display("failed to build nifi-rust-client for context {context:?}: {source}"))]
    ClientBuildFailed {
        context: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale; callers must
    /// box explicitly.
    #[snafu(display("login failed for context {context:?}: {source}"))]
    LoginFailed {
        context: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// `DynamicClient::detected_version()` returned `None` after a successful
    /// `login()`. This is a library invariant violation in practice — the
    /// `Option` exists because version detection is lazy — but nifi-lens
    /// relies on the post-login guarantee that detection has already run.
    #[snafu(display(
        "context {context:?}: NiFi version was not detected after login; \
         this indicates a nifi-rust-client invariant violation"
    ))]
    VersionDetectionMissing { context: String },

    /// See `ClientBuildFailed` for the boxed-source rationale; callers must
    /// box explicitly.
    #[snafu(display("failed to fetch /flow/about for context {context:?}: {source}"))]
    AboutFailed {
        context: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale; callers must
    /// box explicitly.
    #[snafu(display("failed to fetch controller status for context {context:?}: {source}"))]
    ControllerStatusFailed {
        context: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale; callers must
    /// box explicitly.
    #[snafu(display("failed to fetch process-group status for context {context:?}: {source}"))]
    ProcessGroupStatusFailed {
        context: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale; callers must
    /// box explicitly.
    #[snafu(display("failed to fetch bulletin board for context {context:?}: {source}"))]
    BulletinBoardFailed {
        context: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale; callers must
    /// box explicitly.
    #[snafu(display("failed to fetch browser tree for context {context:?}: {source}"))]
    BrowserTreeFailed {
        context: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale; callers
    /// must box explicitly.
    #[snafu(display(
        "failed to fetch process-group {id:?} detail for context {context:?}: {source}"
    ))]
    ProcessGroupDetailFailed {
        context: String,
        id: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale; callers
    /// must box explicitly.
    #[snafu(display(
        "failed to fetch controller services for PG {id:?} for context {context:?}: {source}"
    ))]
    ControllerServicesListFailed {
        context: String,
        id: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale; callers
    /// must box explicitly.
    #[snafu(display("failed to fetch processor {id:?} detail for context {context:?}: {source}"))]
    ProcessorDetailFailed {
        context: String,
        id: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale; callers
    /// must box explicitly.
    #[snafu(display("failed to fetch connection {id:?} detail for context {context:?}: {source}"))]
    ConnectionDetailFailed {
        context: String,
        id: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale; callers
    /// must box explicitly.
    #[snafu(display(
        "failed to fetch controller service {id:?} detail for context {context:?}: {source}"
    ))]
    ControllerServiceDetailFailed {
        context: String,
        id: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[snafu(display("write intents are disabled in Phase 0 (intent: {intent_name})"))]
    WriteIntentRefused { intent_name: &'static str },

    #[snafu(display("failed to initialize the terminal: {source}"))]
    TerminalInit { source: std::io::Error },

    /// Boxed because `tracing-subscriber`'s reload / init error types are
    /// not uniformly `'static` across versions. No context field because
    /// logging is global, not per-cluster.
    #[snafu(display("failed to initialize logging: {source}"))]
    LoggingInit {
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Last-resort fallback for I/O errors that do not have a more specific
    /// variant (`ConfigWriteFailed`, `CaCertReadFailed`, `TerminalInit`, …).
    /// Prefer a specific variant when one exists so the user sees the
    /// operation context.
    #[snafu(display("I/O error: {source}"))]
    Io { source: std::io::Error },
}
