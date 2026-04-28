//! AppState and the pure state reducer.
//!
//! The reducer folds AppEvent into AppState and returns whether a redraw
//! is needed. State is owned exclusively by the UI task.

mod browser;
mod bulletins;
mod events;
mod hints;
mod overview;
mod tracer;

pub use hints::collect_hints;
#[cfg(test)]
pub(crate) use hints::modal_hints_for_test;

use std::time::Instant;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use semver::Version;

use crate::NifiLensError;
use crate::config::Config;
use crate::event::{AppEvent, IntentOutcome, ViewPayload};
use crate::intent::CrossLink;
use crate::view::browser::state::{BrowserState, FlowIndex, NodeDetail, rebuild_visible};
use crate::view::bulletins::state::BulletinsState;
use crate::view::events::state::EventsState;
use crate::view::overview::OverviewState;
use crate::view::overview::state::OverviewFocus;
use crate::view::tracer::state::TracerState;

// ---------------------------------------------------------------------------
// ViewKeyHandler trait
// ---------------------------------------------------------------------------

/// Per-view key-handling trait. Uses static methods (not `&mut self`)
/// because handlers need `&mut AppState`.
pub(crate) trait ViewKeyHandler {
    /// Handle a typed view-local verb. Returns `Some(UpdateResult)` if
    /// the verb was consumed, `None` to let it fall through.
    fn handle_verb(state: &mut AppState, verb: crate::input::ViewVerb) -> Option<UpdateResult>;

    /// Handle a typed focus action. Returns `Some(UpdateResult)` if
    /// the action was consumed, `None` to let it fall through.
    fn handle_focus(
        state: &mut AppState,
        action: crate::input::FocusAction,
    ) -> Option<UpdateResult>;

    /// The single "most natural cross-link" for the current selection,
    /// used by Rule 1a (Enter-fallback). Default: none.
    fn default_cross_link(_state: &AppState) -> Option<crate::input::GoTarget> {
        None
    }

    /// True when the view is in a text-input mode (search box, UUID
    /// entry, etc). The dispatcher bypasses `KeyMap` entirely in that
    /// case and forwards the raw `KeyEvent` to `handle_text_input`.
    fn is_text_input_focused(_state: &AppState) -> bool {
        false
    }

    /// True when app-level shortcuts (F1-F5 tab switch, `?` help, `:`
    /// context switcher, `f` fuzzy find) must be suppressed because the
    /// user is actively typing into a modal-style input bar.
    ///
    /// This is a strict subset of [`Self::is_text_input_focused`]:
    /// Bulletins' filter bar and Events' filter edit both capture chars
    /// *and* block global shortcuts, but Tracer's Entry (UUID input)
    /// captures chars without blocking — the user still needs F1-F5 to
    /// leave the tab from the default Entry screen. Default: `false`.
    fn blocks_app_shortcuts(_state: &AppState) -> bool {
        false
    }

    /// Handle a raw `KeyEvent` while in text-input mode. Default: drop
    /// (return `None`). Views with text-input mode override this.
    fn handle_text_input(_state: &mut AppState, _key: KeyEvent) -> Option<UpdateResult> {
        None
    }
}

/// Expand to a `match` over every `ViewId` variant that calls the
/// named `ViewKeyHandler` static method on the corresponding per-view
/// handler type. Adding a new view means one new arm here plus one
/// `ViewId` variant; the call sites do not change.
macro_rules! dispatch_handler {
    ($tab:expr, $method:ident, $state:expr $(, $arg:expr)* $(,)?) => {
        match $tab {
            ViewId::Overview  => overview::OverviewHandler::$method($state $(, $arg)*),
            ViewId::Bulletins => bulletins::BulletinsHandler::$method($state $(, $arg)*),
            ViewId::Browser   => browser::BrowserHandler::$method($state $(, $arg)*),
            ViewId::Events    => events::EventsHandler::$method($state $(, $arg)*),
            ViewId::Tracer    => tracer::TracerHandler::$method($state $(, $arg)*),
        }
    };
}

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewId {
    Overview,
    Bulletins,
    Browser,
    Events,
    Tracer,
}

impl ViewId {
    pub fn next(self) -> Self {
        match self {
            Self::Overview => Self::Bulletins,
            Self::Bulletins => Self::Browser,
            Self::Browser => Self::Events,
            Self::Events => Self::Tracer,
            Self::Tracer => Self::Overview,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Overview => Self::Tracer,
            Self::Bulletins => Self::Overview,
            Self::Browser => Self::Bulletins,
            Self::Events => Self::Browser,
            Self::Tracer => Self::Events,
        }
    }
}

/// Cluster-wide summary shown in the top-bar identity strip.
///
/// Populated by the cluster store. Until the first cluster-nodes update
/// arrives the fields stay `None` and the top-bar renders `nodes ?/?`
/// as a muted placeholder.
#[derive(Debug, Default, Clone)]
pub struct ClusterSummary {
    pub connected_nodes: Option<usize>,
    pub total_nodes: Option<usize>,
}

/// Maximum time the UI thread will block waiting for an `arboard`
/// clipboard read or write. On a stalled X11 / Wayland clipboard
/// daemon the underlying syscall can hang indefinitely; bounding it
/// here keeps the UI responsive.
const CLIPBOARD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Wrapper around `arboard::Clipboard` so `AppState` can still derive
/// `Debug`. The real clipboard handle has no `Debug` impl.
pub struct ClipboardHandle(pub arboard::Clipboard);

impl std::fmt::Debug for ClipboardHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardHandle").finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct AppState {
    pub current_tab: ViewId,
    pub context_name: String,
    pub detected_version: Version,
    pub last_refresh: Instant,
    pub cluster_summary: ClusterSummary,
    pub modal: Option<Modal>,
    pub overview: OverviewState,
    pub bulletins: BulletinsState,
    pub browser: BrowserState,
    pub events: EventsState,
    pub tracer: TracerState,
    pub flow_index: Option<FlowIndex>,
    pub status: StatusLine,
    pub timestamp_cfg: crate::timestamp::TimestampConfig,
    pub polling: crate::config::PollingConfig,
    pub tracer_config: crate::config::TracerConfig,
    pub browser_config: crate::config::BrowserConfig,
    pub cluster: crate::cluster::ClusterStore,
    pub error_detail: Option<String>,
    pub should_quit: bool,
    /// Set by the context-switch handler so the app loop can force-restart
    /// the current view worker (the registry no-ops when the view matches).
    pub pending_worker_restart: bool,
    /// Cross-link back/forward history.
    pub history: crate::app::history::TabHistory,
    /// Typed input layer — translates raw `KeyEvent`s into `InputEvent`s.
    pub keymap: crate::input::KeyMap,
    /// Persistent arboard clipboard handle, lazily initialized on first
    /// use. Kept alive for the life of the TUI to prevent arboard's
    /// X11 `Drop` teardown from running on every keypress — that teardown
    /// writes a debug-mode warning to stderr (`x11.rs:1167`) which
    /// corrupts the ratatui alt-screen grid, and tears down the X11
    /// server thread before clipboard managers can grab the content.
    pub clipboard: Option<ClipboardHandle>,
}

impl AppState {
    pub fn new(
        context_name: String,
        detected_version: Version,
        config: &Config,
        base_url: String,
    ) -> Self {
        Self {
            current_tab: ViewId::Overview,
            context_name,
            detected_version,
            last_refresh: Instant::now(),
            cluster_summary: ClusterSummary::default(),
            modal: None,
            overview: OverviewState::new(),
            bulletins: BulletinsState::with_capacity(config.bulletins.ring_size),
            browser: BrowserState::new(),
            events: EventsState::new(),
            tracer: TracerState::new(),
            flow_index: None,
            status: StatusLine::default(),
            timestamp_cfg: crate::timestamp::TimestampConfig {
                format: config.ui.timestamp_format,
                tz: config.ui.timestamp_tz,
            },
            polling: config.polling.clone(),
            tracer_config: config.tracer.clone(),
            browser_config: config.browser.clone(),
            cluster: crate::cluster::ClusterStore::new(
                config.polling.cluster.clone(),
                config.bulletins.ring_size,
                base_url,
            ),
            error_detail: None,
            should_quit: false,
            pending_worker_restart: false,
            history: crate::app::history::TabHistory::default(),
            keymap: crate::input::KeyMap::default(),
            clipboard: None,
        }
    }

    /// Returns the set of `GoTarget`s meaningful for the current tab + selection.
    /// Used by `AppAction::Goto`'s enabled predicate.
    pub fn selection_cross_links(&self) -> Vec<crate::input::GoTarget> {
        use crate::input::GoTarget;
        let mut out = Vec::new();
        for target in [GoTarget::Browser, GoTarget::Events, GoTarget::Tracer] {
            if build_go_crosslink(self, target).is_some() {
                out.push(target);
            }
        }
        out
    }

    pub fn browser_selection_has_properties(&self) -> bool {
        if self.current_tab != ViewId::Browser {
            return false;
        }
        let Some(&arena) = self.browser.visible.get(self.browser.selected) else {
            return false;
        };
        let Some(node) = self.browser.nodes.get(arena) else {
            return false;
        };
        matches!(
            node.kind,
            crate::client::NodeKind::Processor | crate::client::NodeKind::ControllerService
        )
    }

    /// True iff the Browser tab is active and the selected row is a
    /// UUID-bearing component for which NiFi records flow-configuration
    /// action history (processor, PG, connection, controller service,
    /// input/output port). Folder rows return false.
    pub fn browser_selection_supports_action_history(&self) -> bool {
        if self.current_tab != ViewId::Browser {
            return false;
        }
        let Some(&arena) = self.browser.visible.get(self.browser.selected) else {
            return false;
        };
        let Some(node) = self.browser.nodes.get(arena) else {
            return false;
        };
        matches!(
            node.kind,
            crate::client::NodeKind::Processor
                | crate::client::NodeKind::ProcessGroup
                | crate::client::NodeKind::Connection
                | crate::client::NodeKind::ControllerService
                | crate::client::NodeKind::InputPort
                | crate::client::NodeKind::OutputPort,
        )
    }

