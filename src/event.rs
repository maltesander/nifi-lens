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

/// Result of a successful intent dispatch.
#[derive(Debug, Clone)]
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
}
