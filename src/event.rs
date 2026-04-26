//! Events that flow through the single AppEvent channel.

use crate::error::NifiLensError;

/// Events flowing through the single channel consumed by the UI task.
pub enum AppEvent {
    /// Terminal input forwarded by the crossterm task.
    Input(crossterm::event::Event),
    /// Periodic tick for time-based UI updates (status bar).
    Tick,
    /// Per-view data from a worker task. Declared for Phase 1+; unused in Phase 0.
    Data(ViewPayload),
    /// Result of an intent dispatch, folded back into AppState by the UI task.
    IntentOutcome(Result<IntentOutcome, NifiLensError>),
    /// Raw fetcher output from `ClusterStore`. Main loop applies it to
    /// `AppState.cluster` and follows up with `ClusterChanged`.
    ClusterUpdate(crate::cluster::ClusterUpdate),
    /// Emitted by the main loop after `ClusterUpdate` is applied, so
    /// per-view reducers can re-derive their projections.
    ClusterChanged(crate::cluster::ClusterEndpoint),
    /// Graceful quit request.
    Quit,
}

/// Data delivered from a view's worker task back into the UI loop.
///
/// Task 8 deleted the `Overview` variant — Overview is now a
/// store-only consumer that reacts to `ClusterChanged` events and
/// reads projections straight from `state.cluster.snapshot`.
#[derive(Debug, Clone)]
pub enum ViewPayload {
    Browser(BrowserPayload),
    Tracer(TracerPayload),
    Events(EventsPayload),
}

/// Payload sent from the Browser detail worker — a single variant
/// carrying one completed per-node detail fetch.
///
/// Task 6 of the central-cluster-store refactor removed the `Tree`
/// variant: the Browser arena is now rebuilt from
/// `AppState.cluster.snapshot` whenever any of `RootPgStatus`,
/// `ControllerServices`, or `ConnectionsByPg` updates arrive.
#[derive(Debug, Clone)]
pub enum BrowserPayload {
    Detail(Box<crate::view::browser::state::NodeDetailSnapshot>),
    /// Result of a successful version-control modal load: identity
    /// re-fetched from `/versions/process-groups/{id}` plus the
    /// flattened diff from `/process-groups/{id}/local-modifications`.
    VersionControlModalLoaded {
        pg_id: String,
        identity: Option<crate::cluster::snapshot::VersionControlSummary>,
        differences: crate::client::FlowComparisonGrouped,
    },
    /// Failure of either fetch in the modal load. The reducer renders
    /// `err` inside the modal and clears the worker handle.
    VersionControlModalFailed {
        pg_id: String,
        err: String,
    },
}

/// Result of a successful intent dispatch.
#[derive(Debug)]
pub enum IntentOutcome {
    ContextSwitched {
        new_context_name: String,
        new_version: semver::Version,
        new_base_url: String,
    },
    ViewRefreshed {
        view: crate::app::state::ViewId,
    },
    Quitting,
    /// The intent is valid but its target phase hasn't landed yet.
    /// The banner shows `"{intent_name}: not yet wired (Phase {phase})"`.
    NotImplementedInPhase {
        intent_name: &'static str,
        phase: u8,
    },
    /// Phase 3: the user asked to goto a component in Browser.
    /// The reducer switches tabs, expands ancestors, and sets selection.
    OpenInBrowserTarget {
        component_id: String,
        group_id: String,
    },
    /// Phase 4: cross-link from Bulletins tab navigates Tracer to latest-events view.
    TracerLandingOn {
        component_id: String,
    },
    /// Phase 4: a lineage query has been submitted; holds the abort handle.
    TracerLineageStarted {
        uuid: String,
        abort: tokio::task::AbortHandle,
    },
    /// Phase 4: user submitted input that is not a valid UUID.
    TracerInputInvalid {
        raw: String,
    },
    /// Phase 6: cross-link from Bulletins/Browser `t` lands on Events
    /// pre-filled with the component and a 15-minute time window. The
    /// reducer switches tabs, seeds `filters.source`, and kicks off a
    /// query submission.
    EventsLandingOn {
        component_id: String,
    },
    /// Parameter-contexts feature: open the parameter-context modal on
    /// Browser scoped to the given PG, optionally pre-selecting a
    /// parameter name. The reducer calls
    /// `BrowserState::open_parameter_context_modal`.
    OpenParameterContextModalTarget {
        pg_id: String,
        preselect: Option<String>,
    },
}

/// Payload variants pushed from Tracer workers back into the UI loop.
#[derive(Debug, Clone)]
pub enum TracerPayload {
    LatestEvents(crate::client::LatestEventsSnapshot),
    LatestEventsFailed {
        component_id: String,
        error: String,
    },
    LineageSubmitted {
        uuid: String,
        query_id: String,
        cluster_node_id: Option<String>,
    },
    LineagePartial {
        query_id: String,
        percent: u8,
    },
    LineageDone {
        uuid: String,
        query_id: String,
        snapshot: crate::client::LineageSnapshot,
        fetched_at: std::time::SystemTime,
    },
    LineageFailed {
        uuid: String,
        query_id: String,
        error: String,
    },
    EventDetail {
        event_id: i64,
        detail: Box<crate::client::ProvenanceEventDetail>,
    },
    EventDetailFailed {
        event_id: i64,
        error: String,
    },
    Content(crate::client::ContentSnapshot),
    ContentFailed {
        event_id: i64,
        side: crate::client::ContentSide,
        error: String,
    },
    ContentSaved {
        path: std::path::PathBuf,
    },
    ContentSaveFailed {
        path: std::path::PathBuf,
        error: String,
    },
    ModalChunk {
        event_id: i64,
        side: crate::client::ContentSide,
        offset: usize,
        bytes: Vec<u8>,
        eof: bool,
        /// The length the worker asked for; the reducer uses this to
        /// decide whether a short read triggers EOF independently of
        /// client-side classification.
        requested_len: usize,
    },
    ModalChunkFailed {
        event_id: i64,
        side: crate::client::ContentSide,
        offset: usize,
        error: String,
    },
    /// Result of an off-thread tabular decode (Parquet/Avro) spawned
    /// via `tokio::task::spawn_blocking`. The UI-task handler calls
    /// `apply_tabular_decode_result` to install the render.
    ContentDecoded {
        event_id: i64,
        side: crate::client::ContentSide,
        render: crate::client::tracer::ContentRender,
    },
}

/// Payload variants pushed from the Events tab worker back into the UI loop.
///
/// The full lifecycle:
/// 1. User submits a query → worker emits `QueryStarted { query_id }`.
/// 2. Worker polls and emits `QueryProgress { percent }` until the server
///    reports `finished = true`.
/// 3. Worker emits `QueryDone { events, fetched_at, truncated }` on success.
/// 4. Worker emits `QueryFailed { error }` on any error during the above.
#[derive(Debug, Clone)]
pub enum EventsPayload {
    QueryStarted {
        query_id: String,
    },
    QueryProgress {
        query_id: String,
        percent: u8,
    },
    QueryDone {
        query_id: String,
        events: Vec<crate::client::ProvenanceEventSummary>,
        fetched_at: std::time::SystemTime,
        truncated: bool,
    },
    QueryFailed {
        query_id: Option<String>,
        error: String,
    },
}