    /// True iff the Browser tab is active and the selected node is any
    /// ProcessGroup (versioned or not, including the root PG).
    pub fn browser_selection_is_pg(&self) -> bool {
        if self.current_tab != ViewId::Browser {
            return false;
        }
        let Some(&arena) = self.browser.visible.get(self.browser.selected) else {
            return false;
        };
        let Some(node) = self.browser.nodes.get(arena) else {
            return false;
        };
        matches!(node.kind, crate::client::NodeKind::ProcessGroup)
    }

    /// True iff the Browser tab is active, the selected node is a
    /// ProcessGroup, and the cluster snapshot has a bound parameter context
    /// for that PG.
    pub fn browser_selection_pg_has_parameter_context_binding(&self) -> bool {
        if self.current_tab != ViewId::Browser {
            return false;
        }
        let Some(&arena) = self.browser.visible.get(self.browser.selected) else {
            return false;
        };
        let Some(node) = self.browser.nodes.get(arena) else {
            return false;
        };
        if !matches!(node.kind, crate::client::NodeKind::ProcessGroup) {
            return false;
        }
        node.parameter_context_ref.is_some()
    }

    /// True iff the Browser tab is active, the selected node is a
    /// ProcessGroup, and the cluster snapshot has a `VersionControlSummary`
    /// for that PG (i.e. the PG is under version control).
    pub fn browser_selection_is_versioned_pg(&self) -> bool {
        if self.current_tab != ViewId::Browser {
            return false;
        }
        let Some(&arena) = self.browser.visible.get(self.browser.selected) else {
            return false;
        };
        let Some(node) = self.browser.nodes.get(arena) else {
            return false;
        };
        if !matches!(node.kind, crate::client::NodeKind::ProcessGroup) {
            return false;
        }
        crate::view::browser::state::BrowserState::version_control_for(
            &self.cluster.snapshot,
            &node.id,
        )
        .is_some()
    }

    pub fn tracer_content_tab_is_active(&self) -> bool {
        if self.current_tab != ViewId::Tracer {
            return false;
        }
        if let crate::view::tracer::state::TracerMode::Lineage(ref view) = self.tracer.mode {
            matches!(
                view.active_detail_tab,
                crate::view::tracer::state::DetailTab::Input
                    | crate::view::tracer::state::DetailTab::Output
            )
        } else {
            false
        }
    }

    pub fn tracer_has_any_side_available(&self) -> bool {
        use crate::view::tracer::state::{EventDetail, TracerMode};
        if let TracerMode::Lineage(ref view) = self.tracer.mode {
            matches!(
                &view.event_detail,
                EventDetail::Loaded { event, .. }
                    if event.input_available || event.output_available
            )
        } else {
            false
        }
    }

    pub fn tracer_attributes_tab_is_active(&self) -> bool {
        if self.current_tab != ViewId::Tracer {
            return false;
        }
        if let crate::view::tracer::state::TracerMode::Lineage(ref view) = self.tracer.mode {
            view.active_detail_tab == crate::view::tracer::state::DetailTab::Attributes
        } else {
            false
        }
    }

    /// Copy `text` to the system clipboard, using a persistent
    /// `arboard` handle held in `self.clipboard`. Lazily initializes
    /// the handle on first use. Returns `Ok(())` on success or
    /// `Err(String)` describing the clipboard failure (which the
    /// caller should surface as a Warning banner).
    ///
    /// Holding a single long-lived handle keeps arboard's X11
    /// `strong_count` at `MIN_OWNERS` forever, so the teardown branch
    /// in its `Drop` impl (which writes to stderr and corrupts the
    /// ratatui alt-screen grid) never runs until the TUI exits.
    ///
    /// The `set_text` call runs on a worker thread bounded by a
    /// 2 second deadline (`CLIPBOARD_TIMEOUT`). On timeout the handle
    /// is dropped (the next call re-inits a fresh one) and the worker
    /// thread is detached — its hung syscall completes when the OS
    /// clipboard daemon recovers.
    pub fn copy_to_clipboard(&mut self, text: String) -> Result<(), String> {
        if self.clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(cb) => {
                    self.clipboard = Some(ClipboardHandle(cb));
                }
                Err(err) => return Err(err.to_string()),
            }
        }

        // Take the handle out so the worker thread can own it.
        let mut handle = self
            .clipboard
            .take()
            .ok_or_else(|| "clipboard handle unavailable".to_string())?;

        let (tx, rx) = std::sync::mpsc::channel::<(Result<(), String>, ClipboardHandle)>();
        let _detach = std::thread::spawn(move || {
            let result = handle.0.set_text(text).map_err(|e| e.to_string());
            // If the receiver has been dropped (timeout fired), this send is
            // discarded and the thread exits silently.
            let _ = tx.send((result, handle));
        });

        match rx.recv_timeout(CLIPBOARD_TIMEOUT) {
            Ok((result, handle)) => {
                self.clipboard = Some(handle);
                result
            }
            Err(_) => {
                // Worker hung past the 2s deadline. Detach the JoinHandle
                // (drop without join) and leave self.clipboard = None so the
                // next call re-inits a fresh handle.
                Err("clipboard: write timed out after 2s".into())
            }
        }
    }

    /// Read a string from the system clipboard.
    ///
    /// Uses the same persistent `arboard` handle as `copy_to_clipboard`,
    /// lazily initializing it on first use.
    ///
    /// The `get_text` call runs on a worker thread bounded by a
    /// 2 second deadline (`CLIPBOARD_TIMEOUT`). Timeout handling
    /// matches `copy_to_clipboard`.
    pub fn get_from_clipboard(&mut self) -> Result<String, String> {
        if self.clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(cb) => {
                    self.clipboard = Some(ClipboardHandle(cb));
                }
                Err(err) => return Err(err.to_string()),
            }
        }

        // Take the handle out so the worker thread can own it.
        let mut handle = self
            .clipboard
            .take()
            .ok_or_else(|| "clipboard handle unavailable".to_string())?;

        let (tx, rx) = std::sync::mpsc::channel::<(Result<String, String>, ClipboardHandle)>();
        let _detach = std::thread::spawn(move || {
            let result = handle.0.get_text().map_err(|e| e.to_string());
            let _ = tx.send((result, handle));
        });

        match rx.recv_timeout(CLIPBOARD_TIMEOUT) {
            Ok((result, handle)) => {
                self.clipboard = Some(handle);
                result
            }
            Err(_) => Err("clipboard: read timed out after 2s".into()),
        }
    }

    /// Returns true when the currently active view has a text-input field open.
    /// Used by `AppAction::Paste/Cut` enabled predicates.
    pub fn text_input_is_active(&self) -> bool {
        dispatch_handler!(self.current_tab, is_text_input_focused, self)
    }

    /// Returns true when the current view is in a modal-style input mode
    /// that must suppress app-level shortcuts (F1-F5, `?`, `:`, `f`).
    /// See `ViewKeyHandler::blocks_app_shortcuts` for the Tracer-Entry
    /// carve-out.
    pub fn app_shortcuts_blocked(&self) -> bool {
        dispatch_handler!(self.current_tab, blocks_app_shortcuts, self)
    }
}

#[derive(Debug)]
pub enum Modal {
    Help,
    ContextSwitcher(ContextSwitcherState),
    ErrorDetail,
    FuzzyFind(crate::widget::fuzzy_find::FuzzyFindState),
    Properties(crate::view::browser::state::PropertiesModalState),
    SaveEventContent(crate::widget::save_modal::SaveEventContentState),
    /// Per-node detail popup opened from the Overview Nodes panel.
    NodeDetail(Box<crate::client::overview::NodeHealthRow>),
    /// Cross-tab goto menu — shown when `AppAction::Goto` resolves to multiple targets.
    /// Full render logic is added in Task 11; this variant is a stub so Task 10 compiles.
    GotoMenu(crate::widget::goto_menu::GotoMenuState),
}

#[derive(Debug)]
pub struct ContextSwitcherState {
    pub entries: Vec<ContextEntry>,
    pub cursor: usize,
}

impl ContextSwitcherState {
    pub fn from_config(config: &Config, active_name: &str, active_version: &Version) -> Self {
        let entries = config
            .contexts
            .iter()
            .map(|c| ContextEntry {
                name: c.name.clone(),
                url: c.url.clone(),
                is_active: c.name == active_name,
                version: if c.name == active_name {
                    Some(active_version.clone())
                } else {
                    None
                },
                connecting: false,
            })
            .collect::<Vec<_>>();
        let cursor = entries.iter().position(|e| e.is_active).unwrap_or(0);
        Self { entries, cursor }
    }

    pub fn move_cursor_down(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.cursor = (self.cursor + 1) % self.entries.len();
    }

    pub fn move_cursor_up(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        if self.cursor == 0 {
            self.cursor = self.entries.len() - 1;
        } else {
            self.cursor -= 1;
        }
    }

    pub fn selected(&self) -> Option<&ContextEntry> {
        self.entries.get(self.cursor)
    }
}

#[derive(Debug, Clone)]
pub struct ContextEntry {
    pub name: String,
    pub url: String,
    pub is_active: bool,
    pub version: Option<Version>,
    pub connecting: bool,
}

#[derive(Debug, Default)]
pub struct StatusLine {
    pub banner: Option<Banner>,
}

#[derive(Debug, Clone)]
pub struct Banner {
    pub severity: BannerSeverity,
    pub message: String,
    pub detail: Option<String>,
}

impl AppState {
    /// Post an error banner. `detail`, if present, is shown in a modal
    /// when the user presses Enter on the banner.
    pub fn post_error(&mut self, message: String, detail: Option<String>) {
        self.status.banner = Some(Banner {
            severity: BannerSeverity::Error,
            message,
            detail,
        });
    }

