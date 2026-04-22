//! AppState and the pure state reducer.
//!
//! The reducer folds AppEvent into AppState and returns whether a redraw
//! is needed. State is owned exclusively by the UI task.

mod browser;
mod bulletins;
mod events;
mod overview;
mod tracer;

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
/// Populated by the Overview worker in Phase 3. In Phase 1 the fields
/// stay `None` and the top-bar renders `nodes ?/?` as a muted placeholder.
#[derive(Debug, Default, Clone)]
pub struct ClusterSummary {
    pub connected_nodes: Option<usize>,
    pub total_nodes: Option<usize>,
}

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
    pub fn new(context_name: String, detected_version: Version, config: &Config) -> Self {
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
            cluster: crate::cluster::ClusterStore::new(
                config.polling.cluster.clone(),
                config.bulletins.ring_size,
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
    pub fn copy_to_clipboard(&mut self, text: String) -> Result<(), String> {
        if self.clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(cb) => {
                    self.clipboard = Some(ClipboardHandle(cb));
                }
                Err(err) => return Err(err.to_string()),
            }
        }
        let handle = self
            .clipboard
            .as_mut()
            .ok_or_else(|| "clipboard handle unavailable".to_string())?;
        handle.0.set_text(text).map_err(|e| e.to_string())
    }

    /// Read a string from the system clipboard.
    ///
    /// Uses the same persistent `arboard` handle as `copy_to_clipboard`,
    /// lazily initializing it on first use.
    pub fn get_from_clipboard(&mut self) -> Result<String, String> {
        if self.clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(cb) => {
                    self.clipboard = Some(ClipboardHandle(cb));
                }
                Err(err) => return Err(err.to_string()),
            }
        }
        let handle = self
            .clipboard
            .as_mut()
            .ok_or_else(|| "clipboard handle unavailable".to_string())?;
        handle.0.get_text().map_err(|e| e.to_string())
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
    NodeDetail(Box<crate::client::health::NodeHealthRow>),
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
// Hint collection
// ---------------------------------------------------------------------------

