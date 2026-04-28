#![allow(clippy::module_name_repetitions)]

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use tokio::task::JoinHandle;

use crate::widget::scroll::VerticalScrollState;
use crate::widget::search::SearchState;

/// Top-level state for the connection-detail flowfile listing panel.
/// `BrowserState::queue_listing` is `Some` exactly when the user has
/// selected a Connection node — populated even when `flow_files_queued == 0`
/// so the renderer can show the muted "queue empty" line.
#[derive(Debug)]
pub struct QueueListingState {
    pub queue_id: String,
    pub queue_name: String,
    pub request_id: Option<String>,
    pub percent: u8,
    pub rows: Vec<QueueListingRow>,
    pub total: u64,
    pub truncated: bool,
    pub fetched_at: Option<SystemTime>,
    pub filter: Option<String>,
    pub selected: usize,
    pub error: Option<String>,
    pub timed_out: bool,
    pub handle: Option<QueueListingHandle>,
    pub peek: Option<QueueListingPeekState>,
}

#[derive(Debug, Clone)]
pub struct QueueListingRow {
    pub uuid: String,
    pub filename: Option<String>,
    pub size: u64,
    pub queued_duration: Duration,
    pub position: u64,
    pub penalized: bool,
    pub cluster_node_id: Option<String>,
    pub lineage_duration: Duration,
}

#[derive(Debug)]
pub struct QueueListingPeekState {
    pub uuid: String,
    pub queue_id: String,
    pub cluster_node_id: Option<String>,
    pub identity: PeekIdentity,
    pub attrs: Option<BTreeMap<String, String>>,
    pub error: Option<String>,
    pub scroll: VerticalScrollState,
    pub search: Option<SearchState>,
    pub fetch_handle: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone)]
pub struct PeekIdentity {
    pub uuid: String,
    pub filename: Option<String>,
    pub size: u64,
    pub mime_type: Option<String>,
    pub content_claim: Option<ContentClaimSummary>,
    pub cluster_node_id: Option<String>,
    pub lineage_duration: Duration,
    pub penalized: bool,
}

#[derive(Debug, Clone)]
pub struct ContentClaimSummary {
    pub container: String,
    pub section: String,
    pub identifier: String,
    pub offset: u64,
    pub file_size: u64,
}

// Re-export so callers say `state::queue_listing::QueueListingHandle`. The
// struct lives in `worker.rs` so the Drop-DELETE impl can sit alongside the
// worker that constructs it (Task 5 fills out the body).
pub use crate::view::browser::worker::QueueListingHandle;

impl QueueListingState {
    /// Construct a fresh state for a connection that just became
    /// selected. Worker spawn + handle attachment happens in
    /// `BrowserState`'s selection-change reducer (Task 13).
    pub fn pending(queue_id: String, queue_name: String) -> Self {
        Self {
            queue_id,
            queue_name,
            request_id: None,
            percent: 0,
            rows: Vec::new(),
            total: 0,
            truncated: false,
            fetched_at: None,
            filter: None,
            selected: 0,
            error: None,
            timed_out: false,
            handle: None,
            peek: None,
        }
    }

    /// Apply a `BrowserPayload::QueueListingProgress`. Returns `true` if
    /// the payload matches the active queue and the state mutated.
    pub fn apply_progress(&mut self, queue_id: &str, percent: u8) -> bool {
        if self.queue_id != queue_id {
            return false;
        }
        self.percent = percent;
        self.error = None;
        true
    }

    /// Apply a `BrowserPayload::QueueListingComplete`. Returns `true` if
    /// the payload matches the active queue and the state mutated.
    /// Sets `rows`, `total`, `truncated`, `percent = 100`, `fetched_at`,
    /// clears `error` / `timed_out`, and re-clamps the selection.
    pub fn apply_complete(
        &mut self,
        queue_id: &str,
        rows: Vec<QueueListingRow>,
        total: u64,
        truncated: bool,
    ) -> bool {
        if self.queue_id != queue_id {
            return false;
        }
        self.rows = rows;
        self.total = total;
        self.truncated = truncated;
        self.percent = 100;
        self.fetched_at = Some(SystemTime::now());
        self.error = None;
        self.timed_out = false;
        self.clamp_selection();
        true
    }

    /// Apply a `BrowserPayload::QueueListingError`. Returns `true` if
    /// the payload matches the active queue and the state mutated.
    /// Stores the error message and resets `percent` to 0.
    pub fn apply_error(&mut self, queue_id: &str, msg: String) -> bool {
        if self.queue_id != queue_id {
            return false;
        }
        self.error = Some(msg);
        self.percent = 0;
        true
    }

