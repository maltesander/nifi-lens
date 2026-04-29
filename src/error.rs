//! Top-level error type for nifi-lens.
//!
//! Sub-modules (config, client, intent) define their own snafu error types
//! that roll up into this enum via `#[snafu(source)]` variants. The goal is
//! to present one error type at the application edge while letting each
//! module keep its own local context selectors.

use std::path::PathBuf;

use snafu::Snafu;

/// Top-level error type surfaced at the application edge.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum NifiLensError {
    #[snafu(display("write mode is not implemented; --allow-writes is reserved for v2"))]
    WritesNotImplemented,

    /// 404 from NiFi for `/flow/{type}/{id}/status/history`. The
    /// sparkline worker maps this onto `AppEvent::SparklineEndpointMissing`
    /// instead of warn-logging — the endpoint is genuinely absent for
    /// some component shapes, and the renderer shows the muted
    /// "no history yet" state.
    #[snafu(display("status_history endpoint missing for component {id:?} (NiFi 404): {source}"))]
    SparklineEndpointMissing {
        id: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Non-404 fetch failure for the status-history endpoint. The
    /// worker logs at `warn!` and continues looping; this variant only
    /// exists so the error type round-trips through the worker without
    /// introducing a Box-dyn-Error variant of its own.
    #[snafu(display("failed to fetch status_history for component {id:?}: {source}"))]
    StatusHistoryFetchFailed {
        id: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Two-phase listing request for a queue failed at submission time.
    #[snafu(display("failed to submit listing request for queue {queue_id}: {source}"))]
    QueueListingFailed {
        queue_id: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Polling an in-flight listing request failed.
    #[snafu(display("failed to poll listing request {request_id} for queue {queue_id}: {source}"))]
    QueueListingPollFailed {
        queue_id: String,
        request_id: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Fetching a single flowfile's metadata from a queue failed.
    #[snafu(display("failed to fetch flowfile {flowfile_uuid} from queue {queue_id}: {source}"))]
    FlowfilePeekFailed {
        queue_id: String,
        flowfile_uuid: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Component kind that has no `/status/history` endpoint
    /// (controller services and ports). The dispatcher returns this so
    /// callers don't have to redo the kind check before calling.
    #[snafu(display("sparkline not available for component kind {kind}"))]
    SparklineUnsupportedKind { kind: String },

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

    /// An auth env var (password_env or token_env) is not set.
    #[snafu(display("context {context:?}: auth uses env var {var:?} but it is not set"))]
    MissingAuthEnvVar { context: String, var: String },

    #[snafu(display("CA cert file not found at {}", path.display()))]
    CaCertNotFound { path: PathBuf },

    #[snafu(display("failed to read CA cert at {}: {source}", path.display()))]
    CaCertReadFailed {
        path: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display("client identity file not found at {}", path.display()))]
    ClientIdentityNotFound { path: PathBuf },

    #[snafu(display("failed to read client identity at {}: {source}", path.display()))]
    ClientIdentityReadFailed {
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
         - check the [contexts.auth] section in your config\n\
         - for type=\"password\": verify username and password/password_env\n\
         - for type=\"token\": verify the JWT is valid and not expired\n\
         - for type=\"mtls\": verify the client identity PEM"
    ))]
    NifiUnauthorized { context: String },

    /// `nifi-rust-client` 0.5.0's error type is not yet audited. We box
    /// the source as a trait object so this variant compiles before that
    /// audit, at the cost of losing snafu's automatic `From` conversion —
    /// call sites must box the source explicitly.
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

    /// See `ClientBuildFailed` for the boxed-source rationale.
    #[snafu(display(
        "failed to fetch latest provenance events for component {component_id:?} \
         in context {context:?}: {source}"
    ))]
    LatestProvenanceEventsFailed {
        context: String,
        component_id: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale.
    #[snafu(display(
        "failed to submit lineage query for flowfile {uuid:?} in context {context:?}: {source}"
    ))]
    LineageQuerySubmitFailed {
        context: String,
        uuid: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale.
    #[snafu(display("failed to poll lineage query {query_id:?} in context {context:?}: {source}"))]
    LineageQueryPollFailed {
        context: String,
        query_id: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale. Delete
    /// failures are logged at warn level and never surfaced to the user.
    #[snafu(display(
        "failed to delete lineage query {query_id:?} in context {context:?}: {source}"
    ))]
    LineageQueryDeleteFailed {
        context: String,
        query_id: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale.
    #[snafu(display("failed to submit provenance query in context {context:?}: {source}"))]
    ProvenanceQuerySubmitFailed {
        context: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale.
    #[snafu(display(
        "failed to poll provenance query {query_id:?} in context {context:?}: {source}"
    ))]
    ProvenanceQueryPollFailed {
        context: String,
        query_id: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale. Delete
    /// failures are logged at warn level and never surfaced to the user.
    #[snafu(display(
        "failed to delete provenance query {query_id:?} in context {context:?}: {source}"
    ))]
    ProvenanceQueryDeleteFailed {
        context: String,
        query_id: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale.
    #[snafu(display(
        "failed to fetch provenance event {event_id} in context {context:?}: {source}"
    ))]
    ProvenanceEventFetchFailed {
        context: String,
        event_id: i64,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale.
    #[snafu(display(
        "failed to fetch {side} content for provenance event {event_id} \
         in context {context:?}: {source}"
    ))]
    ProvenanceContentFetchFailed {
        context: String,
        event_id: i64,
        side: &'static str,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[snafu(display("failed to write provenance content to {}: {source}", path.display()))]
    ContentSaveFailed {
        path: PathBuf,
        source: std::io::Error,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale.
    #[snafu(display("failed to fetch system diagnostics for context {context:?}: {source}"))]
    SystemDiagnosticsFailed {
        context: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale; callers
    /// must box explicitly. Standalone NiFi servers return HTTP 409 for
    /// `/controller/cluster` — the fetcher recognizes that shape and
    /// does not produce a `ClusterNodesFailed`; only true errors end up
    /// here.
    #[snafu(display("failed to fetch cluster nodes for context {context:?}: {source}"))]
    ClusterNodesFailed {
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
    /// must box explicitly. Raised per-PG by the cluster store's
    /// connections-by-PG fetcher when `/process-groups/{id}/connections`
    /// fails; per-PG errors are non-fatal — the snapshot's
    /// `EndpointState::Failed` arm preserves any prior `last_ok`.
    #[snafu(display(
        "failed to fetch connections for PG {id:?} for context {context:?}: {source}"
    ))]
    PgConnectionsFetchFailed {
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

    /// See `ClientBuildFailed` for the boxed-source rationale; callers
    /// must box explicitly.
    #[snafu(display(
        "failed to fetch remote process group {id:?} detail for context {context:?}: {source}"
    ))]
    RemoteProcessGroupDetailFailed {
        context: String,
        id: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale; callers
    /// must box explicitly.
    #[snafu(display(
        "failed to fetch version-control information for PG {pg_id:?} in context {context:?}: {source}"
    ))]
    VersionInformationFailed {
        context: String,
        pg_id: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale; callers
    /// must box explicitly.
    #[snafu(display(
        "failed to fetch local modifications for PG {pg_id:?} in context {context:?}: {source}"
    ))]
    LocalModificationsFailed {
        context: String,
        pg_id: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// See `ClientBuildFailed` for the boxed-source rationale; callers
    /// must box explicitly.
    #[snafu(display(
        "failed to fetch {kind} port {id:?} detail for context {context:?}: {source}"
    ))]
    PortDetailFailed {
        context: String,
        id: String,
        kind: &'static str,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[snafu(display("write intents are disabled (intent: {intent_name})"))]
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