/// Collect the hint spans for the current state, respecting modal priority.
pub fn collect_hints(state: &AppState) -> Vec<crate::widget::hint_bar::HintSpan> {
    use std::borrow::Cow;

    use crate::input::{AppAction, BrowserVerb, BulletinsVerb, EventsVerb, TracerVerb, Verb};
    use crate::widget::hint_bar::HintSpan;

    // Modal-priority hints remain hand-written because they're short
    // and context-specific.
    if let Some(ref modal) = state.modal {
        return modal_hints(modal);
    }

    // Text-input-focused views show their own edit-mode hint strip.
    // The keymap is bypassed in this mode; the hint bar advertises
    // the conventional type/apply/cancel contract.
    if state.text_input_is_active() {
        return vec![
            HintSpan {
                key: Cow::Borrowed("type"),
                action: Cow::Borrowed("filter"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("Enter"),
                action: Cow::Borrowed("apply"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("Esc"),
                action: Cow::Borrowed("cancel"),
                enabled: true,
            },
        ];
    }

    // Default path: tab-specific verbs only. General navigation,
    // history, tab cycling, fuzzy find, quit, and the help modal are
    // documented via `?` — no point repeating them in every frame.
    let ctx = crate::input::HintContext::new(state);
    let mut out: Vec<HintSpan> = Vec::new();

    fn push_verb<V: crate::input::Verb>(
        out: &mut Vec<HintSpan>,
        v: V,
        ctx: &crate::input::HintContext<'_>,
    ) {
        if !v.show_in_hint_bar() {
            return;
        }
        out.push(HintSpan {
            key: Cow::Owned(v.chord().display()),
            action: Cow::Borrowed(v.hint()),
            enabled: v.enabled(ctx),
        });
    }

    // Per-view verbs — these are the tab-specific commands. Disabled
    // verbs (e.g. Browser Properties with no eligible selection) stay
    // in the bar but render dim, so users learn what's possible.
    match state.current_tab {
        ViewId::Overview => {}
        ViewId::Bulletins => {
            for &v in BulletinsVerb::all() {
                push_verb(&mut out, v, &ctx);
            }
        }
        ViewId::Browser => {
            for &v in BrowserVerb::all() {
                push_verb(&mut out, v, &ctx);
            }
        }
        ViewId::Events => {
            for &v in EventsVerb::all() {
                push_verb(&mut out, v, &ctx);
            }
        }
        ViewId::Tracer => {
            for &v in TracerVerb::all() {
                push_verb(&mut out, v, &ctx);
            }
            // Override the "save" hint label when the content pane shows a
            // truncated preview — append "(fetches full <total>)" so the user
            // knows the save action will re-fetch the complete content.
            let save_label: Option<Cow<'static, str>> = {
                use crate::client::ContentSide;
                use crate::view::tracer::state::{ContentPane, EventDetail, TracerMode};
                if let TracerMode::Lineage(ref view) = state.tracer.mode
                    && let EventDetail::Loaded {
                        event,
                        content:
                            ContentPane::Shown {
                                truncated: true,
                                side,
                                ..
                            },
                    } = &view.event_detail
                {
                    let total_size = match side {
                        ContentSide::Input => event.input_size,
                        ContentSide::Output => event.output_size,
                    };
                    Some(match total_size {
                        Some(total) => Cow::Owned(format!(
                            "save (fetches full {})",
                            crate::view::tracer::render::human_bytes(total),
                        )),
                        None => Cow::Borrowed("save (fetches full)"),
                    })
                } else {
                    None
                }
            };
            // Rewrite the save hint span that was just pushed, but only when
            // the label differs from the static default (i.e. content is truncated).
            if let Some(label) = save_label
                && let Some(span) = out.iter_mut().rev().find(|s| s.action == "save")
            {
                span.action = label;
            }
        }
    }

    // Cross-tab goto — show when the current selection has at least one
    // actionable destination so the bar doesn't advertise a dead combo.
    push_verb(&mut out, AppAction::Goto, &ctx);

    // Trailing `?` pointer so users always know where to find the
    // full reference. Everything else (navigation, history, tab
    // cycling, quit, fuzzy find, context switcher) lives in the help
    // modal.
    out.push(HintSpan {
        key: Cow::Borrowed("?"),
        action: Cow::Borrowed("help"),
        enabled: true,
    });

    out
}

fn modal_hints(modal: &Modal) -> Vec<crate::widget::hint_bar::HintSpan> {
    use std::borrow::Cow;

    use crate::widget::hint_bar::HintSpan;
    match modal {
        Modal::Help => vec![HintSpan {
            key: Cow::Borrowed("Esc"),
            action: Cow::Borrowed("close"),
            enabled: true,
        }],
        Modal::ContextSwitcher(_) => vec![
            HintSpan {
                key: Cow::Borrowed("\u{2191}/\u{2193}"),
                action: Cow::Borrowed("nav"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("Enter"),
                action: Cow::Borrowed("switch"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("Esc"),
                action: Cow::Borrowed("cancel"),
                enabled: true,
            },
        ],
        Modal::FuzzyFind(_) => vec![
            HintSpan {
                key: Cow::Borrowed("type"),
                action: Cow::Borrowed("filter"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("Enter"),
                action: Cow::Borrowed("select"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("Esc"),
                action: Cow::Borrowed("cancel"),
                enabled: true,
            },
        ],
        Modal::Properties(_) => vec![
            HintSpan {
                key: Cow::Borrowed("\u{2191}/\u{2193}"),
                action: Cow::Borrowed("nav"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("Enter"),
                action: Cow::Borrowed("goto"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("c"),
                action: Cow::Borrowed("copy value"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("Esc"),
                action: Cow::Borrowed("close"),
                enabled: true,
            },
        ],
        Modal::ErrorDetail => vec![HintSpan {
            key: Cow::Borrowed("Esc"),
            action: Cow::Borrowed("close"),
            enabled: true,
        }],
        Modal::SaveEventContent(_) => vec![
            HintSpan {
                key: Cow::Borrowed("Enter"),
                action: Cow::Borrowed("confirm"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("Esc"),
                action: Cow::Borrowed("cancel"),
                enabled: true,
            },
        ],
        Modal::NodeDetail(_) => vec![HintSpan {
            key: Cow::Borrowed("Esc"),
            action: Cow::Borrowed("close"),
            enabled: true,
        }],
        // Task 11 adds full hint spans for the goto menu.
        Modal::GotoMenu(_) => vec![
            HintSpan {
                key: Cow::Borrowed("\u{2191}/\u{2193}"),
                action: Cow::Borrowed("nav"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("Enter"),
                action: Cow::Borrowed("goto"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("Esc"),
                action: Cow::Borrowed("cancel"),
                enabled: true,
            },
        ],
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) fn modal_hints_for_test(state: &AppState) -> Vec<crate::widget::hint_bar::HintSpan> {
    state.modal.as_ref().map(modal_hints).unwrap_or_default()
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

/// Restore selection from a history entry's anchor.
fn restore_anchor(state: &mut AppState, entry: &crate::app::history::HistoryEntry) {
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
            }
        }
        (Some(SelectionAnchor::RowIndex(idx)), ViewId::Bulletins) => {
            let max = state.bulletins.filtered_indices().len().saturating_sub(1);
            state.bulletins.selected = (*idx).min(max);
        }
        _ => {}
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
        },
        AppEvent::Input(_) => UpdateResult::default(),
        AppEvent::Tick => UpdateResult {
            redraw: false,
            intent: None,
            tracer_followup: None,
        },
        AppEvent::Data(ViewPayload::Browser(payload)) => {
            handle_browser_payload(state, payload);
            state.last_refresh = Instant::now();
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }
        AppEvent::Data(ViewPayload::Tracer(payload)) => {
            // Mirror several tracer outcomes into the global status banner
            // so the footer surfaces them the same way other tab errors do;
            // in-pane Failed rendering still shows the detail locally.
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
                    state.post_error(format!("save to {} failed: {error}", path.display()), None);
                }
                _ => {}
            }
            let followup = crate::view::tracer::state::apply_payload(&mut state.tracer, payload);
            state.last_refresh = Instant::now();
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: followup,
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
        },
        AppEvent::Quit => {
            state.should_quit = true;
            UpdateResult {
                redraw: false,
                intent: Some(PendingIntent::Quit),
                tracer_followup: None,
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
    let input_event = state.keymap.translate(key, state.current_tab);

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
            if let Some(entry) = state.history.pop_back(current) {
                state.current_tab = entry.tab;
                restore_anchor(state, &entry);
            }
            return UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
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
            if let Some(entry) = state.history.pop_forward(current) {
                state.current_tab = entry.tab;
                restore_anchor(state, &entry);
            }
            return UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
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
            };
        }
        InputEvent::App(AppAction::Quit) => {
            // Quit always fires, even in modal or text-input mode.
            state.should_quit = true;
            return UpdateResult {
                redraw: false,
                intent: Some(PendingIntent::Quit),
                tracer_followup: None,
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
            };
        }
        InputEvent::App(AppAction::FuzzyFind) => {
            if state.flow_index.is_none() {
                state.post_warning("fuzzy find: flow not indexed yet, open Browser to seed".into());
                return UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
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
                        };
                    }
                    KeyCode::Down => {
                        cs.move_cursor_down();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                        };
                    }
                    KeyCode::Up => {
                        cs.move_cursor_up();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
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
                        };
                    }
                    (KeyCode::Up, _) | (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
                        fs.move_up();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                        };
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
                        fs.move_down();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
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
                            };
                        }
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
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
                        };
                    }
                    (KeyCode::Up, _) => {
                        CursorRef::new(&mut ps.selected, props_len).move_up();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                        };
                    }
                    (KeyCode::Down, _) => {
                        CursorRef::new(&mut ps.selected, props_len).move_down();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                        };
                    }
                    (KeyCode::PageUp, _) => {
                        CursorRef::new(&mut ps.selected, props_len).page_up(10);
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                        };
                    }
                    (KeyCode::PageDown, _) => {
                        CursorRef::new(&mut ps.selected, props_len).page_down(10);
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                        };
                    }
                    (KeyCode::Home, _) => {
                        CursorRef::new(&mut ps.selected, props_len).goto_first();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                        };
                    }
                    (KeyCode::End, _) => {
                        CursorRef::new(&mut ps.selected, props_len).goto_last();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
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
                        };
                    }
                    (KeyCode::Enter, _) => {
                        // Descend: if the selected row's value resolves to an arena
                        // node, close the modal and emit a Goto(OpenInBrowser) intent.
                        // Non-resolvable rows are a no-op.
                        let props: Vec<(String, String)> =
                            match state.browser.details.get(&ps.arena_idx) {
                                Some(NodeDetail::Processor(p)) => p.properties.clone(),
                                Some(NodeDetail::ControllerService(c)) => c.properties.clone(),
                                _ => Vec::new(),
                            };
                        let Some((_, value)) = props.get(ps.selected) else {
                            return UpdateResult::default();
                        };
                        let Some(resolved) = state.browser.resolve_id(value) else {
                            return UpdateResult::default();
                        };
                        let intent = PendingIntent::Goto(CrossLink::OpenInBrowser {
                            component_id: value.trim().to_string(),
                            group_id: resolved.group_id,
                        });
                        state.modal = None;
                        return UpdateResult {
                            redraw: true,
                            intent: Some(intent),
                            tracer_followup: None,
                        };
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
                        };
                    }
                    KeyCode::Backspace => {
                        save.backspace();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                        };
                    }
                    KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        save.push_char(ch);
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
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
                    };
                }
                KeyCode::Up => {
                    jm.move_up();
                    return UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    };
                }
                KeyCode::Down => {
                    jm.move_down();
                    return UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
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
                        };
                    }
                    return UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
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