    /// Post a warning banner (e.g. recoverable failures like clipboard unavailable).
    pub fn post_warning(&mut self, message: String) {
        self.status.banner = Some(Banner {
            severity: BannerSeverity::Warning,
            message,
            detail: None,
        });
    }

    /// Post an informational banner (e.g. "copied: foo", "saved to /tmp/x").
    pub fn post_info(&mut self, message: String) {
        self.status.banner = Some(Banner {
            severity: BannerSeverity::Info,
            message,
            detail: None,
        });
    }

    /// If the current banner carries a detail, copy it into `error_detail`
    /// and open the error-detail modal. Returns `true` if a modal was opened,
    /// `false` if there was no banner or no detail to escalate.
    pub fn open_banner_detail(&mut self) -> bool {
        let Some(b) = &self.status.banner else {
            return false;
        };
        let Some(detail) = &b.detail else {
            return false;
        };
        self.error_detail = Some(detail.clone());
        self.modal = Some(Modal::ErrorDetail);
        true
    }

    /// Close the current modal and clear any detail text so `error_detail`
    /// does not linger between modal openings.
    pub fn close_modal(&mut self) {
        self.modal = None;
        self.error_detail = None;
    }

    /// Emit a detail-fetch request for the current Browser selection
    /// and reconcile both the sparkline worker and the queue-listing
    /// worker with the new selection. Returns a pair of optional
    /// intents: `(sparkline_followup, queue_listing_followup)`. Callers
    /// should populate both fields of `UpdateResult` with these values.
    pub fn browser_selection_changed(&mut self) -> (Option<PendingIntent>, Option<PendingIntent>) {
        self.browser.emit_detail_request_for_current_selection();
        let sparkline = self.refresh_sparkline_for_selection();
        let queue_listing = self.refresh_queue_listing_for_selection();
        (sparkline, queue_listing)
    }

    /// Reconcile `BrowserState.sparkline` with the current Browser
    /// selection. Returns a `PendingIntent::SpawnSparklineFetchLoop`
    /// when a new worker needs to be spawned (selection changed to a
    /// supported kind, or no sparkline was open before). Returns
    /// `None` when the selection didn't change (worker is still
    /// valid) or the new selection has no `/status/history` endpoint
    /// (CS / Port / Folder), in which case any open sparkline is torn
    /// down via `close_sparkline`.
    ///
    /// The `(kind, id)` mismatch path also aborts the previous
    /// worker handle inside `open_sparkline_for_selection`, so the
    /// dispatcher never has two concurrent sparkline workers
    /// emitting against the same `BrowserState.sparkline` slot.
    pub fn refresh_sparkline_for_selection(&mut self) -> Option<PendingIntent> {
        let next = self.browser.current_selection_for_sparkline();
        let current = self
            .browser
            .sparkline
            .as_ref()
            .map(|s| (s.kind, s.id.clone()));
        if next == current {
            return None;
        }
        match next {
            Some((kind, id)) => {
                let cadence = self.polling.cluster.status_history;
                self.browser.open_sparkline_for_selection(kind, id.clone());
                Some(PendingIntent::SpawnSparklineFetchLoop { kind, id, cadence })
            }
            None => {
                self.browser.close_sparkline();
                None
            }
        }
    }

