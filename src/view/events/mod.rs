//! Events tab: cluster-wide provenance search.

pub mod render;
pub mod state;
pub mod worker;

use std::time::Duration;

use crate::event::EventsPayload;
use crate::view::events::state::{EventsState, MatchedEvent, WatchStatus};

/// Reduce a watch-worker payload into `EventsState`. No-op when the
/// state is not in watch mode (defensive — the worker may emit one
/// final payload after the user cleared watch mode).
///
/// `selected_row` is the currently-focused buffer index, if any.
/// Used by `WatchSession::push_event` to keep the cursor anchored
/// to the row the user is investigating during overflow.
pub fn handle_watch_payload(
    state: &mut EventsState,
    payload: EventsPayload,
    selected_row: Option<usize>,
) {
    let Some(watch) = state.watch_mut() else {
        return;
    };
    match payload {
        EventsPayload::WatchMatch { summary, attrs } => {
            watch.push_event(MatchedEvent { summary, attrs }, selected_row);
        }
        EventsPayload::WatchTick {
            events_per_sec_ewma,
            last_poll_latency_ms,
            scanned: _,
            matched: _,
            detail_fetch_errors,
        } => {
            watch.stats.events_per_sec_ewma = events_per_sec_ewma;
            watch.stats.last_poll_latency = Some(Duration::from_millis(last_poll_latency_ms));
            watch.stats.detail_fetch_errors = detail_fetch_errors;
            // Promote Waiting -> Tailing once we've seen any tick.
            if matches!(watch.status, WatchStatus::Waiting) {
                watch.status = WatchStatus::Tailing;
            }
        }
        EventsPayload::WatchFailed { error, retry_in_ms } => {
            watch.status = WatchStatus::Failed {
                error,
                retry_in: Duration::from_millis(retry_in_ms),
            };
        }
        _ => { /* not a watch payload */ }
    }
}

/// Flip the active watch to `Paused`. Call from `WorkerRegistry`
/// teardown when the user navigates away from the Events tab. The
/// session (predicate, buffer, cursor) is preserved on `EventsState`
/// so re-entering the tab can resume from where we left off.
pub fn pause_watch(state: &mut EventsState) {
    if let Some(w) = state.watch_mut() {
        w.status = crate::view::events::state::WatchStatus::Paused;
    }
}

/// Flip the active watch back to `Waiting` so the next `WatchTick`
/// promotes it to `Tailing`. Call after re-spawning the worker on
/// tab re-entry. No-op when the watch is already in any non-Paused
/// state — re-entering with a Failed or Tailing watch leaves the
/// status untouched.
pub fn resume_watch(state: &mut EventsState) {
    if let Some(w) = state.watch_mut()
        && matches!(w.status, crate::view::events::state::WatchStatus::Paused)
    {
        w.status = crate::view::events::state::WatchStatus::Waiting;
    }
}