fn handle_browser_payload(state: &mut AppState, payload: crate::event::BrowserPayload) {
    use crate::event::BrowserPayload;
    match payload {
        BrowserPayload::Detail(detail) => {
            crate::view::browser::state::apply_node_detail(&mut state.browser, *detail);
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
            state.cluster =
                crate::cluster::ClusterStore::new(state.polling.cluster.clone(), ring_cap);

            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }
        Ok(IntentOutcome::ViewRefreshed { .. }) => {
            state.last_refresh = Instant::now();
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }
        Ok(IntentOutcome::Quitting) => {
            state.should_quit = true;
            UpdateResult {
                redraw: false,
                intent: None,
                tracer_followup: None,
            }
        }
        Ok(IntentOutcome::NotImplementedInPhase { intent_name, phase }) => {
            state.post_info(format!("{intent_name}: not yet wired (Phase {phase})"));
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
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
            state.status.banner = None;
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
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
            }
        }
        Ok(IntentOutcome::TracerInputInvalid { raw }) => {
            state.post_warning(format!("invalid UUID: {raw}"));
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
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
mod tests {
    use super::*;
    use crate::config::{
        AuthConfig, Context, PasswordAuthConfig, PasswordCredentials, VersionStrategy,
    };

    pub(super) fn fresh_state() -> AppState {
        let c = tiny_config();
        AppState::new("dev".into(), Version::new(2, 9, 0), &c)
    }

    pub(super) fn tiny_config() -> Config {
        Config {
            current_context: "dev".into(),
            bulletins: Default::default(),
            ui: Default::default(),
            polling: Default::default(),
            tracer: Default::default(),
            contexts: vec![
                Context {
                    name: "dev".into(),
                    url: "https://dev:8443".into(),
                    auth: AuthConfig::Password(PasswordAuthConfig {
                        username: "admin".into(),
                        credentials: PasswordCredentials::Plain {
                            password: "x".into(),
                        },
                    }),
                    version_strategy: VersionStrategy::Strict,
                    insecure_tls: false,
                    ca_cert_path: None,
                    proxied_entities_chain: None,
                    proxy_url: None,
                    http_proxy_url: None,
                    https_proxy_url: None,
                },
                Context {
                    name: "prod".into(),
                    url: "https://prod:8443".into(),
                    auth: AuthConfig::Password(PasswordAuthConfig {
                        username: "admin".into(),
                        credentials: PasswordCredentials::Plain {
                            password: "y".into(),
                        },
                    }),
                    version_strategy: VersionStrategy::Strict,
                    insecure_tls: false,
                    ca_cert_path: None,
                    proxied_entities_chain: None,
                    proxy_url: None,
                    http_proxy_url: None,
                    https_proxy_url: None,
                },
            ],
        }
    }

    pub(super) fn key(code: KeyCode, mods: KeyModifiers) -> AppEvent {
        AppEvent::Input(Event::Key(KeyEvent::new(code, mods)))
    }

    pub(super) fn seeded_browser_state() -> (AppState, Config) {
        use crate::client::{NodeKind, NodeStatusSummary, RawNode, RecursiveSnapshot};
        use std::time::SystemTime;

        let mut s = fresh_state();
        let c = tiny_config();
        let snap = RecursiveSnapshot {
            nodes: vec![
                RawNode {
                    parent_idx: None,
                    kind: NodeKind::ProcessGroup,
                    id: "root".into(),
                    group_id: "root".into(),
                    name: "root".into(),
                    status_summary: NodeStatusSummary::ProcessGroup {
                        running: 0,
                        stopped: 0,
                        invalid: 0,
                        disabled: 0,
                    },
                },
                RawNode {
                    parent_idx: Some(0),
                    kind: NodeKind::Processor,
                    id: "gen".into(),
                    group_id: "root".into(),
                    name: "Gen".into(),
                    status_summary: NodeStatusSummary::Processor {
                        run_status: "Running".into(),
                    },
                },
                RawNode {
                    parent_idx: Some(0),
                    kind: NodeKind::ProcessGroup,
                    id: "ingest".into(),
                    group_id: "root".into(),
                    name: "ingest".into(),
                    status_summary: NodeStatusSummary::ProcessGroup {
                        running: 0,
                        stopped: 0,
                        invalid: 0,
                        disabled: 0,
                    },
                },
            ],
            fetched_at: SystemTime::now(),
        };
        crate::view::browser::state::apply_tree_snapshot(&mut s.browser, snap);
        s.flow_index = Some(crate::view::browser::state::build_flow_index(&s.browser));
        s.current_tab = ViewId::Browser;
        (s, c)
    }

    #[test]
    fn tab_no_longer_cycles_tabs() {
        // Tab is now FocusAction::NextPane (pane cycling within a view),
        // not a tab-switch action. Pressing Tab must not change the active tab.
        let mut s = fresh_state();
        let c = tiny_config();
        update(&mut s, key(KeyCode::Tab, KeyModifiers::NONE), &c);
        assert_eq!(s.current_tab, ViewId::Overview);
    }

    #[test]
    fn back_tab_no_longer_cycles_tabs() {
        // BackTab is now FocusAction::PrevPane. It must not change the active tab.
        let mut s = fresh_state();
        let c = tiny_config();
        update(&mut s, key(KeyCode::BackTab, KeyModifiers::NONE), &c);
        assert_eq!(s.current_tab, ViewId::Overview);
    }

    #[test]
    fn function_keys_goto_tabs() {
        let mut s = fresh_state();
        let c = tiny_config();
        update(&mut s, key(KeyCode::F(3), KeyModifiers::NONE), &c);
        assert_eq!(s.current_tab, ViewId::Browser);
        update(&mut s, key(KeyCode::F(4), KeyModifiers::NONE), &c);
        assert_eq!(s.current_tab, ViewId::Events);
    }

    #[test]
    fn f_keys_leave_tracer_while_in_entry_mode() {
        // Regression: Tracer starts in TracerMode::Entry (UUID input),
        // which routes printable chars to handle_text_input but must NOT
        // suppress global F1-F5 tab-switch shortcuts. Otherwise the user
        // is trapped in the Tracer tab.
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Tracer;
        assert!(
            matches!(
                s.tracer.mode,
                crate::view::tracer::state::TracerMode::Entry(_)
            ),
            "Tracer should start in Entry mode"
        );
        update(&mut s, key(KeyCode::F(1), KeyModifiers::NONE), &c);
        assert_eq!(s.current_tab, ViewId::Overview, "F1 should leave Tracer");
    }

    #[test]
    fn q_requests_quit() {
        let mut s = fresh_state();
        let c = tiny_config();
        let r = update(&mut s, key(KeyCode::Char('q'), KeyModifiers::NONE), &c);
        assert!(s.should_quit);
        assert!(matches!(r.intent, Some(PendingIntent::Quit)));
    }

    #[test]
    fn ctrl_c_requests_quit() {
        let mut s = fresh_state();
        let c = tiny_config();
        update(&mut s, key(KeyCode::Char('c'), KeyModifiers::CONTROL), &c);
        assert!(s.should_quit);
    }

    #[test]
    fn help_modal_toggles() {
        let mut s = fresh_state();
        let c = tiny_config();
        update(&mut s, key(KeyCode::Char('?'), KeyModifiers::NONE), &c);
        assert!(matches!(s.modal, Some(Modal::Help)));
        update(&mut s, key(KeyCode::Char('?'), KeyModifiers::NONE), &c);
        assert!(s.modal.is_none());
    }

    #[test]
    fn esc_closes_modal() {
        let mut s = fresh_state();
        let c = tiny_config();
        update(&mut s, key(KeyCode::Char('?'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
        assert!(s.modal.is_none());
    }

    #[test]
    fn esc_dismisses_the_status_banner_at_top_level() {
        let mut s = fresh_state();
        let c = tiny_config();
        // Seed a banner — any severity works.
        s.status.banner = Some(Banner {
            severity: BannerSeverity::Warning,
            message: "nodewise diagnostics unavailable".into(),
            detail: None,
        });
        // Ensure no modal is open so Esc reaches the global dispatch.
        assert!(s.modal.is_none());
        update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
        assert!(s.status.banner.is_none(), "Esc must clear the banner");
    }

    #[test]
    fn esc_with_no_banner_is_idempotent() {
        let mut s = fresh_state();
        let c = tiny_config();
        assert!(s.status.banner.is_none());
        update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
        assert!(s.status.banner.is_none());
    }

    #[test]
    fn capital_k_opens_context_switcher() {
        let mut s = fresh_state();
        let c = tiny_config();
        update(&mut s, key(KeyCode::Char('K'), KeyModifiers::SHIFT), &c);
        let modal = s.modal.as_ref().unwrap();
        match modal {
            Modal::ContextSwitcher(cs) => {
                assert_eq!(cs.entries.len(), 2);
                assert!(cs.entries[0].is_active);
            }
            _ => panic!("expected ContextSwitcher"),
        }
    }

    #[test]
    fn context_switcher_enter_emits_intent() {
        let mut s = fresh_state();
        let c = tiny_config();
        update(&mut s, key(KeyCode::Char('K'), KeyModifiers::SHIFT), &c);
        update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
        let r = update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        match r.intent {
            Some(PendingIntent::SwitchContext(name)) => assert_eq!(name, "prod"),
            other => panic!("expected SwitchContext, got {other:?}"),
        }
    }

    #[test]
    fn context_switched_outcome_updates_version_and_closes_modal() {
        let mut s = fresh_state();
        let c = tiny_config();
        update(&mut s, key(KeyCode::Char('K'), KeyModifiers::SHIFT), &c);
        let outcome = Ok(IntentOutcome::ContextSwitched {
            new_context_name: "other-ctx".into(),
            new_version: Version::new(2, 7, 2),
        });
        update(&mut s, AppEvent::IntentOutcome(outcome), &c);
        assert_eq!(s.detected_version, Version::new(2, 7, 2));
        assert_eq!(s.context_name, "other-ctx");
        assert!(s.modal.is_none());
        assert!(s.pending_worker_restart, "worker restart must be flagged");
    }

    #[test]
    fn context_switched_clears_cluster_summary_and_history() {
        use crate::app::history::{HistoryEntry, SelectionAnchor};

        let mut s = fresh_state();
        let c = tiny_config();
        s.cluster_summary = ClusterSummary {
            connected_nodes: Some(3),
            total_nodes: Some(3),
        };
        s.history.push(HistoryEntry {
            tab: ViewId::Browser,
            anchor: Some(SelectionAnchor::ComponentId("stale-cid".into())),
        });

        let outcome = Ok(IntentOutcome::ContextSwitched {
            new_context_name: "other-ctx".into(),
            new_version: Version::new(2, 7, 2),
        });
        update(&mut s, AppEvent::IntentOutcome(outcome), &c);

        assert_eq!(s.cluster_summary.connected_nodes, None);
        assert_eq!(s.cluster_summary.total_nodes, None);
        assert!(!s.history.can_go_back(), "history must be wiped");
        assert!(!s.history.can_go_forward());
    }

    #[test]
    fn context_switched_clears_events_results() {
        use crate::client::ProvenanceEventSummary;
        use crate::view::events::state::EventsQueryStatus;
        use std::time::SystemTime;

        let mut s = fresh_state();
        let c = tiny_config();
        s.events.events.push(ProvenanceEventSummary {
            event_id: 1,
            event_time_iso: "2026-04-17T00:00:00Z".into(),
            event_type: "DROP".into(),
            component_id: "cid".into(),
            component_name: "CName".into(),
            component_type: "PROCESSOR".into(),
            group_id: "gid".into(),
            flow_file_uuid: "ff-1".into(),
            relationship: None,
            details: None,
        });
        s.events.selected_row = Some(0);
        s.events.status = EventsQueryStatus::Done {
            fetched_at: SystemTime::now(),
            truncated: false,
            took_ms: 42,
        };

        let outcome = Ok(IntentOutcome::ContextSwitched {
            new_context_name: "other-ctx".into(),
            new_version: Version::new(2, 7, 2),
        });
        update(&mut s, AppEvent::IntentOutcome(outcome), &c);

        assert!(
            s.events.events.is_empty(),
            "stale provenance results must be cleared"
        );
        assert!(s.events.selected_row.is_none());
        assert!(matches!(s.events.status, EventsQueryStatus::Idle));
    }

    #[test]
    fn intent_error_sets_banner() {
        let mut s = fresh_state();
        let c = tiny_config();
        let err = NifiLensError::WriteIntentRefused {
            intent_name: "StartProcessor",
        };
        update(&mut s, AppEvent::IntentOutcome(Err(err)), &c);
        assert!(s.status.banner.is_some());
        assert_eq!(
            s.status.banner.as_ref().unwrap().severity,
            BannerSeverity::Error
        );
    }

    #[test]
    fn cross_link_open_in_browser_pushes_history() {
        let (mut s, c) = seeded_browser_state();
        s.current_tab = ViewId::Bulletins;
        let outcome = Ok(IntentOutcome::OpenInBrowserTarget {
            component_id: "gen".into(),
            group_id: "root".into(),
        });
        update(&mut s, AppEvent::IntentOutcome(outcome), &c);
        assert!(s.history.can_go_back(), "back stack should have an entry");
        assert_eq!(s.current_tab, ViewId::Browser);
    }

    #[test]
    fn cross_link_tracer_landing_pushes_history() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Browser;
        let outcome = Ok(IntentOutcome::TracerLandingOn {
            component_id: "some-comp".into(),
        });
        update(&mut s, AppEvent::IntentOutcome(outcome), &c);
        assert!(s.history.can_go_back(), "back stack should have an entry");
        assert_eq!(s.current_tab, ViewId::Tracer);
    }

    #[test]
    fn shift_left_navigates_history_back_replaces_bracket() {
        // `[` is unmapped; history back is now Shift+Left via the central
        // InputEvent::History(Back) dispatch.
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        s.history.push(crate::app::history::HistoryEntry {
            tab: ViewId::Bulletins,
            anchor: None,
        });
        s.current_tab = ViewId::Browser;

        update(&mut s, key(KeyCode::Left, KeyModifiers::SHIFT), &c);
        assert_eq!(s.current_tab, ViewId::Bulletins);
    }

    #[test]
    fn shift_right_navigates_forward() {
        // `]` is unmapped; history forward is now Shift+Right via the central
        // InputEvent::History(Forward) dispatch.
        let mut s = fresh_state();
        let c = tiny_config();
        // Simulate: was on Bulletins, pushed history, moved to Browser,
        // then popped back. Forward stack should have Browser.
        s.history.push(crate::app::history::HistoryEntry {
            tab: ViewId::Bulletins,
            anchor: None,
        });
        s.current_tab = ViewId::Browser;
        // Pop back to Bulletins (populates forward with Browser).
        let current = crate::app::history::HistoryEntry {
            tab: ViewId::Browser,
            anchor: None,
        };
        let entry = s.history.pop_back(current);
        assert!(entry.is_some());
        s.current_tab = ViewId::Bulletins;
        assert!(s.history.can_go_forward());

        update(&mut s, key(KeyCode::Right, KeyModifiers::SHIFT), &c);
        assert_eq!(s.current_tab, ViewId::Browser);
    }

    #[test]
    fn left_bracket_noop_when_history_empty() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Browser;
        update(&mut s, key(KeyCode::Char('['), KeyModifiers::NONE), &c);
        // Tab unchanged — no history.
        assert_eq!(s.current_tab, ViewId::Browser);
    }

    #[test]
    fn new_state_has_empty_cluster_summary() {
        let state = fresh_state();
        assert_eq!(state.cluster_summary.connected_nodes, None);
        assert_eq!(state.cluster_summary.total_nodes, None);
    }

    #[test]
    fn fuzzy_find_modal_f_key_is_captured_as_query_character() {
        // Regression: the FuzzyFind close arm used to include Char('f') which
        // ate every search starting with `f`. Only Esc closes the modal now.
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Browser;
        // Seed the flow index so the fuzzy find modal can actually open.
        s.flow_index = Some(crate::view::browser::state::FlowIndex { entries: vec![] });
        // Open the modal via Shift+F.
        update(&mut s, key(KeyCode::Char('F'), KeyModifiers::SHIFT), &c);
        assert!(
            matches!(s.modal, Some(Modal::FuzzyFind(_))),
            "Shift+F should open the FuzzyFind modal"
        );
        // Type 'f' again — this should append to the query, NOT close the modal.
        update(&mut s, key(KeyCode::Char('f'), KeyModifiers::NONE), &c);
        assert!(
            matches!(s.modal, Some(Modal::FuzzyFind(_))),
            "second f should be captured as query char, not close the modal"
        );
        if let Some(Modal::FuzzyFind(ref fs)) = s.modal {
            assert_eq!(fs.query, "f", "query buffer should contain 'f'");
        }
        // Esc closes it.
        update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
        assert!(s.modal.is_none(), "Esc should close the modal");
    }

    #[test]
    fn collect_hints_advertises_new_bracket_chords_not_alt_arrows() {
        let mut s = fresh_state();
        // Put something in history so the back/fwd hints are emitted.
        s.history.push(crate::app::history::HistoryEntry {
            tab: ViewId::Bulletins,
            anchor: None,
        });
        s.current_tab = ViewId::Browser;
        let hints = collect_hints(&s);
        let hint_text: String = hints
            .iter()
            .map(|h| format!("{} {}", h.key, h.action))
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(
            !hint_text.contains("Alt+"),
            "hint bar must not advertise old Alt+ chords: {hint_text}"
        );
    }

    #[test]
    fn severity_filter_hints_are_hidden_from_status_bar() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Bulletins;
        let hints = collect_hints(&s);
        // The three 1/2/3 hints must not appear in the status-bar strip —
        // they're surfaced by the [E n] [W n] [I n] chips one row above.
        assert!(
            !hints.iter().any(|h| h.key == "1"),
            "key '1' must not be in status bar; got {:?}",
            hints.iter().map(|h| h.key.as_ref()).collect::<Vec<_>>(),
        );
        assert!(
            !hints.iter().any(|h| h.key == "2"),
            "key '2' must not be in status bar"
        );
        assert!(
            !hints.iter().any(|h| h.key == "3"),
            "key '3' must not be in status bar"
        );
        // Sanity: other Bulletins hints are still present.
        assert!(hints.iter().any(|h| h.key == "/"), "other hints unaffected");
    }

    #[test]
    fn capital_k_with_shift_opens_context_switcher() {
        // ContextSwitcher is bound to Shift+K via the central
        // InputEvent::App(ContextSwitcher) dispatch. The legacy loose match
        // for K-without-SHIFT is no longer supported.
        let mut s = fresh_state();
        let c = tiny_config();
        update(&mut s, key(KeyCode::Char('K'), KeyModifiers::SHIFT), &c);
        assert!(
            matches!(s.modal, Some(Modal::ContextSwitcher(_))),
            "Shift+K should open the context switcher"
        );
    }

    fn build_test_sysdiag_with_two_nodes() -> crate::client::health::SystemDiagSnapshot {
        use crate::client::health::{
            GcSnapshot, NodeDiagnostics, RepoUsage, SystemDiagAggregate, SystemDiagSnapshot,
        };
        use std::time::Instant;

        let node = |address: &str| NodeDiagnostics {
            address: address.into(),
            heap_used_bytes: 512 * 1024 * 1024,
            heap_max_bytes: 1024 * 1024 * 1024,
            gc: vec![GcSnapshot {
                name: "G1 Young".into(),
                collection_count: 10,
                collection_millis: 50,
            }],
            load_average: Some(1.5),
            available_processors: Some(4),
            total_threads: 50,
            uptime: "1h".into(),
            content_repos: vec![RepoUsage {
                identifier: "content".into(),
                used_bytes: 60,
                total_bytes: 100,
                free_bytes: 40,
                utilization_percent: 60,
            }],
            flowfile_repo: Some(RepoUsage {
                identifier: "flowfile".into(),
                used_bytes: 30,
                total_bytes: 100,
                free_bytes: 70,
                utilization_percent: 30,
            }),
            provenance_repos: vec![RepoUsage {
                identifier: "provenance".into(),
                used_bytes: 20,
                total_bytes: 100,
                free_bytes: 80,
                utilization_percent: 20,
            }],
        };

        SystemDiagSnapshot {
            aggregate: SystemDiagAggregate {
                content_repos: vec![RepoUsage {
                    identifier: "content".into(),
                    used_bytes: 60,
                    total_bytes: 100,
                    free_bytes: 40,
                    utilization_percent: 60,
                }],
                flowfile_repo: Some(RepoUsage {
                    identifier: "flowfile".into(),
                    used_bytes: 30,
                    total_bytes: 100,
                    free_bytes: 70,
                    utilization_percent: 30,
                }),
                provenance_repos: vec![RepoUsage {
                    identifier: "provenance".into(),
                    used_bytes: 20,
                    total_bytes: 100,
                    free_bytes: 80,
                    utilization_percent: 20,
                }],
            },
            nodes: vec![node("node1:8080"), node("node2:8080")],
            fetched_at: Instant::now(),
        }
    }

    #[test]
    fn sysdiag_redraw_populates_cluster_summary() {
        use crate::cluster::snapshot::{EndpointState, FetchMeta};
        use std::time::{Duration, Instant};

        let mut s = fresh_state();

        // Pre-condition: cluster_summary is empty placeholder.
        assert_eq!(s.cluster_summary.connected_nodes, None);
        assert_eq!(s.cluster_summary.total_nodes, None);

        // Seed the cluster snapshot and invoke `redraw_sysdiag`
        // directly. The main-loop `ClusterChanged` arm
        // (`src/app/mod.rs`) routes to this reducer; the reducer test
        // is the canonical coverage for the projection logic.
        s.cluster.snapshot.system_diagnostics = EndpointState::Ready {
            data: build_test_sysdiag_with_two_nodes(),
            meta: FetchMeta {
                fetched_at: Instant::now(),
                fetch_duration: Duration::from_millis(5),
                next_interval: Duration::from_secs(30),
            },
        };

        crate::view::overview::state::redraw_sysdiag(&mut s);

        assert_eq!(s.cluster_summary.total_nodes, Some(2));
        // NodeDiagnostics has no status field, so connected_nodes equals total.
        assert_eq!(s.cluster_summary.connected_nodes, Some(2));
    }

    #[test]
    fn context_switcher_row_nav_uses_arrows_only_no_jk() {
        // Open the context switcher (2 entries via tiny_config).
        let mut s = fresh_state();
        let c = tiny_config();
        update(&mut s, key(KeyCode::Char('K'), KeyModifiers::SHIFT), &c);
        let before = match s.modal.as_ref().unwrap() {
            Modal::ContextSwitcher(cs) => cs.cursor,
            _ => panic!("expected ContextSwitcher"),
        };

        // j is a no-op inside the modal.
        update(&mut s, key(KeyCode::Char('j'), KeyModifiers::NONE), &c);
        let after_j = match s.modal.as_ref().unwrap() {
            Modal::ContextSwitcher(cs) => cs.cursor,
            _ => panic!("expected ContextSwitcher"),
        };
        assert_eq!(after_j, before, "j dropped");

        // Down still moves the cursor.
        update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
        let after_down = match s.modal.as_ref().unwrap() {
            Modal::ContextSwitcher(cs) => cs.cursor,
            _ => panic!("expected ContextSwitcher"),
        };
        assert!(after_down > before, "Down still works");

        let before = after_down;
        // k is a no-op.
        update(&mut s, key(KeyCode::Char('k'), KeyModifiers::NONE), &c);
        let after_k = match s.modal.as_ref().unwrap() {
            Modal::ContextSwitcher(cs) => cs.cursor,
            _ => panic!("expected ContextSwitcher"),
        };
        assert_eq!(after_k, before, "k dropped");

        // Up still moves the cursor back.
        update(&mut s, key(KeyCode::Up, KeyModifiers::NONE), &c);
        let after_up = match s.modal.as_ref().unwrap() {
            Modal::ContextSwitcher(cs) => cs.cursor,
            _ => panic!("expected ContextSwitcher"),
        };
        assert!(after_up < before, "Up still works");
    }

    #[test]
    fn events_landing_on_seeds_filters_and_switches_tab() {
        let mut s = fresh_state();
        let c = tiny_config();
        let outcome = crate::event::IntentOutcome::EventsLandingOn {
            component_id: "proc-42".into(),
        };
        let r = update(&mut s, AppEvent::IntentOutcome(Ok(outcome)), &c);
        assert_eq!(s.current_tab, ViewId::Events);
        assert_eq!(s.events.filters.source, "proc-42");
        assert_eq!(s.events.filters.time, "last 15m");
        assert!(matches!(
            s.events.status,
            crate::view::events::state::EventsQueryStatus::Running { .. }
        ));
        assert!(matches!(
            r.intent,
            Some(PendingIntent::RunProvenanceQuery { .. })
        ));
    }

    #[test]
    fn tab_switch_away_from_events_clears_failed_status() {
        use crate::event::{EventsPayload, ViewPayload};
        use crate::view::events::state::EventsQueryStatus;

        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Events;

        // Drive the events state into Running so QueryFailed applies.
        update(
            &mut s,
            AppEvent::Data(ViewPayload::Events(EventsPayload::QueryStarted {
                query_id: "q-1".into(),
            })),
            &c,
        );
        update(
            &mut s,
            AppEvent::Data(ViewPayload::Events(EventsPayload::QueryFailed {
                query_id: Some("q-1".into()),
                error: "boom".into(),
            })),
            &c,
        );
        assert!(matches!(s.events.status, EventsQueryStatus::Failed { .. }));

        // Press F1 to switch to Overview.
        update(&mut s, key(KeyCode::F(1), KeyModifiers::NONE), &c);
        assert_eq!(s.current_tab, ViewId::Overview);
        assert!(
            matches!(s.events.status, EventsQueryStatus::Idle),
            "leaving Events must reset Failed to Idle"
        );
    }

    fn seed_one_bulletin(state: &mut AppState) {
        use crate::client::BulletinSnapshot;
        state.bulletins.ring.push_back(BulletinSnapshot {
            id: 1,
            level: "ERROR".into(),
            message: "test-msg".into(),
            source_id: "src-42".into(),
            source_name: "Proc-42".into(),
            source_type: "PROCESSOR".into(),
            group_id: "root".into(),
            timestamp_iso: "2026-04-14T00:00:00Z".into(),
            timestamp_human: String::new(),
        });
    }

    #[test]
    fn shift_left_navigates_history_back() {
        use crossterm::event::{KeyEvent, KeyModifiers};

        let mut s = fresh_state();
        let c = tiny_config();

        // Build a history: start on Overview, move to Bulletins, then
        // history back should return to Overview.
        s.history.push(crate::app::history::HistoryEntry {
            tab: ViewId::Overview,
            anchor: None,
        });
        s.current_tab = ViewId::Bulletins;

        let r = update(
            &mut s,
            AppEvent::Input(Event::Key(KeyEvent::new(
                KeyCode::Left,
                KeyModifiers::SHIFT,
            ))),
            &c,
        );
        assert!(r.redraw);
        assert_eq!(s.current_tab, ViewId::Overview);
    }

    #[test]
    fn g_from_bulletins_opens_goto_menu_then_enter_gotos_to_browser() {
        use crossterm::event::{KeyEvent, KeyModifiers};

        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        seed_one_bulletin(&mut s);

        // Press `g` — maps to AppAction::Goto; with Browser + Events cross-links
        // available, a GotoMenu modal opens (no intent emitted yet).
        let r1 = update(
            &mut s,
            AppEvent::Input(Event::Key(KeyEvent::new(
                KeyCode::Char('g'),
                KeyModifiers::NONE,
            ))),
            &c,
        );
        assert!(r1.intent.is_none());
        assert!(matches!(s.modal, Some(Modal::GotoMenu(_))));

        // Press Enter — selects index 0 = Browser (the first cross-link).
        let r2 = update(
            &mut s,
            AppEvent::Input(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            ))),
            &c,
        );
        assert!(matches!(
            r2.intent,
            Some(PendingIntent::Goto(CrossLink::OpenInBrowser { .. }))
        ));
    }

    #[test]
    fn handle_verb_toggles_error_filter_after_port() {
        // After the Bulletins port (Phase 3 Task 12), handle_verb dispatches
        // directly — ToggleSeverity(Error) flips show_error immediately.
        use crate::input::{Severity, ViewVerb};

        let mut s = fresh_state();
        s.current_tab = ViewId::Bulletins;
        let before = s.bulletins.filters.show_error;
        let _ = bulletins::BulletinsHandler::handle_verb(
            &mut s,
            ViewVerb::Bulletins(crate::input::BulletinsVerb::ToggleSeverity(Severity::Error)),
        );
        assert_ne!(
            s.bulletins.filters.show_error, before,
            "handle_verb must toggle show_error after Bulletins port"
        );
    }

    #[test]
    fn bare_e_does_not_open_error_detail() {
        use crossterm::event::{KeyEvent, KeyModifiers};
        let mut s = fresh_state();
        let c = tiny_config();
        s.status.banner = Some(Banner {
            severity: BannerSeverity::Error,
            message: "test".into(),
            detail: Some("detail".into()),
        });
        update(
            &mut s,
            AppEvent::Input(Event::Key(KeyEvent::new(
                KeyCode::Char('e'),
                KeyModifiers::NONE,
            ))),
            &c,
        );
        assert!(
            !matches!(s.modal, Some(Modal::ErrorDetail)),
            "bare 'e' must not open error detail"
        );
    }

    #[test]
    fn enter_on_error_banner_opens_detail() {
        use crossterm::event::{KeyEvent, KeyModifiers};
        let mut s = fresh_state();
        let c = tiny_config();
        s.status.banner = Some(Banner {
            severity: BannerSeverity::Error,
            message: "test".into(),
            detail: Some("detail".into()),
        });
        update(
            &mut s,
            AppEvent::Input(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            ))),
            &c,
        );
        assert!(matches!(s.modal, Some(Modal::ErrorDetail)));
    }

    #[test]
    fn esc_dismisses_error_banner_via_ascend() {
        use crossterm::event::{KeyEvent, KeyModifiers};
        let mut s = fresh_state();
        let c = tiny_config();
        s.status.banner = Some(Banner {
            severity: BannerSeverity::Error,
            message: "test".into(),
            detail: None,
        });
        update(
            &mut s,
            AppEvent::Input(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))),
            &c,
        );
        assert!(s.status.banner.is_none(), "Esc must dismiss the banner");
    }

    #[test]
    fn overview_noisy_g_b_builds_open_in_browser() {
        use crate::view::overview::state::{NoisyComponent, OverviewFocus, Severity as OvSev};
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        s.overview.focus = OverviewFocus::Noisy;
        s.overview.noisy = vec![NoisyComponent {
            source_id: "proc-1".into(),
            group_id: "grp-1".into(),
            source_name: "MyProc".into(),
            count: 3,
            max_severity: OvSev::Error,
        }];
        s.overview.noisy_selected = 0;
        let link = build_go_crosslink(&s, crate::input::GoTarget::Browser);
        assert!(
            matches!(&link, Some(CrossLink::OpenInBrowser { component_id, group_id })
                if component_id == "proc-1" && group_id == "grp-1"),
            "got {link:?}"
        );
    }

    #[test]
    fn overview_noisy_g_e_builds_goto_events() {
        use crate::view::overview::state::{NoisyComponent, OverviewFocus, Severity as OvSev};
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        s.overview.focus = OverviewFocus::Noisy;
        s.overview.noisy = vec![NoisyComponent {
            source_id: "proc-2".into(),
            group_id: "grp-2".into(),
            source_name: "OtherProc".into(),
            count: 1,
            max_severity: OvSev::Warning,
        }];
        s.overview.noisy_selected = 0;
        let link = build_go_crosslink(&s, crate::input::GoTarget::Events);
        assert!(
            matches!(&link, Some(CrossLink::GotoEvents { component_id })
                if component_id == "proc-2"),
            "got {link:?}"
        );
    }

    #[test]
    fn overview_queues_g_b_builds_open_in_browser() {
        use crate::view::overview::state::{OverviewFocus, UnhealthyQueue};
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        s.overview.focus = OverviewFocus::Queues;
        s.overview.unhealthy = vec![UnhealthyQueue {
            id: "conn-1".into(),
            group_id: "grp-3".into(),
            name: "q1".into(),
            source_name: "A".into(),
            destination_name: "B".into(),
            fill_percent: 80,
            flow_files_queued: 800,
            bytes_queued: 0,
            queued_display: "800".into(),
        }];
        s.overview.queues_selected = 0;
        let link = build_go_crosslink(&s, crate::input::GoTarget::Browser);
        assert!(
            matches!(&link, Some(CrossLink::OpenInBrowser { component_id, group_id })
                if component_id == "grp-3" && group_id == "grp-3"),
            "got {link:?}"
        );
    }

    #[test]
    fn overview_no_focus_g_b_returns_none() {
        use crate::view::overview::state::OverviewFocus;
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        s.overview.focus = OverviewFocus::None;
        assert!(build_go_crosslink(&s, crate::input::GoTarget::Browser).is_none());
    }

    #[test]
    fn selection_cross_links_empty_on_folder_row() {
        use crate::client::{FolderKind, NodeKind, NodeStatusSummary, RawNode, RecursiveSnapshot};
        let mut s = fresh_state();
        s.current_tab = ViewId::Browser;
        crate::view::browser::state::apply_tree_snapshot(
            &mut s.browser,
            RecursiveSnapshot {
                nodes: vec![
                    RawNode {
                        parent_idx: None,
                        kind: NodeKind::ProcessGroup,
                        id: "root".into(),
                        group_id: "root".into(),
                        name: "root".into(),
                        status_summary: NodeStatusSummary::ProcessGroup {
                            running: 0,
                            stopped: 0,
                            invalid: 0,
                            disabled: 0,
                        },
                    },
                    RawNode {
                        parent_idx: Some(0),
                        kind: NodeKind::ControllerService,
                        id: "cs".into(),
                        group_id: "root".into(),
                        name: "pool".into(),
                        status_summary: NodeStatusSummary::ControllerService {
                            state: "ENABLED".into(),
                        },
                    },
                ],
                fetched_at: std::time::SystemTime::now(),
            },
        );
        let folder_arena = s
            .browser
            .nodes
            .iter()
            .position(|n| matches!(n.kind, NodeKind::Folder(FolderKind::ControllerServices)))
            .unwrap();
        s.browser.selected = s
            .browser
            .visible
            .iter()
            .position(|&i| i == folder_arena)
            .unwrap();
        assert!(
            s.selection_cross_links().is_empty(),
            "folder row must not produce any cross-link targets"
        );
    }

    #[test]
    fn g_from_bulletins_no_selection_is_noop() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        // With no bulletin selected, cross-links are empty → no-op.
        let r = update(&mut s, key(KeyCode::Char('g'), KeyModifiers::NONE), &c);
        assert!(r.intent.is_none());
        assert!(s.modal.is_none(), "no modal when no cross-links");
    }

    #[test]
    fn g_auto_gotos_directly_when_single_cross_link() {
        // Overview + Queues focus: only GoTarget::Browser is available (Events
        // returns None for Queues focus), so selection_cross_links() → [Browser].
        // The single-target arm must fire a JumpTo intent without opening a modal.
        use crate::view::overview::state::{OverviewFocus, UnhealthyQueue};
        use crossterm::event::{KeyEvent, KeyModifiers};

        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Overview;
        s.overview.focus = OverviewFocus::Queues;
        s.overview.unhealthy = vec![UnhealthyQueue {
            id: "conn-1".into(),
            group_id: "grp-3".into(),
            name: "q1".into(),
            source_name: "A".into(),
            destination_name: "B".into(),
            fill_percent: 80,
            flow_files_queued: 800,
            bytes_queued: 0,
            queued_display: "800".into(),
        }];
        s.overview.queues_selected = 0;

        // Verify precondition: exactly one cross-link available.
        assert_eq!(
            s.selection_cross_links(),
            vec![crate::input::GoTarget::Browser],
            "expected exactly [Browser] for Overview+Queues"
        );

        let r = update(
            &mut s,
            AppEvent::Input(Event::Key(KeyEvent::new(
                KeyCode::Char('g'),
                KeyModifiers::NONE,
            ))),
            &c,
        );

        assert!(
            matches!(
                r.intent,
                Some(PendingIntent::Goto(CrossLink::OpenInBrowser { .. }))
            ),
            "single cross-link must auto-goto without a modal; got intent={:?}",
            r.intent
        );
        assert!(
            s.modal.is_none(),
            "no modal should open for single-target auto-goto"
        );
    }

    // ── banner + modal helper methods ────────────────────────────────

    #[test]
    fn post_error_sets_error_severity_and_detail() {
        let mut s = fresh_state();
        s.post_error("boom".to_string(), Some("stack".to_string()));
        let b = s.status.banner.as_ref().expect("banner was set");
        assert_eq!(b.severity, BannerSeverity::Error);
        assert_eq!(b.message, "boom");
        assert_eq!(b.detail.as_deref(), Some("stack"));
    }

    #[test]
    fn post_info_replaces_prior_banner() {
        let mut s = fresh_state();
        s.post_error("err".to_string(), Some("d".to_string()));
        s.post_info("copied: foo".to_string());
        let b = s.status.banner.as_ref().expect("banner was set");
        assert_eq!(b.severity, BannerSeverity::Info);
        assert_eq!(b.message, "copied: foo");
        assert!(
            b.detail.is_none(),
            "post_info must not carry over prior detail"
        );
    }

    #[test]
    fn post_warning_sets_warning_severity() {
        let mut s = fresh_state();
        s.post_warning("clipboard: no display".to_string());
        let b = s.status.banner.as_ref().expect("banner was set");
        assert_eq!(b.severity, BannerSeverity::Warning);
        assert_eq!(b.message, "clipboard: no display");
        assert!(b.detail.is_none());
    }

    #[test]
    fn open_banner_detail_is_noop_without_banner() {
        let mut s = fresh_state();
        assert!(!s.open_banner_detail());
        assert!(s.modal.is_none());
        assert!(s.error_detail.is_none());
    }

    #[test]
    fn open_banner_detail_is_noop_when_banner_has_no_detail() {
        let mut s = fresh_state();
        s.post_info("copied".to_string());
        assert!(!s.open_banner_detail());
        assert!(s.modal.is_none());
        assert!(s.error_detail.is_none());
    }

    #[test]
    fn open_banner_detail_copies_detail_and_sets_modal() {
        let mut s = fresh_state();
        s.post_error("boom".to_string(), Some("full chain".to_string()));
        assert!(s.open_banner_detail());
        assert!(matches!(s.modal, Some(Modal::ErrorDetail)));
        assert_eq!(s.error_detail.as_deref(), Some("full chain"));
    }

    #[test]
    fn close_modal_clears_both_modal_and_error_detail() {
        let mut s = fresh_state();
        s.post_error("boom".to_string(), Some("d".to_string()));
        s.open_banner_detail();
        assert!(s.modal.is_some());
        assert!(s.error_detail.is_some());
        s.close_modal();
        assert!(s.modal.is_none());
        assert!(s.error_detail.is_none());
    }
}

