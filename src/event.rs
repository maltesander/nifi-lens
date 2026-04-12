//! Events that flow through the single AppEvent channel.

use crate::error::NifiLensError;

pub enum AppEvent {
    /// Terminal input forwarded by the crossterm task.
    Input(crossterm::event::Event),
    /// Periodic tick for time-based UI updates (status bar).
    Tick,
    /// Per-view data from a worker task. Declared for Phase 1+; unused in Phase 0.
    Data(ViewPayload),
    /// Result of an intent dispatch, folded back into AppState by the UI task.
    IntentOutcome(Result<IntentOutcome, NifiLensError>),
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
    Health(HealthPayload),
}

/// One poll cycle's worth of data for the Overview tab. Composed inside the
/// worker from three parallel client calls, then pushed as a single event
/// so the reducer treats the refresh as atomic.
#[derive(Debug, Clone)]
pub struct OverviewPayload {
    pub about: crate::client::AboutSnapshot,
    pub controller: crate::client::ControllerStatusSnapshot,
    pub root_pg: crate::client::RootPgStatusSnapshot,
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
    /// Phase 3: the user asked to jump to a component in Browser.
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
}

/// Payload variants pushed from the Health worker back into the UI loop.
#[derive(Debug, Clone)]
pub enum HealthPayload {
    PgStatus(crate::client::health::FullPgStatusSnapshot),
    SystemDiag(crate::client::health::SystemDiagSnapshot),
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
        detail: crate::client::ProvenanceEventDetail,
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
