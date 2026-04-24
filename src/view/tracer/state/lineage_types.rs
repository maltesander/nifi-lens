//! Pure type defs for the lineage-view portion of tracer state.
//!
//! Re-exported from `super` via `pub use lineage_types::*;` so external
//! callers continue to see these under `crate::view::tracer::state::*`.

use std::time::SystemTime;

use tokio::task::AbortHandle;

use crate::app::navigation::ListNavigation;
use crate::client::{
    AttributeTriple, ContentRender, ContentSide, LineageSnapshot, ProvenanceEventDetail,
};

// ── LineageRunning ───────────────────────────────────────────────────────────

/// State while a lineage query is being polled.
#[derive(Debug)]
pub struct LineageRunningState {
    /// The flowfile UUID being traced.
    pub uuid: String,
    /// Opaque query ID returned by the NiFi server.
    pub query_id: String,
    /// Cluster node ID returned by the server in cluster mode. Must be
    /// passed to poll and delete calls.
    pub cluster_node_id: Option<String>,
    /// Last reported completion percentage (0–100).
    pub percent: u8,
    /// Wall-clock time when the query was submitted.
    pub started_at: SystemTime,
    /// Handle to cancel the polling task if the user presses Escape.
    pub abort: Option<AbortHandle>,
}

// ── LineageView ──────────────────────────────────────────────────────────────

/// Where keyboard focus lives in Lineage mode.
///
/// Mirrors the Browser tab's focus cycle: the timeline is the default
/// "list" pane, and `l` / `Right` steps through sub-panels on the
/// detail side — first the attribute table, then the content pane
/// once it has been loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineageFocus {
    /// Arrow keys navigate the event timeline.
    #[default]
    Timeline,
    /// Arrow keys navigate rows inside the attribute table.
    Attributes {
        /// Row index into the currently-visible (filtered) attribute list.
        row: usize,
    },
    /// Arrow keys scroll the content pane. The pane is expanded to
    /// consume most of the detail area under this focus.
    Content {
        /// Top-line scroll offset, in rendered content lines.
        scroll: u16,
    },
}

/// Classifies an attribute row for diff-style coloring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeClass {
    /// Attribute is new in this event (no previous value).
    Added,
    /// Attribute was removed by this event (no current value).
    Deleted,
    /// Attribute is present on both sides and its value changed.
    Updated,
    /// Attribute is present on both sides and its value is identical.
    Unchanged,
}

impl AttributeClass {
    /// Returns the class of the given triple.
    pub fn of(attr: &AttributeTriple) -> Self {
        match (attr.previous.as_ref(), attr.current.as_ref()) {
            (None, Some(_)) => Self::Added,
            (Some(_), None) => Self::Deleted,
            _ if attr.is_changed() => Self::Updated,
            _ => Self::Unchanged,
        }
    }
}

// ── DetailTab ─────────────────────────────────────────────────────────────────

/// Which detail-pane tab is active when `LineageFocus::Detail` is in effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DetailTab {
    /// Shows the event's attribute table (prev → current diff).
    #[default]
    Attributes,
    /// Shows the input content claim.
    Input,
    /// Shows the output content claim.
    Output,
}

impl DetailTab {
    /// Steps to the next enabled tab, wrapping around.
    ///
    /// `has_input` / `has_output` reflect whether the currently loaded event
    /// actually has a claim on that side. `Attributes` is always enabled.
    pub fn cycle_right(self, has_input: bool, has_output: bool) -> Self {
        let order = [Self::Attributes, Self::Input, Self::Output];
        let enabled = |t: Self| match t {
            Self::Attributes => true,
            Self::Input => has_input,
            Self::Output => has_output,
        };
        let start = match self {
            Self::Attributes => 0,
            Self::Input => 1,
            Self::Output => 2,
        };
        for step in 1..=3 {
            let idx = (start + step) % 3;
            let t = order[idx];
            if enabled(t) {
                return t;
            }
        }
        self
    }