#[cfg(test)]
mod goto_subject_tests {
    use super::*;
    use crate::client::BulletinSnapshot;
    use crate::input::GoTarget;
    use crate::test_support::fresh_state;
    use crate::widget::goto_menu::GotoSubject;

    fn stock_bulletin() -> BulletinSnapshot {
        BulletinSnapshot {
            id: 1,
            level: "WARNING".into(),
            message: "boom".into(),
            source_id: "src-a".into(),
            source_name: "ProcA".into(),
            source_type: "PROCESSOR".into(),
            group_id: "grp-a".into(),
            timestamp_iso: "2026-04-17T00:00:00Z".into(),
            timestamp_human: String::new(),
        }
    }

    fn stock_event() -> crate::client::ProvenanceEventSummary {
        crate::client::ProvenanceEventSummary {
            event_id: 7,
            event_time_iso: "2026-04-17T00:00:00Z".into(),
            event_type: "DROP".into(),
            component_id: "cid".into(),
            component_name: "CName".into(),
            component_type: "PROCESSOR".into(),
            group_id: "gid".into(),
            flow_file_uuid: "ff-42".into(),
            relationship: None,
            details: None,
        }
    }

    #[test]
    fn bulletins_browser_subject_is_component_with_name_and_id() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Bulletins;
        s.bulletins.ring.push_front(stock_bulletin());
        s.bulletins.selected = 0;
        let subject = build_goto_subject(&s, GoTarget::Browser).expect("subject");
        assert_eq!(
            subject,
            GotoSubject::Component {
                name: "ProcA".into(),
                id: "src-a".into(),
            }
        );
    }

    #[test]
    fn events_tracer_subject_is_flowfile_uuid() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Events;
        s.events.events.push(stock_event());
        s.events.selected_row = Some(0);

        let subject = build_goto_subject(&s, GoTarget::Tracer).expect("subject");
        assert_eq!(
            subject,
            GotoSubject::Flowfile {
                uuid: "ff-42".into()
            }
        );
    }

    #[test]
    fn events_browser_subject_is_component_name_and_id() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Events;
        s.events.events.push(stock_event());
        s.events.selected_row = Some(0);

        let subject = build_goto_subject(&s, GoTarget::Browser).expect("subject");
        assert_eq!(
            subject,
            GotoSubject::Component {
                name: "CName".into(),
                id: "cid".into(),
            }
        );
    }

    #[test]
    fn no_selection_returns_none() {
        let s = fresh_state();
        // Default tab = Overview with no noisy/queue selection populated.
        assert!(build_goto_subject(&s, GoTarget::Browser).is_none());
    }
}