    /// Apply a `BrowserPayload::QueueListingTimeout`. Returns `true` if
    /// the payload matches the active queue and the state mutated.
    /// Sets `timed_out = true`, records a fixed error message, and resets `percent` to 0.
    pub fn apply_timeout(&mut self, queue_id: &str) -> bool {
        if self.queue_id != queue_id {
            return false;
        }
        self.timed_out = true;
        self.error = Some("listing timeout".to_string());
        self.percent = 0;
        true
    }

    /// Apply a filename filter, lowercasing the input. Empty/whitespace
    /// filters reset to `None`. Selection is re-clamped to the visible
    /// window.
    pub fn set_filter(&mut self, filter: Option<String>) {
        self.filter = filter
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty());
        self.clamp_selection();
    }

    /// Indices into `self.rows` matching the active filter. With no
    /// filter, returns `0..rows.len()`. Always returns indices in row
    /// order.
    pub fn visible_indices(&self) -> Vec<usize> {
        match &self.filter {
            None => (0..self.rows.len()).collect(),
            Some(needle) => self
                .rows
                .iter()
                .enumerate()
                .filter(|(_, row)| {
                    row.filename
                        .as_deref()
                        .map(|n| n.to_lowercase().contains(needle))
                        .unwrap_or(false)
                })
                .map(|(i, _)| i)
                .collect(),
        }
    }

    fn clamp_selection(&mut self) {
        let visible = self.visible_indices();
        if visible.is_empty() {
            self.selected = 0;
        } else if self.selected >= visible.len() {
            self.selected = visible.len() - 1;
        }
    }
}

impl QueueListingPeekState {
    /// Construct from the row that was selected when `i` was pressed.
    /// Identity fields populate immediately so the modal renders
    /// something useful before the GET completes.
    pub fn from_row(queue_id: String, row: &QueueListingRow) -> Self {
        Self {
            uuid: row.uuid.clone(),
            queue_id,
            cluster_node_id: row.cluster_node_id.clone(),
            identity: PeekIdentity {
                uuid: row.uuid.clone(),
                filename: row.filename.clone(),
                size: row.size,
                mime_type: None,
                content_claim: None,
                cluster_node_id: row.cluster_node_id.clone(),
                lineage_duration: row.lineage_duration,
                penalized: row.penalized,
            },
            attrs: None,
            error: None,
            scroll: VerticalScrollState::default(),
            search: None,
            fetch_handle: None,
        }
    }

    /// Apply a `BrowserPayload::FlowfilePeek` payload. Returns `true`
    /// when `(queue_id, uuid)` matches and state mutated.
    pub fn apply_peek(
        &mut self,
        queue_id: &str,
        uuid: &str,
        attrs: BTreeMap<String, String>,
        content_claim: Option<ContentClaimSummary>,
        mime_type: Option<String>,
    ) -> bool {
        if self.queue_id != queue_id || self.uuid != uuid {
            return false;
        }
        self.attrs = Some(attrs);
        self.identity.content_claim = content_claim;
        self.identity.mime_type = mime_type;
        self.error = None;
        true
    }