    /// Reconcile `BrowserState.queue_listing` with the current Browser
    /// selection. Returns a `PendingIntent::SpawnQueueListingFetch` when
    /// the selection just landed on a Connection with `flow_files_queued > 0`.
    /// Returns `None` (and tears down any open listing) when the new
    /// selection is not a Connection or when the connection has no queued
    /// flowfiles. Also resets `listing_focused` to `false` on every
    /// selection change that changes the active queue.
    ///
    /// Re-creating the `QueueListingState` drops the prior handle, which
    /// fires the cleanup `DELETE` for the prior listing-request id.
    pub fn refresh_queue_listing_for_selection(&mut self) -> Option<PendingIntent> {
        let selection = self.browser.current_selection_for_queue_listing();
        let current = self
            .browser
            .queue_listing
            .as_ref()
            .map(|s| s.queue_id.clone());

        match selection {
            Some((queue_id, queue_name, queued_gt_zero)) => {
                // Same queue still selected — keep existing state.
                if Some(&queue_id) == current.as_ref() {
                    return None;
                }
                // Different connection (or first connection selected): drop prior
                // listing (if any), construct fresh pending state, reset focus,
                // and emit Spawn intent only if there are queued flowfiles.
                self.browser.listing_focused = false;
                self.browser.queue_listing = Some(
                    crate::view::browser::state::queue_listing::QueueListingState::pending(
                        queue_id.clone(),
                        queue_name.clone(),
                    ),
                );
                if queued_gt_zero {
                    Some(PendingIntent::SpawnQueueListingFetch {
                        queue_id,
                        queue_name,
                    })
                } else {
                    None
                }
            }
            None => {
                // Not a Connection (or no selection). Drop any prior listing.
                if self.browser.queue_listing.is_some() {
                    self.browser.listing_focused = false;
                    self.browser.queue_listing = None;
                    Some(PendingIntent::DropQueueListing)
                } else {
                    None
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerSeverity {
    Error,
    Warning,
    Info,
}

/// Outcome of processing one AppEvent.
#[derive(Debug, Default)]
pub struct UpdateResult {
    pub redraw: bool,
    pub intent: Option<PendingIntent>,
    /// Side-effect from the Tracer reducer that the app loop must dispatch.
    pub tracer_followup: Option<crate::view::tracer::state::Followup>,
    /// Secondary intent emitted by selection-change paths (Browser key
    /// dispatch, history navigation, cross-link landings) when the new
    /// selection requires the sparkline worker to be (re-)spawned. The
    /// app loop dispatches this alongside the primary `intent` field
    /// without disturbing existing single-intent semantics.
    pub sparkline_followup: Option<PendingIntent>,
    /// Secondary intent emitted by selection-change paths when the new
    /// selection is a Connection (or when navigating away from one) so the
    /// queue-listing worker can be spawned or torn down. Dispatched alongside
    /// `sparkline_followup` without disturbing existing intent semantics.
    pub queue_listing_followup: Option<PendingIntent>,
}

/// An intent the reducer wants the caller to dispatch. The caller owns the
/// dispatcher (because it holds async state we can't touch inside the
/// reducer). The reducer just describes what it wants.
#[derive(Debug)]
pub enum PendingIntent {
    SwitchContext(String),
    Goto(CrossLink),
    Dispatch(crate::intent::Intent),
    SaveEventContent(PendingSave),
    RunProvenanceQuery {
        query: crate::client::ProvenanceQuery,
    },
    SpawnModalChunks(Vec<crate::view::tracer::state::ModalFetchRequest>),
    /// Spawn an off-thread tabular decode (Parquet/Avro) via
    /// `tokio::task::spawn_blocking`. The dispatcher calls
    /// `classify_content` and emits `TracerPayload::ContentDecoded`.
    DecodeTabular {
        event_id: i64,
        side: crate::client::ContentSide,
        bytes: Vec<u8>,
    },
    /// Spawn the Browser version-control modal's one-shot
    /// identity+diff fetch worker. The handle is stored on
    /// `BrowserState.version_modal_handle` and aborted on close /
    /// refresh.
    SpawnVersionControlModalFetch {
        pg_id: String,
    },
    /// Spawn the Browser parameter-context modal's one-shot chain-fetch
    /// worker. The handle is stored on
    /// `BrowserState.parameter_modal_handle` and aborted on close /
    /// refresh.
    SpawnParameterContextModalFetch {
        pg_id: String,
        bound_context_id: String,
    },
    /// Spawn the Browser action-history modal's paginator worker.
    /// The handle is stored on `BrowserState.action_history_modal_handle`
    /// and aborted on close / refresh / tab switch / selection change.
    /// `fetch_signal` is the modal's `Notify` — the worker awaits on
    /// `notified()` to fetch the next page after the reducer fires
    /// `notify_one()` on scroll-near-tail.
    SpawnActionHistoryModalFetch {
        source_id: String,
        fetch_signal: std::sync::Arc<tokio::sync::Notify>,
    },
    /// Spawn the per-selection sparkline fetch loop. The dispatcher
    /// owns the spawn; the handle lands on
    /// `BrowserState.sparkline_handle` for abort-on-selection-change.
    SpawnSparklineFetchLoop {
        kind: crate::client::history::ComponentKind,
        id: String,
        cadence: std::time::Duration,
    },
    /// Spawn the queue-listing fetch worker for the currently selected
    /// Connection. The dispatcher calls `spawn_queue_listing_fetch` and
    /// stores the returned `QueueListingHandle` on
    /// `BrowserState.queue_listing.handle`. Emitted by
    /// `AppState::refresh_queue_listing_for_selection` when the selection
    /// lands on a Connection with `flow_files_queued > 0`.
    SpawnQueueListingFetch {
        queue_id: String,
        queue_name: String,
    },
    /// Re-spawn the queue-listing fetch worker for the same Connection
    /// that is already selected (used by the refresh chord `r` in T14).
    /// The dispatcher reuses `spawn_queue_listing_fetch` identically to
    /// `SpawnQueueListingFetch`.
    SpawnQueueListingRefresh {
        queue_id: String,
    },
    /// Spawn the flowfile-peek fetch worker for the selected row in the
    /// queue-listing panel. The dispatcher calls
    /// `spawn_flowfile_peek_fetch` and stores the returned `JoinHandle`
    /// on `BrowserState.queue_listing.peek.fetch_handle`.
    SpawnFlowfilePeekFetch {
        queue_id: String,
        uuid: String,
        cluster_node_id: Option<String>,
    },
    /// Emitted when the selection moves away from a Connection so the
    /// app loop can observe the intent (currently a no-op in the
    /// dispatcher — the reducer already cleared the field and the
    /// `QueueListingHandle` Drop impl fired the DELETE). Kept as an
    /// explicit variant so future observability hooks have a clean hook
    /// point.
    DropQueueListing,
    Quit,
}

/// Data needed to write raw content bytes to a file outside the
/// reducer. Fields identify the event whose content should be
/// re-fetched for the write; the reducer does not cache raw bytes
/// for this purpose — the worker fetches fresh when the save runs.
#[derive(Debug)]
pub struct PendingSave {
    pub path: std::path::PathBuf,
    pub event_id: i64,
    pub side: crate::client::ContentSide,
}

// ---------------------------------------------------------------------------
// Tab-history anchor helpers
// ---------------------------------------------------------------------------

/// Capture the current selection anchor for the active tab.
fn capture_anchor(state: &AppState) -> Option<crate::app::history::SelectionAnchor> {
    use crate::app::history::SelectionAnchor;
    match state.current_tab {
        ViewId::Browser => {
            state
                .browser
                .visible
                .get(state.browser.selected)
                .and_then(|&arena_idx| {
                    state
                        .browser
                        .nodes
                        .get(arena_idx)
                        .map(|n| SelectionAnchor::ComponentId(n.id.clone()))
                })
        }
        ViewId::Bulletins => Some(SelectionAnchor::RowIndex(state.bulletins.selected)),
        ViewId::Overview | ViewId::Events | ViewId::Tracer => None,
    }
}

/// Restore selection from a history entry's anchor. Returns an
/// optional `SpawnSparklineFetchLoop` intent the caller folds into
/// `UpdateResult.sparkline_followup` when the restore landed on a
/// Browser row whose `(kind, id)` differs from the active sparkline.
fn restore_anchor(
    state: &mut AppState,
    entry: &crate::app::history::HistoryEntry,
) -> Option<PendingIntent> {
    use crate::app::history::SelectionAnchor;
    match (&entry.anchor, entry.tab) {
        (Some(SelectionAnchor::ComponentId(id)), ViewId::Browser) => {
            let target = state.browser.nodes.iter().position(|n| n.id == *id);
            if let Some(arena_idx) = target {
                let mut cursor = state.browser.nodes[arena_idx].parent;
                while let Some(p) = cursor {
                    state.browser.expanded.insert(p);
                    cursor = state.browser.nodes[p].parent;
                }
                rebuild_visible(&mut state.browser);
                if let Some(pos) = state.browser.visible.iter().position(|&i| i == arena_idx) {
                    state.browser.selected = pos;
                }
                state.browser.emit_detail_request_for_current_selection();
                return state.refresh_sparkline_for_selection();
            }
            None
        }
        (Some(SelectionAnchor::RowIndex(idx)), ViewId::Bulletins) => {
            let max = state.bulletins.filtered_indices().len().saturating_sub(1);
            state.bulletins.selected = (*idx).min(max);
            None
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Top-level update() entry point
// ---------------------------------------------------------------------------

pub fn update(state: &mut AppState, event: AppEvent, config: &Config) -> UpdateResult {
    let prev_tab = state.current_tab;
    let result = update_inner(state, event, config);
    if prev_tab == ViewId::Events && state.current_tab != ViewId::Events {
        state.events.clear_failed_status();
    }
    result
}

fn update_inner(state: &mut AppState, event: AppEvent, config: &Config) -> UpdateResult {
    match event {
        AppEvent::Input(Event::Key(key)) => handle_key(state, key, config),
        AppEvent::Input(Event::Resize(_, _)) => UpdateResult {
            redraw: true,
            intent: None,
            tracer_followup: None,
            sparkline_followup: None,
            queue_listing_followup: None,
        },
        AppEvent::Input(_) => UpdateResult::default(),
        AppEvent::Tick => UpdateResult {
            redraw: false,
            intent: None,
            tracer_followup: None,
            sparkline_followup: None,
            queue_listing_followup: None,
        },
        AppEvent::Data(ViewPayload::Browser(payload)) => {
            handle_browser_payload(state, payload);
            state.last_refresh = Instant::now();
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }
        AppEvent::Data(ViewPayload::Tracer(payload)) => {
            // Intercept modal-chunk variants before the general apply_payload
            // dispatch so the reducers run with full AppState context (needed
            // for `tracer_config.ceiling` and the lazy diff
            // cache recompute).
            match payload {
                crate::event::TracerPayload::ModalChunk {
                    event_id,
                    side,
                    offset,
                    bytes,
                    eof,
                    requested_len,
                } => {
                    let cfg = state.tracer_config.ceiling.clone();
                    crate::view::tracer::state::apply_modal_chunk_with_ceiling(
                        &mut state.tracer,
                        event_id,
                        side,
                        offset,
                        bytes,
                        eof,
                        requested_len,
                        &cfg,
                    );
                    // For tabular content, check whether a deferred off-thread
                    // decode should be spawned now that the buffer is complete.
                    let decode_intent = crate::view::tracer::state::take_pending_tabular_decode(
                        &mut state.tracer,
                        event_id,
                        side,
                    )
                    .map(|(eid, s, bytes)| PendingIntent::DecodeTabular {
                        event_id: eid,
                        side: s,
                        bytes,
                    });
                    // Recompute diffability and (lazily) populate the
                    // diff cache after every chunk — independent of the
                    // currently active tab, since the user may switch to
                    // Diff after both sides have already loaded.
                    if let Some(modal) = state.tracer.content_modal.as_mut() {
                        crate::view::tracer::state::resolve_and_cache_diff(modal, &cfg);
                    }
                    state.last_refresh = Instant::now();
                    UpdateResult {
                        redraw: true,
                        intent: decode_intent,
                        tracer_followup: None,
                        sparkline_followup: None,
                        queue_listing_followup: None,
                    }
                }
                crate::event::TracerPayload::ContentDecoded {
                    event_id,
                    side,
                    render,
                } => {
                    let cfg = state.tracer_config.ceiling.clone();
                    crate::view::tracer::state::apply_tabular_decode_result(
                        &mut state.tracer,
                        event_id,
                        side,
                        render,
                    );
                    // Re-evaluate diff eligibility now that decoded is populated.
                    if let Some(modal) = state.tracer.content_modal.as_mut() {
                        crate::view::tracer::state::resolve_and_cache_diff(modal, &cfg);
                    }
                    state.last_refresh = Instant::now();
                    UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                        sparkline_followup: None,
                        queue_listing_followup: None,
                    }
                }
                crate::event::TracerPayload::ModalChunkFailed {
                    event_id,
                    side,
                    offset,
                    error,
                } => {
                    crate::view::tracer::state::apply_modal_chunk_failed(
                        &mut state.tracer,
                        event_id,
                        side,
                        offset,
                        error,
                    );
                    state.last_refresh = Instant::now();
                    UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                        sparkline_followup: None,
                        queue_listing_followup: None,
                    }
                }
                payload => {
                    // Mirror several tracer outcomes into the global status
                    // banner so the footer surfaces them the same way other
                    // tab errors do; in-pane Failed rendering still shows the
                    // detail locally.
                    match &payload {
                        crate::event::TracerPayload::EventDetailFailed { error, .. } => {
                            state.post_error(format!("event detail failed: {error}"), None);
                        }
                        crate::event::TracerPayload::ContentFailed { error, side, .. } => {
                            state.post_error(
                                format!("event {} content failed: {error}", side.as_str()),
                                None,
                            );
                        }
                        crate::event::TracerPayload::ContentSaved { path } => {
                            state.post_info(format!("saved to {}", path.display()));
                        }
                        crate::event::TracerPayload::ContentSaveFailed { path, error } => {
                            state.post_error(
                                format!("save to {} failed: {error}", path.display()),
                                None,
                            );
                        }
                        _ => {}
                    }
                    let followup =
                        crate::view::tracer::state::apply_payload(&mut state.tracer, payload);
                    state.last_refresh = Instant::now();
                    UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: followup,
                        sparkline_followup: None,
                        queue_listing_followup: None,
                    }
                }
            }
        }
        AppEvent::Data(ViewPayload::Events(payload)) => {
            // Mirror QueryFailed errors into the global status banner so
            // the footer surfaces them the same way other tab errors do.
            if let crate::event::EventsPayload::QueryFailed { error, .. } = &payload {
                state.post_error(format!("Events query failed: {error}"), None);
            }
            crate::view::events::state::apply_payload(&mut state.events, payload);
            state.last_refresh = Instant::now();
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }
        AppEvent::IntentOutcome(outcome) => handle_intent_outcome(state, outcome),
        // ClusterStore events are intercepted in the async main loop
        // (`src/app/mod.rs::run`) which owns `tx`; the reducer never
        // sees them. These arms exist only to keep `update_inner`
        // exhaustive and provide a graceful no-op if that invariant
        // ever slips. Task 1 keeps both arms inert — Tasks 3/5/7 may
        // trigger view-level reducer work on `ClusterChanged`.
        AppEvent::ClusterUpdate(_) | AppEvent::ClusterChanged(_) => UpdateResult {
            redraw: false,
            intent: None,
            tracer_followup: None,
            sparkline_followup: None,
            queue_listing_followup: None,
        },
        // Sparkline reducer arms. Both delegate to SparklineState's
        // `(kind, id)`-guarded apply methods so a stale emit between
        // worker abort and exit can't pollute the new selection's
        // strip. The outer `if let sparkline.as_mut()` adds defense-
        // in-depth: if the user navigated to an unsupported kind
        // (sparkline = None) and a stale emit arrives, drop silently.
        AppEvent::SparklineUpdate { kind, id, series } => {
            if let Some(sparkline) = state.browser.sparkline.as_mut() {
                sparkline.apply_update(kind, &id, series);
            }
            UpdateResult {
                redraw: state.current_tab == ViewId::Browser,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }
        AppEvent::SparklineEndpointMissing { kind, id } => {
            if let Some(sparkline) = state.browser.sparkline.as_mut() {
                sparkline.apply_endpoint_missing(kind, &id);
            }
            UpdateResult {
                redraw: state.current_tab == ViewId::Browser,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }
        AppEvent::Quit => {
            state.should_quit = true;
            UpdateResult {
                redraw: false,
                intent: Some(PendingIntent::Quit),
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Key dispatch
// ---------------------------------------------------------------------------

fn handle_key(state: &mut AppState, key: KeyEvent, config: &Config) -> UpdateResult {
    use crate::input::{AppAction, HistoryAction, InputEvent, TabAction};

    if matches!(key.code, KeyCode::F(12)) {
        let table = state.keymap.reverse_table();
        for (chord, source) in &table {
            tracing::info!(target: "nifi_lens::input", "{chord:12} = {source}");
        }
        for (endpoint, subs) in state.cluster.subscribers.debug_snapshot() {
            let subs_list: Vec<String> = subs.iter().map(|s| format!("{:?}", s.0)).collect();
            tracing::info!(
                target: "nifi_lens::input",
                endpoint = %endpoint,
                subscribers = ?subs_list,
                subs_count = subs.len(),
                next_interval_ms = state
                    .cluster
                    .snapshot
                    .next_interval_for(endpoint)
                    .map(|d| d.as_millis() as u64),
                "cluster endpoint state"
            );
        }
        return UpdateResult::default();
    }

    // Text-input bypass: if the active view is in text-input mode, skip
    // the KeyMap entirely and forward the raw KeyEvent. This preserves
    // the spec's rule that text-input modes own Esc/Enter semantics.
    if state.modal.is_none() && state.text_input_is_active() {
        let handler_result = dispatch_handler!(state.current_tab, handle_text_input, state, key);
        if let Some(r) = handler_result {
            return r;
        }
    }

    // translate() runs before view dispatch so the InputEvent is ready.
    // `content_modal_open` shadows outer-tab keys onto ContentModalVerb.
    // We also gate on `state.modal.is_none()` because an app-wide modal
    // (e.g. Save) can be opened FROM the content modal; while that modal
    // is up its input must not be swallowed by the content-modal shadow.
    let content_modal_open = state.current_tab == crate::app::state::ViewId::Tracer
        && state.tracer.content_modal.is_some()
        && state.modal.is_none();
    let version_modal_open = state.current_tab == crate::app::state::ViewId::Browser
        && state.browser.version_modal.is_some()
        && state.modal.is_none();
    let parameter_modal_open = state.current_tab == crate::app::state::ViewId::Browser
        && state.browser.parameter_modal.is_some()
        && state.modal.is_none();
    let action_history_modal_open = state.current_tab == crate::app::state::ViewId::Browser
        && state.browser.action_history_modal.is_some()
        && state.modal.is_none();
    let peek_modal_open = state.current_tab == crate::app::state::ViewId::Browser
        && state
            .browser
            .queue_listing
            .as_ref()
            .and_then(|s| s.peek.as_ref())
            .is_some()
        && state.modal.is_none();
    let input_event = state.keymap.translate(
        key,
        state.current_tab,
        content_modal_open,
        version_modal_open,
        parameter_modal_open,
        action_history_modal_open,
        peek_modal_open,
        state,
    );

    // Auto-clear non-Error banners on the next input event so info /
    // warning toasts don't linger after the user has moved on. Error
    // banners stay sticky — they're acknowledgements, not transients,
    // and may have detail the user wants to expand. Skip when the
    // input is `Focus(Ascend)` (Esc): the existing top-level handler
    // below dismisses the banner *and* short-circuits per-view
    // dispatch, so we mustn't clear it here or the view would also
    // consume the Ascend.
    if state.modal.is_none()
        && !matches!(
            input_event,
            InputEvent::Focus(crate::input::FocusAction::Ascend)
        )
        && state
            .status
            .banner
            .as_ref()
            .is_some_and(|b| b.severity != BannerSeverity::Error)
    {
        state.status.banner = None;
    }

    // Central dispatch for typed InputEvent variants. History / Tab / App
    // are handled here and return early. Focus / View dispatch to per-view
    // handlers. Unmapped falls through to the modal block then drops.
    match input_event {
        InputEvent::Unmapped => {
            // Key has no typed mapping — fall through to the modal block.
            // If no modal is active, the key is dropped.
        }
        InputEvent::History(HistoryAction::Back) => {
            if state.modal.is_some() {
                return UpdateResult::default();
            }
            let anchor = capture_anchor(state);
            let current = crate::app::history::HistoryEntry {
                tab: state.current_tab,
                anchor,
            };
            let mut sparkline_followup = None;
            let mut queue_listing_followup = None;
            if let Some(entry) = state.history.pop_back(current) {
                state.current_tab = entry.tab;
                sparkline_followup = restore_anchor(state, &entry);
                queue_listing_followup = state.refresh_queue_listing_for_selection();
            }
            return UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup,
                queue_listing_followup,
            };
        }
        InputEvent::History(HistoryAction::Forward) => {
            if state.modal.is_some() {
                return UpdateResult::default();
            }
            let anchor = capture_anchor(state);
            let current = crate::app::history::HistoryEntry {
                tab: state.current_tab,
                anchor,
            };
            let mut sparkline_followup = None;
            let mut queue_listing_followup = None;
            if let Some(entry) = state.history.pop_forward(current) {
                state.current_tab = entry.tab;
                sparkline_followup = restore_anchor(state, &entry);
                queue_listing_followup = state.refresh_queue_listing_for_selection();
            }
            return UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup,
                queue_listing_followup,
            };
        }
        InputEvent::Tab(TabAction::Goto(n)) => {
            if state.modal.is_some() || state.app_shortcuts_blocked() {
                return UpdateResult::default();
            }
            state.current_tab = match n {
                1 => ViewId::Overview,
                2 => ViewId::Bulletins,
                3 => ViewId::Browser,
                4 => ViewId::Events,
                5 => ViewId::Tracer,
                _ => return UpdateResult::default(),
            };
            return UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            };
        }
        InputEvent::App(AppAction::Quit) => {
            // Quit always fires, even in modal or text-input mode.
            state.should_quit = true;
            return UpdateResult {
                redraw: false,
                intent: Some(PendingIntent::Quit),
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            };
        }
        InputEvent::App(_) if state.modal.is_some() || state.app_shortcuts_blocked() => {
            // All other App actions fall through when a modal is active or
            // a per-view text-input mode is active:
            //  - modal active: let the modal handler run (e.g. `?` closes
            //    Help modal; `f` types into FuzzyFind query bar).
            //  - text-input active: let per-view handler capture the key
            //    (e.g. Shift+K is a printable char in the filter bar).
        }
        InputEvent::App(AppAction::Help) => {
            state.modal = Some(Modal::Help);
            return UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            };
        }
        InputEvent::App(AppAction::ContextSwitcher) => {
            let cs = ContextSwitcherState::from_config(
                config,
                &state.context_name,
                &state.detected_version,
            );
            state.modal = Some(Modal::ContextSwitcher(cs));
            return UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            };
        }
        InputEvent::App(AppAction::FuzzyFind) => {
            if state.flow_index.is_none() {
                state.post_warning("fuzzy find: flow not indexed yet, open Browser to seed".into());
                return UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                    sparkline_followup: None,
                    queue_listing_followup: None,
                };
            }
            let mut fs = crate::widget::fuzzy_find::FuzzyFindState::new();
            if let Some(idx) = state.flow_index.as_ref() {
                fs.rebuild_matches(idx);
            }
            state.modal = Some(Modal::FuzzyFind(fs));
            return UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            };
        }
        InputEvent::App(AppAction::Goto) => {
            let targets = state.selection_cross_links();
            match targets.len() {
                0 => {
                    // Nothing to goto — no-op.
                    return UpdateResult::default();
                }
                1 => {
                    // Single target: auto-goto without showing a menu.
                    if let Some(link) = build_go_crosslink(state, targets[0]) {
                        return UpdateResult {
                            redraw: true,
                            intent: Some(PendingIntent::Goto(link)),
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    return UpdateResult::default();
                }
                _ => {
                    // Multiple targets: open goto menu modal.
                    let subjects: Vec<crate::widget::goto_menu::GotoSubject> = targets
                        .iter()
                        .map(|t| {
                            build_goto_subject(state, *t)
                                .unwrap_or_else(crate::widget::goto_menu::GotoSubject::unknown)
                        })
                        .collect();
                    state.modal = Some(Modal::GotoMenu(
                        crate::widget::goto_menu::GotoMenuState::new(targets, subjects),
                    ));
                    return UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                        sparkline_followup: None,
                        queue_listing_followup: None,
                    };
                }
            }
        }
        InputEvent::App(AppAction::Paste) | InputEvent::App(AppAction::Cut) => {
            // Properly wired in Task 12 via text-input bypass
            return UpdateResult::default();
        }
        InputEvent::Focus(action) => {
            // Special-case the error banner as the outermost focus target:
            // Descend (Enter) expands it; Ascend (Esc) dismisses it.
            // These checks run before per-view dispatch so the banner takes
            // priority over any view's own Descend/Ascend handling.
            if state.modal.is_none() {
                if matches!(action, crate::input::FocusAction::Descend)
                    && state.open_banner_detail()
                {
                    return UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                        sparkline_followup: None,
                        queue_listing_followup: None,
                    };
                }
                if matches!(action, crate::input::FocusAction::Ascend)
                    && state.status.banner.is_some()
                {
                    state.status.banner = None;
                    return UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                        sparkline_followup: None,
                        queue_listing_followup: None,
                    };
                }
            }
            // Dispatch through typed handle_focus.
            if state.modal.is_none() {
                let consumed = dispatch_handler!(state.current_tab, handle_focus, state, action);
                if let Some(r) = consumed {
                    return r;
                }
                // Rule 1a: ported view returned None for Descend →
                // fall back to default_cross_link.
                if matches!(action, crate::input::FocusAction::Descend) {
                    let cross_target =
                        dispatch_handler!(state.current_tab, default_cross_link, state);
                    if let Some(target) = cross_target
                        && let Some(cross) = build_go_crosslink(state, target)
                    {
                        return UpdateResult {
                            redraw: true,
                            intent: Some(PendingIntent::Goto(cross)),
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                }
                // Unhandled actions fall through to the modal block then drop.
            }
        }
        InputEvent::View(verb) => {
            // Dispatch through typed handle_verb.
            if state.modal.is_none() {
                let consumed = dispatch_handler!(state.current_tab, handle_verb, state, verb);
                if let Some(r) = consumed {
                    return r;
                }
                // Unhandled verbs fall through.
            }
        }
    }

    // Modal-specific handling takes priority.
    if let Some(modal) = state.modal.as_mut() {
        match modal {
            Modal::Help => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('?')) {
                    state.modal = None;
                    return UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                        sparkline_followup: None,
                        queue_listing_followup: None,
                    };
                }
                return UpdateResult::default();
            }
            Modal::ErrorDetail => {
                if matches!(key.code, KeyCode::Esc) {
                    state.close_modal();
                    return UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                        sparkline_followup: None,
                        queue_listing_followup: None,
                    };
                }
                return UpdateResult::default();
            }
            Modal::ContextSwitcher(cs) => {
                if cs.entries.iter().any(|e| e.connecting) {
                    return UpdateResult::default();
                }
                match key.code {
                    KeyCode::Esc => {
                        state.modal = None;
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    KeyCode::Down => {
                        cs.move_cursor_down();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    KeyCode::Up => {
                        cs.move_cursor_up();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    KeyCode::Enter => {
                        if let Some(entry) = cs.selected() {
                            let name = entry.name.clone();
                            if let Some(e) = cs.entries.iter_mut().find(|e| e.name == name) {
                                e.connecting = true;
                            }
                            return UpdateResult {
                                redraw: true,
                                intent: Some(PendingIntent::SwitchContext(name)),
                                tracer_followup: None,
                                sparkline_followup: None,
                                queue_listing_followup: None,
                            };
                        }
                        return UpdateResult::default();
                    }
                    _ => return UpdateResult::default(),
                }
            }
            Modal::FuzzyFind(fs) => {
                match (key.code, key.modifiers) {
                    (KeyCode::Esc, _) => {
                        state.modal = None;
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    (KeyCode::Up, _) | (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
                        fs.move_up();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
                        fs.move_down();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    (KeyCode::Backspace, KeyModifiers::NONE) => {
                        fs.pop_char();
                        if let Some(idx) = state.flow_index.as_ref() {
                            fs.rebuild_matches(idx);
                        }
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    (KeyCode::Char(ch), KeyModifiers::NONE)
                    | (KeyCode::Char(ch), KeyModifiers::SHIFT) => {
                        fs.push_char(ch);
                        if let Some(idx) = state.flow_index.as_ref() {
                            fs.rebuild_matches(idx);
                        }
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    (KeyCode::Enter, _) => {
                        // Build the CrossLink from the selected match, if any.
                        let link = state
                            .flow_index
                            .as_ref()
                            .and_then(|idx| fs.selected_entry(idx))
                            .map(|entry| crate::intent::CrossLink::OpenInBrowser {
                                component_id: entry.id.clone(),
                                group_id: entry.group_id.clone(),
                            });
                        state.modal = None;
                        if let Some(link) = link {
                            return UpdateResult {
                                redraw: true,
                                intent: Some(PendingIntent::Goto(link)),
                                tracer_followup: None,
                                sparkline_followup: None,
                                queue_listing_followup: None,
                            };
                        }
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    _ => return UpdateResult::default(),
                }
            }
            Modal::Properties(ps) => {
                use crate::app::navigation::{CursorRef, ListNavigation};

                // Resolve current property list length for navigation bounds.
                // The renderer re-resolves the same (name, props) pair each
                // frame, so this is the authoritative length for clamping.
                let props_len: usize = match state.browser.details.get(&ps.arena_idx) {
                    Some(NodeDetail::Processor(p)) => p.properties.len(),
                    Some(NodeDetail::ControllerService(c)) => c.properties.len(),
                    _ => 0,
                };

                match (key.code, key.modifiers) {
                    (KeyCode::Esc, _) | (KeyCode::Char('p'), KeyModifiers::NONE) => {
                        state.modal = None;
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    (KeyCode::Up, _) => {
                        CursorRef::new(&mut ps.selected, props_len).move_up();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    (KeyCode::Down, _) => {
                        CursorRef::new(&mut ps.selected, props_len).move_down();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    (KeyCode::PageUp, _) => {
                        CursorRef::new(&mut ps.selected, props_len).page_up(10);
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    (KeyCode::PageDown, _) => {
                        CursorRef::new(&mut ps.selected, props_len).page_down(10);
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    (KeyCode::Home, _) => {
                        CursorRef::new(&mut ps.selected, props_len).goto_first();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    (KeyCode::End, _) => {
                        CursorRef::new(&mut ps.selected, props_len).goto_last();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    (KeyCode::Char('c'), KeyModifiers::NONE) => {
                        // Copy the value of the selected property row, if any.
                        let props: &[(String, String)] =
                            match state.browser.details.get(&ps.arena_idx) {
                                Some(NodeDetail::Processor(p)) => &p.properties,
                                Some(NodeDetail::ControllerService(c)) => &c.properties,
                                _ => &[],
                            };
                        if let Some((_, value)) = props.get(ps.selected) {
                            let value = value.clone();
                            let preview: String = value.chars().take(40).collect();
                            match state.copy_to_clipboard(value) {
                                Ok(()) => state.post_info(format!("copied: {preview}")),
                                Err(err) => state.post_warning(format!("clipboard: {err}")),
                            }
                        }
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    (KeyCode::Enter, _) => {
                        // Descend: if the selected row's value resolves to an
                        // arena node, close the modal and emit
                        // Goto(OpenInBrowser). If the value contains a
                        // #{name} parameter ref and the owning PG has a bound
                        // context, emit Goto(OpenParameterContextModal).
                        // Non-resolvable rows are a no-op.
                        let (props, owning_pg_id): (Vec<(String, String)>, String) =
                            match state.browser.details.get(&ps.arena_idx) {
                                Some(NodeDetail::Processor(p)) => {
                                    let pg = state
                                        .browser
                                        .nodes
                                        .iter()
                                        .find(|n| n.id == p.id)
                                        .map(|n| n.group_id.clone())
                                        .unwrap_or_default();
                                    (p.properties.clone(), pg)
                                }
                                Some(NodeDetail::ControllerService(c)) => {
                                    let pg = c.parent_group_id.clone().unwrap_or_default();
                                    (c.properties.clone(), pg)
                                }
                                _ => (Vec::new(), String::new()),
                            };
                        let Some((_, value)) = props.get(ps.selected) else {
                            return UpdateResult::default();
                        };
                        // UUID cross-link takes priority.
                        if let Some(resolved) = state.browser.resolve_id(value) {
                            let intent = PendingIntent::Goto(CrossLink::OpenInBrowser {
                                component_id: value.trim().to_string(),
                                group_id: resolved.group_id,
                            });
                            state.modal = None;
                            return UpdateResult {
                                redraw: true,
                                intent: Some(intent),
                                tracer_followup: None,
                                sparkline_followup: None,
                                queue_listing_followup: None,
                            };
                        }
                        // Parameter reference cross-link.
                        if state
                            .browser
                            .parameter_context_ref_for(&owning_pg_id)
                            .is_some()
                        {
                            use crate::view::browser::render::{ParamRefScan, scan_param_refs};
                            let preselect = match scan_param_refs(value.as_str()) {
                                ParamRefScan::Single { name } => Some(name),
                                ParamRefScan::Multiple => None,
                                ParamRefScan::None => return UpdateResult::default(),
                            };
                            let intent =
                                PendingIntent::Goto(CrossLink::OpenParameterContextModal {
                                    pg_id: owning_pg_id,
                                    preselect,
                                });
                            state.modal = None;
                            return UpdateResult {
                                redraw: true,
                                intent: Some(intent),
                                tracer_followup: None,
                                sparkline_followup: None,
                                queue_listing_followup: None,
                            };
                        }
                        return UpdateResult::default();
                    }
                    _ => return UpdateResult::default(),
                }
            }
            Modal::SaveEventContent(save) => {
                match key.code {
                    KeyCode::Esc => {
                        state.modal = None;
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    KeyCode::Backspace => {
                        save.backspace();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        save.push_char(ch);
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    KeyCode::Enter => {
                        // Extract the path before dropping the mutable borrow.
                        let path = std::path::PathBuf::from(&save.path);
                        // Build a PendingSave from the content pane's event id + side.
                        let pending = tracer::build_pending_save(state, path);
                        state.modal = None;
                        return UpdateResult {
                            redraw: true,
                            intent: pending,
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    _ => return UpdateResult::default(),
                }
            }
            Modal::NodeDetail(_) => {
                if matches!(key.code, KeyCode::Esc) {
                    state.modal = None;
                    return UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                        sparkline_followup: None,
                        queue_listing_followup: None,
                    };
                }
                return UpdateResult::default();
            }
            Modal::GotoMenu(jm) => match key.code {
                KeyCode::Esc => {
                    state.modal = None;
                    return UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                        sparkline_followup: None,
                        queue_listing_followup: None,
                    };
                }
                KeyCode::Up => {
                    jm.move_up();
                    return UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                        sparkline_followup: None,
                        queue_listing_followup: None,
                    };
                }
                KeyCode::Down => {
                    jm.move_down();
                    return UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                        sparkline_followup: None,
                        queue_listing_followup: None,
                    };
                }
                KeyCode::Enter => {
                    let maybe_target = jm.selected_target();
                    state.modal = None;
                    if let Some(link) = maybe_target.and_then(|t| build_go_crosslink(state, t)) {
                        return UpdateResult {
                            redraw: true,
                            intent: Some(PendingIntent::Goto(link)),
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        };
                    }
                    return UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                        sparkline_followup: None,
                        queue_listing_followup: None,
                    };
                }
                _ => return UpdateResult::default(),
            },
        }
    }

    UpdateResult::default()
}

// ---------------------------------------------------------------------------
// Go cross-link builder
// ---------------------------------------------------------------------------

fn build_go_crosslink(state: &AppState, target: crate::input::GoTarget) -> Option<CrossLink> {
    use crate::input::GoTarget;

    match (state.current_tab, target) {
        (ViewId::Bulletins, GoTarget::Browser) => {
            let idx = state.bulletins.selected_ring_index()?;
            let b = state.bulletins.ring.get(idx)?;
            Some(CrossLink::OpenInBrowser {
                component_id: b.source_id.clone(),
                group_id: b.group_id.clone(),
            })
        }
        (ViewId::Bulletins, GoTarget::Events) => {
            let idx = state.bulletins.selected_ring_index()?;
            let b = state.bulletins.ring.get(idx)?;
            Some(CrossLink::GotoEvents {
                component_id: b.source_id.clone(),
            })
        }
        (ViewId::Browser, GoTarget::Events) => {
            // Listing focused: hand off the *selected flowfile's
            // upstream component* to Events, so the operator sees
            // events for the connection's source. Tree focused: use
            // the tree node id.
            if state.browser.listing_focused {
                let listing = state.browser.queue_listing.as_ref()?;
                let visible = listing.visible_indices();
                let &row_idx = visible.get(listing.selected)?;
                listing.rows.get(row_idx)?;
                // The connection node carries the source/dest component
                // ids on its detail row; use the connection's source.
                let arena_idx = *state.browser.visible.get(state.browser.selected)?;
                let detail = state.browser.details.get(&arena_idx)?;
                let crate::view::browser::state::NodeDetail::Connection(c) = detail else {
                    return None;
                };
                return Some(CrossLink::GotoEvents {
                    component_id: c.source_id.clone(),
                });
            }
            let arena_idx = *state.browser.visible.get(state.browser.selected)?;
            let node = state.browser.nodes.get(arena_idx)?;
            // Folders are a reducer-only UI construct with a synthetic id;
            // they never map to a real NiFi component, so they cannot be
            // cross-link targets.
            if matches!(node.kind, crate::client::NodeKind::Folder(_)) {
                return None;
            }
            Some(CrossLink::GotoEvents {
                component_id: node.id.clone(),
            })
        }
        (ViewId::Browser, GoTarget::Tracer) => {
            // Tracer is only reachable from listing focus — the
            // selected row's flowfile uuid kicks off lineage. Tree
            // focus has no flowfile-uuid context.
            if !state.browser.listing_focused {
                return None;
            }
            let listing = state.browser.queue_listing.as_ref()?;
            let visible = listing.visible_indices();
            let &row_idx = visible.get(listing.selected)?;
            let row = listing.rows.get(row_idx)?;
            Some(CrossLink::TraceByUuid {
                uuid: row.uuid.clone(),
            })
        }
        (ViewId::Events, GoTarget::Browser) => {
            let event = state.events.selected_event()?;
            Some(CrossLink::OpenInBrowser {
                component_id: event.component_id.clone(),
                group_id: event.group_id.clone(),
            })
        }
        (ViewId::Events, GoTarget::Tracer) => {
            let event = state.events.selected_event()?;
            Some(CrossLink::TraceByUuid {
                uuid: event.flow_file_uuid.clone(),
            })
        }
        (ViewId::Tracer, GoTarget::Browser) => {
            let component_id = state.tracer.selected_component_id()?;
            Some(CrossLink::OpenInBrowser {
                component_id,
                group_id: String::new(),
            })
        }
        (ViewId::Tracer, GoTarget::Events) => {
            let component_id = state.tracer.selected_component_id()?;
            Some(CrossLink::GotoEvents { component_id })
        }
        (ViewId::Overview, GoTarget::Browser) => match state.overview.focus {
            OverviewFocus::Noisy => {
                let n = state.overview.noisy.get(state.overview.noisy_selected)?;
                Some(CrossLink::OpenInBrowser {
                    component_id: n.source_id.clone(),
                    group_id: n.group_id.clone(),
                })
            }
            OverviewFocus::Queues => {
                let q = state
                    .overview
                    .unhealthy
                    .get(state.overview.queues_selected)?;
                // Connections are not nodes in the browser tree — navigate to
                // the process group that owns the connection instead.
                Some(CrossLink::OpenInBrowser {
                    component_id: q.group_id.clone(),
                    group_id: q.group_id.clone(),
                })
            }
            _ => None,
        },
        (ViewId::Overview, GoTarget::Events) => {
            if state.overview.focus == OverviewFocus::Noisy {
                let n = state.overview.noisy.get(state.overview.noisy_selected)?;
                Some(CrossLink::GotoEvents {
                    component_id: n.source_id.clone(),
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

fn build_goto_subject(
    state: &AppState,
    target: crate::input::GoTarget,
) -> Option<crate::widget::goto_menu::GotoSubject> {
    use crate::input::GoTarget;
    use crate::widget::goto_menu::GotoSubject;

    match (state.current_tab, target) {
        (ViewId::Bulletins, GoTarget::Browser) | (ViewId::Bulletins, GoTarget::Events) => {
            let idx = state.bulletins.selected_ring_index()?;
            let b = state.bulletins.ring.get(idx)?;
            Some(GotoSubject::Component {
                name: b.source_name.clone(),
                id: b.source_id.clone(),
            })
        }
        (ViewId::Browser, GoTarget::Events) => {
            // When listing is focused, the goto-menu subject is the
            // upstream component's events context — labelled by the
            // selected row's filename for operator clarity.
            if state.browser.listing_focused {
                let listing = state.browser.queue_listing.as_ref()?;
                let visible = listing.visible_indices();
                let &row_idx = visible.get(listing.selected)?;
                let row = listing.rows.get(row_idx)?;
                let arena_idx = *state.browser.visible.get(state.browser.selected)?;
                let detail = state.browser.details.get(&arena_idx)?;
                let crate::view::browser::state::NodeDetail::Connection(c) = detail else {
                    return None;
                };
                return Some(GotoSubject::Component {
                    name: row
                        .filename
                        .clone()
                        .unwrap_or_else(|| row.uuid.chars().take(8).collect()),
                    id: c.source_id.clone(),
                });
            }
            let arena_idx = *state.browser.visible.get(state.browser.selected)?;
            let node = state.browser.nodes.get(arena_idx)?;
            // Folders are a reducer-only UI construct; no real component to jump to.
            if matches!(node.kind, crate::client::NodeKind::Folder(_)) {
                return None;
            }
            Some(GotoSubject::Component {
                name: node.name.clone(),
                id: node.id.clone(),
            })
        }
        (ViewId::Browser, GoTarget::Tracer) => {
            if !state.browser.listing_focused {
                return None;
            }
            let listing = state.browser.queue_listing.as_ref()?;
            let visible = listing.visible_indices();
            let &row_idx = visible.get(listing.selected)?;
            let row = listing.rows.get(row_idx)?;
            Some(GotoSubject::Flowfile {
                uuid: row.uuid.clone(),
            })
        }
        (ViewId::Events, GoTarget::Browser) => {
            let event = state.events.selected_event()?;
            Some(GotoSubject::Component {
                name: event.component_name.clone(),
                id: event.component_id.clone(),
            })
        }
        (ViewId::Events, GoTarget::Tracer) => {
            let event = state.events.selected_event()?;
            Some(GotoSubject::Flowfile {
                uuid: event.flow_file_uuid.clone(),
            })
        }
        (ViewId::Tracer, GoTarget::Browser) | (ViewId::Tracer, GoTarget::Events) => {
            let id = state.tracer.selected_component_id()?;
            let name = state.tracer.selected_component_label().unwrap_or_default();
            Some(GotoSubject::Component { name, id })
        }
        (ViewId::Overview, GoTarget::Browser) => match state.overview.focus {
            OverviewFocus::Noisy => {
                let n = state.overview.noisy.get(state.overview.noisy_selected)?;
                Some(GotoSubject::Component {
                    name: n.source_name.clone(),
                    id: n.source_id.clone(),
                })
            }
            OverviewFocus::Queues => {
                let q = state
                    .overview
                    .unhealthy
                    .get(state.overview.queues_selected)?;
                // Connection name is the anchor the user selected; the
                // jump itself routes to the owning PG (see `build_go_crosslink`).
                Some(GotoSubject::Component {
                    name: q.source_name.clone(),
                    id: q.group_id.clone(),
                })
            }
            _ => None,
        },
        (ViewId::Overview, GoTarget::Events) => {
            if state.overview.focus == OverviewFocus::Noisy {
                let n = state.overview.noisy.get(state.overview.noisy_selected)?;
                Some(GotoSubject::Component {
                    name: n.source_name.clone(),
                    id: n.source_id.clone(),
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Helpers shared across sub-modules
// ---------------------------------------------------------------------------

pub(crate) fn handle_browser_payload(state: &mut AppState, payload: crate::event::BrowserPayload) {
    use crate::event::BrowserPayload;
    match payload {
        BrowserPayload::Detail(detail) => {
            crate::view::browser::state::apply_node_detail(&mut state.browser, *detail);
        }
        BrowserPayload::VersionControlModalLoaded {
            pg_id,
            identity,
            differences,
        } => {
            state
                .browser
                .apply_version_control_modal_loaded(pg_id, identity, differences);
            state.browser.version_modal_handle = None;
        }
        BrowserPayload::VersionControlModalFailed { pg_id, err } => {
            state.browser.apply_version_control_modal_failed(pg_id, err);
            state.browser.version_modal_handle = None;
        }
        BrowserPayload::ParameterContextModalLoaded { pg_id, chain } => {
            state
                .browser
                .apply_parameter_context_modal_loaded(pg_id, chain);
            state.browser.parameter_modal_handle = None;
        }
        BrowserPayload::ParameterContextModalFailed { pg_id, err } => {
            state
                .browser
                .apply_parameter_context_modal_failed(pg_id, err);
            state.browser.parameter_modal_handle = None;
        }
        BrowserPayload::ActionHistoryPage {
            source_id,
            offset: _,
            actions,
            total,
        } => {
            if let Some(modal) = state.browser.action_history_modal.as_mut() {
                modal.apply_page(&source_id, actions, total);
            }
        }
        BrowserPayload::ActionHistoryError { source_id, err } => {
            if let Some(modal) = state.browser.action_history_modal.as_mut()
                && modal.source_id == source_id
            {
                modal.error = Some(err);
                modal.loading = false;
                state.browser.action_history_modal_handle = None;
            }
        }
        BrowserPayload::QueueListingRequestIdAssigned {
            queue_id,
            request_id,
        } => {
            if let Some(s) = state.browser.queue_listing.as_mut()
                && s.queue_id == queue_id
            {
                s.request_id = Some(request_id);
            }
        }
        BrowserPayload::QueueListingProgress { queue_id, percent } => {
            if let Some(s) = state.browser.queue_listing.as_mut() {
                s.apply_progress(&queue_id, percent);
            }
        }
        BrowserPayload::QueueListingComplete {
            queue_id,
            rows,
            total,
            truncated,
        } => {
            if let Some(s) = state.browser.queue_listing.as_mut() {
                s.apply_complete(&queue_id, rows, total, truncated);
            }
        }
        BrowserPayload::QueueListingError { queue_id, err } => {
            if let Some(s) = state.browser.queue_listing.as_mut() {
                s.apply_error(&queue_id, err);
            }
        }
        BrowserPayload::QueueListingTimeout { queue_id } => {
            if let Some(s) = state.browser.queue_listing.as_mut() {
                s.apply_timeout(&queue_id);
            }
        }
        BrowserPayload::FlowfilePeek {
            queue_id,
            uuid,
            attrs,
            content_claim,
            mime_type,
        } => {
            if let Some(listing) = state.browser.queue_listing.as_mut()
                && let Some(peek) = listing.peek.as_mut()
            {
                peek.apply_peek(&queue_id, &uuid, attrs, content_claim, mime_type);
            }
        }
        BrowserPayload::FlowfilePeekError {
            queue_id,
            uuid,
            err,
        } => {
            // Surface the failure as a status-line error banner so
            // the operator sees it without having to read the modal's
            // chip strip. The full NiFi error message is pushed into
            // the banner detail (Enter to expand). The peek modal
            // stays open showing the identity (uuid/filename/etc.)
            // so the user knows which flowfile failed; Esc closes.
            let short_uuid: String = uuid.chars().take(8).collect();
            let banner = format!("flowfile peek {short_uuid}…: {err}");
            state.post_error(banner, Some(err.clone()));
            if let Some(listing) = state.browser.queue_listing.as_mut()
                && let Some(peek) = listing.peek.as_mut()
            {
                peek.apply_error(&queue_id, &uuid, err);
            }
        }
    }
}

fn handle_intent_outcome(
    state: &mut AppState,
    outcome: Result<IntentOutcome, NifiLensError>,
) -> UpdateResult {
    match outcome {
        Ok(IntentOutcome::ContextSwitched {
            new_context_name,
            new_version,
            new_base_url,
        }) => {
            state.context_name = new_context_name;
            state.detected_version = new_version;
            state.last_refresh = Instant::now();
            state.modal = None;
            state.status.banner = None;

            // Clear all per-view state so stale data from the previous
            // context doesn't linger until the next worker poll.
            let ring_cap = state.bulletins.ring_capacity;
            state.overview = OverviewState::new();
            state.bulletins = BulletinsState::with_capacity(ring_cap);
            state.browser = BrowserState::new();
            state.events = EventsState::new();
            state.tracer = TracerState::new();
            state.flow_index = None;
            state.cluster_summary = ClusterSummary::default();
            state.history = crate::app::history::TabHistory::default();

            // Signal the app loop to force-restart the current view worker.
            state.pending_worker_restart = true;

            // Tear down cluster fetchers bound to the previous client.
            // The app loop will respawn them against the new client in
            // the `pending_worker_restart` branch.
            state.cluster.shutdown();
            state.cluster = crate::cluster::ClusterStore::new(
                state.polling.cluster.clone(),
                ring_cap,
                new_base_url,
            );

            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }
        Ok(IntentOutcome::ViewRefreshed { .. }) => {
            state.last_refresh = Instant::now();
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }
        Ok(IntentOutcome::Quitting) => {
            state.should_quit = true;
            UpdateResult {
                redraw: false,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }
        Ok(IntentOutcome::NotImplemented { intent_name }) => {
            state.post_info(format!("{intent_name}: not yet implemented"));
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }
        Ok(IntentOutcome::OpenInBrowserTarget {
            component_id,
            group_id: _group_id,
        }) => {
            let anchor = capture_anchor(state);
            state.history.push(crate::app::history::HistoryEntry {
                tab: state.current_tab,
                anchor,
            });
            state.current_tab = ViewId::Browser;
            state.close_modal();
            // Walk the arena for any node matching the component id.
            let target_arena = state
                .browser
                .nodes
                .iter()
                .position(|n| n.id == component_id);
            let Some(arena_idx) = target_arena else {
                state.post_warning(format!(
                    "component {component_id} not found in current flow tree"
                ));
                return UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                    sparkline_followup: None,
                    queue_listing_followup: None,
                };
            };
            // Expand every ancestor.
            let mut cursor = state.browser.nodes[arena_idx].parent;
            while let Some(p) = cursor {
                state.browser.expanded.insert(p);
                cursor = state.browser.nodes[p].parent;
            }
            rebuild_visible(&mut state.browser);
            if let Some(pos) = state.browser.visible.iter().position(|&i| i == arena_idx) {
                state.browser.selected = pos;
            }
            state.browser.emit_detail_request_for_current_selection();
            let __sparkline_fu = state.refresh_sparkline_for_selection();
            let __queue_listing_fu = state.refresh_queue_listing_for_selection();
            state.status.banner = None;
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: __sparkline_fu,
                queue_listing_followup: __queue_listing_fu,
            }
        }
        Ok(IntentOutcome::TracerLandingOn { component_id }) => {
            let anchor = capture_anchor(state);
            state.history.push(crate::app::history::HistoryEntry {
                tab: state.current_tab,
                anchor,
            });
            use crate::view::tracer::state::start_latest_events;
            start_latest_events(&mut state.tracer, component_id);
            state.current_tab = ViewId::Tracer;
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }
        Ok(IntentOutcome::TracerLineageStarted { uuid, abort }) => {
            state.current_tab = ViewId::Tracer;
            use crate::view::tracer::state::start_lineage;
            start_lineage(&mut state.tracer, uuid, Some(abort));
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }
        Ok(IntentOutcome::TracerInputInvalid { raw }) => {
            state.post_warning(format!("invalid UUID: {raw}"));
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }
        Ok(IntentOutcome::EventsLandingOn { component_id }) => {
            // Switch to Events, seed filters, and auto-run.
            state.current_tab = ViewId::Events;
            state.events.filters.source = component_id;
            state.events.filters.time = "last 15m".to_string();
            state.events.status = crate::view::events::state::EventsQueryStatus::Running {
                query_id: None,
                submitted_at: std::time::SystemTime::now(),
                percent: 0,
            };
            state.events.events.clear();
            state.events.selected_row = None;
            let query = state.events.build_query();
            UpdateResult {
                redraw: true,
                intent: Some(PendingIntent::RunProvenanceQuery { query }),
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }
        Ok(IntentOutcome::OpenParameterContextModalTarget { pg_id, preselect }) => {
            // Look up the PG's display name from the arena; fall back to
            // the bare id if the PG isn't found (race between navigation
            // and the arena rebuild).
            let pg_path = state
                .browser
                .pg_name_for(&pg_id)
                .unwrap_or(&pg_id)
                .to_string();
            // Look up the bound context id before opening the modal.
            // If the binding isn't known yet (race with the fetcher),
            // open the modal in Error state rather than firing a
            // worker with an unknown id.
            let bound_context_id = state
                .browser
                .parameter_context_ref_for(&pg_id)
                .map(|r| r.id.clone());
            state
                .browser
                .open_parameter_context_modal(pg_id.clone(), pg_path, preselect);
            if let Some(bound_context_id) = bound_context_id {
                UpdateResult {
                    redraw: true,
                    intent: Some(PendingIntent::SpawnParameterContextModalFetch {
                        pg_id,
                        bound_context_id,
                    }),
                    tracer_followup: None,
                    sparkline_followup: None,
                    queue_listing_followup: None,
                }
            } else {
                // No binding known yet; mark the modal as failed so the
                // user sees a message rather than a spinner that never
                // resolves. They can press `r` once the bindings fetch
                // completes.
                state.browser.apply_parameter_context_modal_failed(
                    pg_id,
                    "no bound parameter context found".into(),
                );
                UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                    sparkline_followup: None,
                    queue_listing_followup: None,
                }
            }
        }
        Err(err) => {
            state.post_error(err.to_string(), Some(format!("{err:?}")));
            // Close the context switcher modal so the banner is visible.
            if matches!(state.modal, Some(Modal::ContextSwitcher(_))) {
                state.modal = None;
            }
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }
    }
}

/// Copies `text` to the system clipboard, setting a banner on success or failure.
///
/// Routes through [`AppState::copy_to_clipboard`] so every clipboard
/// write in the app shares the same persistent `arboard` handle.
fn clipboard_copy(state: &mut AppState, text: &str) {
    let preview = text.to_string();
    match state.copy_to_clipboard(text.to_string()) {
        Ok(()) => {
            state.post_info(format!("copied: {preview}"));
        }
        Err(err) => {
            state.post_warning(format!("clipboard: {err}"));
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
