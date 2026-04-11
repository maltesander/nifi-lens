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

/// Phase 0: placeholder. Phase 1+ will add variants per view.
#[derive(Debug, Clone)]
pub enum ViewPayload {}

/// Result of a successful intent dispatch.
#[derive(Debug, Clone)]
pub enum IntentOutcome {
    ContextSwitched { new_version: semver::Version },
    ViewRefreshed { view: crate::app::state::ViewId },
    Quitting,
    NotImplementedInPhase0 { intent_name: &'static str },
}
