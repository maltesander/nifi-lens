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
#[derive(Debug, Clone)]
pub enum ViewPayload {
    Overview(OverviewPayload),
    Bulletins(BulletinsPayload),
    Browser(BrowserPayload),
    Tracer(TracerPayload),
    Events(EventsPayload),
}

/// Payload variants pushed from the merged Overview worker. After
/// Phase 3 the Overview worker runs two parallel pollers (PG status @
/// 10s, system diagnostics @ 30s) and emits one of these variants per
/// poll. The reducer in `view::overview::state::apply_payload` matches
/// on the variant.
#[derive(Debug, Clone)]
pub enum OverviewPayload {
    /// Result of the 10-second PG-status poll. Carries the
    /// pre-Phase-3 set of fields.
    PgStatus(OverviewPgStatusPayload),
    /// Result of the 30-second system-diagnostics poll. Includes
    /// per-node heap, GC, load, and repository fill data.
    SystemDiag(crate::client::health::SystemDiagSnapshot),
    /// Aggregate-only fallback when the nodewise system diagnostics
    /// call failed. Carries the aggregate snapshot plus a warning
    /// message for the banner.
    SystemDiagFallback {
        diag: crate::client::health::SystemDiagSnapshot,
        warning: String,
    },
}

/// Inner payload for the PG-status poll. `root_pg_status` and
/// `controller_services` are sourced from `state.cluster.snapshot` —
/// the Overview worker no longer fetches them; `ClusterStore` owns
/// those endpoints.
#[derive(Debug, Clone)]
pub struct OverviewPgStatusPayload {
    pub about: crate::client::AboutSnapshot,
    pub controller: crate::client::ControllerStatusSnapshot,
    pub bulletin_board: crate::client::BulletinBoardSnapshot,
    /// Wall-clock time (from `std::time::SystemTime`) when the worker
    /// assembled this payload. Used by the reducer to anchor the sparkline
    /// and the "last refresh" label.
    pub fetched_at: std::time::SystemTime,
}

/// One poll cycle's worth of data for the Bulletins tab. Composed inside
/// the worker from a single `bulletin_board(after_id, limit)` call.
#[derive(Debug, Clone)]
pub struct BulletinsPayload {
    pub bulletins: Vec<crate::client::BulletinSnapshot>,
    /// Wall-clock time when the worker assembled this payload. Used by
    /// the renderer for the "last Ns ago" label.
    pub fetched_at: std::time::SystemTime,
}

/// Payload sent from the Browser worker. Two variants — one for the
/// full tree refresh (15 s cadence), one for an on-demand per-node
/// detail fetch.
#[derive(Debug, Clone)]
pub enum BrowserPayload {
    Tree(crate::client::RecursiveSnapshot),
    Detail(Box<crate::view::browser::state::NodeDetailSnapshot>),
}

/// Result of a successful intent dispatch.
#[derive(Debug)]
pub enum IntentOutcome {
    ContextSwitched {
        new_context_name: String,
        new_version: semver::Version,
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