    /// Steps to the previous enabled tab, wrapping around.
    pub fn cycle_left(self, has_input: bool, has_output: bool) -> Self {
        let order = [Self::Attributes, Self::Input, Self::Output];
        let enabled = |t: Self| match t {
            Self::Attributes => true,
            Self::Input => has_input,
            Self::Output => has_output,
        };
        let start = match self {
            Self::Attributes => 0,
            Self::Input => 1,
            Self::Output => 2,
        };
        for step in 1..=3 {
            let idx = (start + 3 - step) % 3;
            let t = order[idx];
            if enabled(t) {
                return t;
            }
        }
        self
    }
}

/// State after the lineage query has finished.
#[derive(Debug)]
pub struct LineageView {
    /// The flowfile UUID that was traced.
    pub uuid: String,
    /// Complete lineage snapshot returned by the server.
    pub snapshot: LineageSnapshot,
    /// Index of the currently selected event row.
    pub selected_event: usize,
    /// Detail pane for the selected event (loaded on demand).
    pub event_detail: EventDetail,
    /// Accumulated cache of per-event details, keyed by event_id.
    ///
    /// Populated as the user navigates the timeline (auto-load on scroll).
    /// Used to render attribute-change and content indicators in each
    /// timeline row without requiring a separate fetch per visible row.
    pub loaded_details: std::collections::HashMap<i64, ProvenanceEventDetail>,
    /// Whether to show all attributes or only changed ones.
    pub diff_mode: AttributeDiffMode,
    /// When the lineage snapshot was last fetched.
    pub fetched_at: SystemTime,
    /// Which sub-pane currently owns keyboard focus.
    pub focus: LineageFocus,
    /// Which tab is shown in the detail pane (Attributes | Input | Output).
    /// Only meaningful when `focus` is `LineageFocus::Detail` or when
    /// an event detail has been loaded.
    pub active_detail_tab: DetailTab,
}

impl ListNavigation for LineageView {
    fn list_len(&self) -> usize {
        self.snapshot.events.len()
    }

    fn selected(&self) -> Option<usize> {
        if self.snapshot.events.is_empty() {
            None
        } else {
            Some(self.selected_event)
        }
    }

    fn set_selected(&mut self, index: Option<usize>) {
        self.selected_event = index.unwrap_or(0);
    }

    fn wraps(&self) -> bool {
        true
    }
}

// ── EventDetail ──────────────────────────────────────────────────────────────

/// Load state of the per-event detail pane.
#[derive(Debug, Default)]
pub enum EventDetail {
    /// No fetch has been requested yet.
    #[default]
    NotLoaded,
    /// A fetch is in flight.
    Loading,
    /// Detail loaded successfully; content may be separately loaded.
    Loaded {
        event: Box<ProvenanceEventDetail>,
        content: ContentPane,
    },
    /// The fetch failed.
    Failed(String),
}

// ── ContentPane ──────────────────────────────────────────────────────────────

/// Load state of the content preview within an event detail pane.
#[derive(Debug, Default)]
pub enum ContentPane {
    /// Not yet requested; user must press a keybind.
    #[default]
    Collapsed,
    /// Input-side fetch is in flight.
    LoadingInput,
    /// Output-side fetch is in flight.
    LoadingOutput,
    /// Content loaded and ready to display.
    Shown {
        side: ContentSide,
        render: ContentRender,
        bytes_fetched: usize,
        truncated: bool,
    },
    /// The content fetch failed.
    Failed(String),
}

// ── AttributeDiffMode ────────────────────────────────────────────────────────

/// Controls which attributes are shown in the detail pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AttributeDiffMode {
    /// Show all attributes regardless of whether they changed.
    #[default]
    All,
    /// Show only attributes whose `previous` differs from `current`.
    Changed,
}

impl AttributeDiffMode {
    /// Cycles between `All` and `Changed`.
    pub fn toggle(self) -> Self {
        match self {
            Self::All => Self::Changed,
            Self::Changed => Self::All,
        }
    }

    /// Returns true if `triple` should be shown under this mode.
    pub fn matches(self, triple: &AttributeTriple) -> bool {
        match self {
            Self::All => true,
            Self::Changed => triple.is_changed(),
        }
    }
}