    /// Apply a `BrowserPayload::FlowfilePeekError` payload. Sets the
    /// error chip; preserves any prior loaded attrs.
    pub fn apply_error(&mut self, queue_id: &str, uuid: &str, err: String) -> bool {
        if self.queue_id != queue_id || self.uuid != uuid {
            return false;
        }
        self.error = Some(err);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(uuid: &str, filename: Option<&str>, queued_ms: u64, penalized: bool) -> QueueListingRow {
        QueueListingRow {
            uuid: uuid.to_string(),
            filename: filename.map(str::to_string),
            size: 1024,
            queued_duration: Duration::from_millis(queued_ms),
            position: 1,
            penalized,
            cluster_node_id: None,
            lineage_duration: Duration::from_millis(queued_ms * 2),
        }
    }

    #[test]
    fn pending_initializes_empty_loading_state() {
        let s = QueueListingState::pending("q1".into(), "Q1".into());
        assert_eq!(s.queue_id, "q1");
        assert_eq!(s.percent, 0);
        assert!(s.rows.is_empty());
        assert!(s.error.is_none());
        assert!(s.peek.is_none());
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn progress_updates_percent() {
        let mut s = QueueListingState::pending("q1".into(), "Q1".into());
        assert!(s.apply_progress("q1", 25));
        assert_eq!(s.percent, 25);
    }

    #[test]
    fn progress_ignored_for_other_queue() {
        let mut s = QueueListingState::pending("q1".into(), "Q1".into());
        assert!(!s.apply_progress("q2", 25));
        assert_eq!(s.percent, 0);
    }

    #[test]
    fn complete_populates_rows_and_clears_error() {
        let mut s = QueueListingState::pending("q1".into(), "Q1".into());
        s.error = Some("stale".into());
        let rows = vec![row("ff-1", Some("a.txt"), 1000, false)];
        assert!(s.apply_complete("q1", rows, 1, false));
        assert_eq!(s.rows.len(), 1);
        assert_eq!(s.percent, 100);
        assert!(s.error.is_none());
        assert!(s.fetched_at.is_some());
        assert!(!s.truncated);
    }

    #[test]
    fn complete_marks_truncation() {
        let mut s = QueueListingState::pending("q1".into(), "Q1".into());
        let rows: Vec<QueueListingRow> = (0..100)
            .map(|i| row(&format!("ff-{i}"), Some("a.txt"), 1000, false))
            .collect();
        s.apply_complete("q1", rows, 4827, true);
        assert!(s.truncated);
        assert_eq!(s.total, 4827);
    }

    #[test]
    fn error_sets_chip_clears_loading() {
        let mut s = QueueListingState::pending("q1".into(), "Q1".into());
        s.percent = 50;
        assert!(s.apply_error("q1", "boom".into()));
        assert_eq!(s.error.as_deref(), Some("boom"));
        assert_eq!(s.percent, 0);
    }

    #[test]
    fn timeout_distinct_from_error() {
        let mut s = QueueListingState::pending("q1".into(), "Q1".into());
        assert!(s.apply_timeout("q1"));
        assert!(s.timed_out);
        assert_eq!(s.error.as_deref(), Some("listing timeout"));
    }

    #[test]
    fn filter_narrows_visible_rows() {
        let mut s = QueueListingState::pending("q1".into(), "Q1".into());
        s.rows = vec![
            row("ff-1", Some("alpha.txt"), 1000, false),
            row("ff-2", Some("beta.parquet"), 1000, false),
            row("ff-3", Some("alphabetical.csv"), 1000, false),
        ];
        s.set_filter(Some("alpha".into()));
        let visible = s.visible_indices();
        assert_eq!(visible, vec![0, 2]);
    }

    #[test]
    fn filter_clears_on_empty_input() {
        let mut s = QueueListingState::pending("q1".into(), "Q1".into());
        s.set_filter(Some("foo".into()));
        assert!(s.filter.is_some());
        s.set_filter(Some("   ".into()));
        assert!(s.filter.is_none());
    }

    #[test]
    fn filter_lowercases_input() {
        let mut s = QueueListingState::pending("q1".into(), "Q1".into());
        s.rows = vec![row("ff-1", Some("Sensor.parquet"), 1000, false)];
        s.set_filter(Some("SENSOR".into()));
        assert_eq!(s.visible_indices(), vec![0]);
    }

    #[test]
    fn selection_clamps_when_filter_narrows() {
        let mut s = QueueListingState::pending("q1".into(), "Q1".into());
        s.rows = vec![
            row("ff-1", Some("alpha.txt"), 1000, false),
            row("ff-2", Some("beta.txt"), 1000, false),
            row("ff-3", Some("alpha-2.txt"), 1000, false),
        ];
        s.selected = 2;
        s.set_filter(Some("beta".into()));
        // visible window is just index 1 (one match) → selected clamps to 0.
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn complete_clamps_selection_when_rows_shrink() {
        let mut s = QueueListingState::pending("q1".into(), "Q1".into());
        s.rows = (0..50)
            .map(|i| row(&format!("ff-{i}"), Some("a"), 1000, false))
            .collect();
        s.selected = 40;
        s.apply_complete("q1", vec![row("ff-x", Some("b"), 1000, false)], 1, false);
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn filter_no_match_returns_empty_visible() {
        let mut s = QueueListingState::pending("q1".into(), "Q1".into());
        s.rows = vec![row("ff-1", Some("alpha.txt"), 1000, false)];
        s.selected = 0;
        s.set_filter(Some("zzz".into()));
        assert!(s.visible_indices().is_empty());
        assert_eq!(s.selected, 0);
    }
}
