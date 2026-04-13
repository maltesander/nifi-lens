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
use crate::view::browser::state::{
    BrowserState, FlowIndex, apply_tree_snapshot, build_flow_index, rebuild_visible,
};
use crate::view::bulletins::state::BulletinsState;
use crate::view::events::state::EventsState;
use crate::view::overview::{OverviewState, apply_payload as apply_overview_payload};
use crate::view::tracer::state::TracerState;

// ---------------------------------------------------------------------------
// ViewKeyHandler trait
// ---------------------------------------------------------------------------

/// Per-view key-handling trait. Uses static methods (not `&mut self`)
/// because handlers need `&mut AppState`.
pub(crate) trait ViewKeyHandler {
    /// Handle a key event for this view. Returns `Some(UpdateResult)` if
    /// the key was consumed, `None` to let it fall through to global
    /// handlers.
    fn handle_key(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult>;

    /// Return context-sensitive hint spans for this view's current state.
    fn hints(state: &AppState) -> Vec<crate::widget::hint_bar::HintSpan>;
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
    pub error_detail: Option<String>,
    pub should_quit: bool,
    /// Set by the context-switch handler so the app loop can force-restart
    /// the current view worker (the registry no-ops when the view matches).
    pub pending_worker_restart: bool,
    /// Cross-link back/forward history.
    pub history: crate::app::history::TabHistory,
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
            error_detail: None,
            should_quit: false,
            pending_worker_restart: false,
            history: crate::app::history::TabHistory::default(),
            clipboard: None,
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
}

#[derive(Debug)]
pub enum Modal {
    Help,
    ContextSwitcher(ContextSwitcherState),
    ErrorDetail,
    FuzzyFind(crate::widget::fuzzy_find::FuzzyFindState),
    Properties(crate::view::browser::state::PropertiesModalState),
    SaveEventContent(crate::widget::save_modal::SaveEventContentState),
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
    JumpTo(CrossLink),
    Dispatch(crate::intent::Intent),
    SaveEventContent(PendingSave),
    RunProvenanceQuery {
        query: crate::client::ProvenanceQuery,
    },
    Quit,
}

/// Data needed to write raw content bytes to a file outside the reducer.
#[derive(Debug)]
pub struct PendingSave {
    pub path: std::path::PathBuf,
    pub raw: std::sync::Arc<[u8]>,
}

// ---------------------------------------------------------------------------
// Hint collection
// ---------------------------------------------------------------------------

/// Collect the hint spans for the current state, respecting modal priority.
pub fn collect_hints(state: &AppState) -> Vec<crate::widget::hint_bar::HintSpan> {
    use crate::widget::hint_bar::HintSpan;

    // Modal hints take full priority — no global hints appended.
    if let Some(ref modal) = state.modal {
        return match modal {
            Modal::Help => vec![HintSpan {
                key: "Esc",
                action: "close",
            }],
            Modal::ContextSwitcher(_) => vec![
                HintSpan {
                    key: "↑/↓",
                    action: "nav",
                },
                HintSpan {
                    key: "Enter",
                    action: "switch",
                },
                HintSpan {
                    key: "Esc",
                    action: "cancel",
                },
            ],
            Modal::FuzzyFind(_) => vec![
                HintSpan {
                    key: "type",
                    action: "filter",
                },
                HintSpan {
                    key: "Enter",
                    action: "select",
                },
                HintSpan {
                    key: "Esc",
                    action: "cancel",
                },
            ],
            Modal::Properties(_) => vec![
                HintSpan {
                    key: "↑/↓",
                    action: "scroll",
                },
                HintSpan {
                    key: "Esc",
                    action: "close",
                },
            ],
            Modal::ErrorDetail => vec![HintSpan {
                key: "Esc",
                action: "close",
            }],
            Modal::SaveEventContent(_) => vec![
                HintSpan {
                    key: "Enter",
                    action: "confirm",
                },
                HintSpan {
                    key: "Esc",
                    action: "cancel",
                },
            ],
        };
    }

    // Per-view hints
    let mut hints = match state.current_tab {
        ViewId::Overview => overview::OverviewHandler::hints(state),
        ViewId::Bulletins => bulletins::BulletinsHandler::hints(state),
        ViewId::Browser => browser::BrowserHandler::hints(state),
        ViewId::Events => events::EventsHandler::hints(state),
        ViewId::Tracer => tracer::TracerHandler::hints(state),
    };

    // Global hints appended
    if state.history.can_go_back() {
        hints.push(HintSpan {
            key: "[",
            action: "back",
        });
    }
    if state.history.can_go_forward() {
        hints.push(HintSpan {
            key: "]",
            action: "fwd",
        });
    }
    hints.push(HintSpan {
        key: "?",
        action: "help",
    });

    hints
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
        AppEvent::Data(ViewPayload::Overview(payload)) => {
            // Side-effects on AppState that need fields outside OverviewState
            // happen here, before delegating to the per-view reducer.
            //
            // Populate the cluster_summary placeholder added in Phase 1.
            // This drives the top-bar identity strip's `nodes N/M`.
            //
            // `NodeDiagnostics` has no `status` field that distinguishes
            // connected from disconnected nodes, so both totals are set to
            // `diag.nodes.len()` until upstream adds that field.
            match &payload {
                crate::event::OverviewPayload::SystemDiag(diag)
                | crate::event::OverviewPayload::SystemDiagFallback { diag, .. } => {
                    state.cluster_summary.total_nodes = Some(diag.nodes.len());
                    state.cluster_summary.connected_nodes = Some(diag.nodes.len());
                }
                crate::event::OverviewPayload::PgStatus(_) => {}
            }

            if let crate::event::OverviewPayload::SystemDiagFallback { warning, .. } = &payload {
                state.status.banner = Some(crate::app::state::Banner {
                    severity: crate::app::state::BannerSeverity::Warning,
                    message: warning.clone(),
                    detail: None,
                });
            }

            apply_overview_payload(&mut state.overview, payload);
            state.last_refresh = Instant::now();
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }
        AppEvent::Data(ViewPayload::Bulletins(payload)) => {
            crate::view::bulletins::state::apply_payload(&mut state.bulletins, payload);
            state.last_refresh = Instant::now();
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }
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
                state.status.banner = Some(Banner {
                    severity: BannerSeverity::Error,
                    message: format!("Events query failed: {error}"),
                    detail: None,
                });
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
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('e')) {
                    state.modal = None;
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
                                intent: Some(PendingIntent::JumpTo(link)),
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
                match key.code {
                    KeyCode::Esc | KeyCode::Char('e') => {
                        state.modal = None;
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                        };
                    }
                    KeyCode::Down => {
                        // The renderer reconciles `scroll` against the
                        // actual flattened row count; we use a large
                        // placeholder max here and let the renderer clamp.
                        ps.scroll_down(usize::MAX);
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                        };
                    }
                    KeyCode::Up => {
                        ps.scroll_up();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                        };
                    }
                    KeyCode::PageDown => {
                        ps.page_down(10, usize::MAX);
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                        };
                    }
                    KeyCode::PageUp => {
                        ps.page_up(10);
                        return UpdateResult {
                            redraw: true,
                            intent: None,
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
                        // Extract raw bytes from the content pane and build a PendingSave.
                        let pending = tracer::extract_raw_for_save(state, path);
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
        }
    }

    // Per-view key dispatch via ViewKeyHandler trait.
    if state.modal.is_none() {
        let consumed = match state.current_tab {
            ViewId::Overview => overview::OverviewHandler::handle_key(state, key),
            ViewId::Bulletins => bulletins::BulletinsHandler::handle_key(state, key),
            ViewId::Browser => browser::BrowserHandler::handle_key(state, key),
            ViewId::Events => events::EventsHandler::handle_key(state, key),
            ViewId::Tracer => tracer::TracerHandler::handle_key(state, key),
        };
        if let Some(r) = consumed {
            return r;
        }
    }

    // Global key handling.
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => {
            if state.status.banner.is_some() {
                state.status.banner = None;
                return UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                };
            }
            UpdateResult::default()
        }
        (KeyCode::Char('q'), KeyModifiers::NONE)
        | (KeyCode::Char('q'), KeyModifiers::CONTROL)
        | (KeyCode::Char('Q'), KeyModifiers::CONTROL) => {
            state.should_quit = true;
            UpdateResult {
                redraw: false,
                intent: Some(PendingIntent::Quit),
                tracer_followup: None,
            }
        }
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            state.should_quit = true;
            UpdateResult {
                redraw: false,
                intent: Some(PendingIntent::Quit),
                tracer_followup: None,
            }
        }
        (KeyCode::Char('['), KeyModifiers::NONE) => {
            let anchor = capture_anchor(state);
            let current = crate::app::history::HistoryEntry {
                tab: state.current_tab,
                anchor,
            };
            if let Some(entry) = state.history.pop_back(current) {
                state.current_tab = entry.tab;
                restore_anchor(state, &entry);
            }
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }
        (KeyCode::Char(']'), KeyModifiers::NONE) => {
            let anchor = capture_anchor(state);
            let current = crate::app::history::HistoryEntry {
                tab: state.current_tab,
                anchor,
            };
            if let Some(entry) = state.history.pop_forward(current) {
                state.current_tab = entry.tab;
                restore_anchor(state, &entry);
            }
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }
        (KeyCode::Tab, _) => {
            state.current_tab = state.current_tab.next();
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }
        (KeyCode::BackTab, _) => {
            state.current_tab = state.current_tab.prev();
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }
        (KeyCode::F(1), _) => {
            state.current_tab = ViewId::Overview;
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }
        (KeyCode::F(2), _) => {
            state.current_tab = ViewId::Bulletins;
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }
        (KeyCode::F(3), _) => {
            state.current_tab = ViewId::Browser;
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }
        (KeyCode::F(4), _) => {
            state.current_tab = ViewId::Events;
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }
        (KeyCode::F(5), _) => {
            state.current_tab = ViewId::Tracer;
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }
        (KeyCode::Char('?'), _) => {
            state.modal = Some(Modal::Help);
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }
        (KeyCode::Char('K'), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            let cs = ContextSwitcherState::from_config(
                config,
                &state.context_name,
                &state.detected_version,
            );
            state.modal = Some(Modal::ContextSwitcher(cs));
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }
        (KeyCode::Char('f'), KeyModifiers::NONE) => {
            if state.flow_index.is_none() {
                state.status.banner = Some(Banner {
                    severity: BannerSeverity::Warning,
                    message: "fuzzy find: flow not indexed yet, open Browser to seed".into(),
                    detail: None,
                });
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
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }
        (KeyCode::Char('e'), KeyModifiers::NONE) => {
            if let Some(b) = &state.status.banner
                && let Some(detail) = &b.detail
            {
                state.error_detail = Some(detail.clone());
                state.modal = Some(Modal::ErrorDetail);
                return UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                };
            }
            UpdateResult::default()
        }
        _ => UpdateResult::default(),
    }
}

// ---------------------------------------------------------------------------
// Helpers shared across sub-modules
// ---------------------------------------------------------------------------

fn handle_browser_payload(state: &mut AppState, payload: crate::event::BrowserPayload) {
    use crate::event::BrowserPayload;
    match payload {
        BrowserPayload::Tree(snap) => {
            apply_tree_snapshot(&mut state.browser, snap);
            state.flow_index = Some(build_flow_index(&state.browser));
        }
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
            state.tracer = TracerState::new();
            state.flow_index = None;

            // Signal the app loop to force-restart the current view worker.
            state.pending_worker_restart = true;

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
            state.status.banner = Some(Banner {
                severity: BannerSeverity::Info,
                message: format!("{intent_name}: not yet wired (Phase {phase})"),
                detail: None,
            });
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
            state.modal = None;
            state.error_detail = None;
            // Walk the arena for any node matching the component id.
            let target_arena = state
                .browser
                .nodes
                .iter()
                .position(|n| n.id == component_id);
            let Some(arena_idx) = target_arena else {
                state.status.banner = Some(Banner {
                    severity: BannerSeverity::Warning,
                    message: format!("component {component_id} not found in current flow tree"),
                    detail: None,
                });
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
            state.status.banner = Some(Banner {
                severity: BannerSeverity::Warning,
                message: format!("invalid UUID: {raw}"),
                detail: None,
            });
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
            let msg = err.to_string();
            state.status.banner = Some(Banner {
                severity: BannerSeverity::Error,
                message: msg.clone(),
                detail: Some(format!("{err:?}")),
            });
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
            state.status.banner = Some(Banner {
                severity: BannerSeverity::Info,
                message: format!("copied: {preview}"),
                detail: None,
            });
        }
        Err(err) => {
            state.status.banner = Some(Banner {
                severity: BannerSeverity::Warning,
                message: format!("clipboard: {err}"),
                detail: None,
            });
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
                },
            ],
        }
    }

    pub(super) fn key(code: KeyCode, mods: KeyModifiers) -> AppEvent {
        AppEvent::Input(Event::Key(KeyEvent::new(code, mods)))
    }

    pub(super) fn seeded_browser_state() -> (AppState, Config) {
        use crate::client::{NodeKind, NodeStatusSummary, RawNode, RecursiveSnapshot};
        use crate::event::{BrowserPayload, ViewPayload};
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
        update(
            &mut s,
            AppEvent::Data(ViewPayload::Browser(BrowserPayload::Tree(snap))),
            &c,
        );
        s.current_tab = ViewId::Browser;
        (s, c)
    }

    #[test]
    fn tab_cycles_forward() {
        let mut s = fresh_state();
        let c = tiny_config();
        update(&mut s, key(KeyCode::Tab, KeyModifiers::NONE), &c);
        assert_eq!(s.current_tab, ViewId::Bulletins);
        update(&mut s, key(KeyCode::Tab, KeyModifiers::NONE), &c);
        assert_eq!(s.current_tab, ViewId::Browser);
        update(&mut s, key(KeyCode::Tab, KeyModifiers::NONE), &c);
        assert_eq!(s.current_tab, ViewId::Events);
        update(&mut s, key(KeyCode::Tab, KeyModifiers::NONE), &c);
        assert_eq!(s.current_tab, ViewId::Tracer);
        update(&mut s, key(KeyCode::Tab, KeyModifiers::NONE), &c);
        assert_eq!(s.current_tab, ViewId::Overview);
    }

    #[test]
    fn back_tab_cycles_backward() {
        let mut s = fresh_state();
        let c = tiny_config();
        update(&mut s, key(KeyCode::BackTab, KeyModifiers::NONE), &c);
        assert_eq!(s.current_tab, ViewId::Tracer);
    }

    #[test]
    fn function_keys_jump_to_tabs() {
        let mut s = fresh_state();
        let c = tiny_config();
        update(&mut s, key(KeyCode::F(3), KeyModifiers::NONE), &c);
        assert_eq!(s.current_tab, ViewId::Browser);
        update(&mut s, key(KeyCode::F(4), KeyModifiers::NONE), &c);
        assert_eq!(s.current_tab, ViewId::Events);
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
    fn left_bracket_navigates_back() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        s.history.push(crate::app::history::HistoryEntry {
            tab: ViewId::Bulletins,
            anchor: None,
        });
        s.current_tab = ViewId::Browser;

        update(&mut s, key(KeyCode::Char('['), KeyModifiers::NONE), &c);
        assert_eq!(s.current_tab, ViewId::Bulletins);
    }

    #[test]
    fn right_bracket_navigates_forward() {
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

        update(&mut s, key(KeyCode::Char(']'), KeyModifiers::NONE), &c);
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
        // Open the modal via `f`.
        update(&mut s, key(KeyCode::Char('f'), KeyModifiers::NONE), &c);
        assert!(
            matches!(s.modal, Some(Modal::FuzzyFind(_))),
            "f should open the FuzzyFind modal"
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
    fn capital_k_opens_context_switcher_without_explicit_shift_modifier() {
        // Some terminals deliver capital letters as KeyCode::Char('K') with
        // KeyModifiers::NONE instead of SHIFT. Match the loose pattern used by
        // other capital handlers in the codebase.
        let mut s = fresh_state();
        let c = tiny_config();
        update(&mut s, key(KeyCode::Char('K'), KeyModifiers::NONE), &c);
        assert!(
            matches!(s.modal, Some(Modal::ContextSwitcher(_))),
            "K without SHIFT modifier should still open the context switcher"
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
    fn overview_sysdiag_payload_populates_cluster_summary() {
        use crate::event::{OverviewPayload, ViewPayload};

        let mut s = fresh_state();
        let c = tiny_config();

        // Pre-condition: cluster_summary is empty placeholder.
        assert_eq!(s.cluster_summary.connected_nodes, None);
        assert_eq!(s.cluster_summary.total_nodes, None);

        let diag = build_test_sysdiag_with_two_nodes();

        update(
            &mut s,
            AppEvent::Data(ViewPayload::Overview(OverviewPayload::SystemDiag(diag))),
            &c,
        );

        assert_eq!(s.cluster_summary.total_nodes, Some(2));
        // NodeDiagnostics has no status field, so connected_nodes equals total.
        assert_eq!(s.cluster_summary.connected_nodes, Some(2));
    }

    #[test]
    fn overview_sysdiag_fallback_payload_sets_warning_banner() {
        use crate::event::{OverviewPayload, ViewPayload};

        let mut s = fresh_state();
        let c = tiny_config();
        let diag = build_test_sysdiag_with_two_nodes();

        update(
            &mut s,
            AppEvent::Data(ViewPayload::Overview(OverviewPayload::SystemDiagFallback {
                diag,
                warning: "nodewise diagnostics unavailable".into(),
            })),
            &c,
        );

        assert!(s.status.banner.is_some());
        let banner = s.status.banner.unwrap();
        assert_eq!(banner.severity, BannerSeverity::Warning);
        assert_eq!(banner.message, "nodewise diagnostics unavailable");
        // Cluster summary should still be populated even on fallback.
        assert_eq!(s.cluster_summary.total_nodes, Some(2));
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
    fn properties_modal_scroll_uses_arrows_only_no_jk() {
        use crate::app::state::Modal;
        use crate::view::browser::state::PropertiesModalState;

        let mut s = fresh_state();
        let c = tiny_config();
        // Seed the Properties modal with scroll at 0.
        s.modal = Some(Modal::Properties(PropertiesModalState::new(1)));

        // j is a no-op inside the modal.
        update(&mut s, key(KeyCode::Char('j'), KeyModifiers::NONE), &c);
        let scroll_after_j = match s.modal.as_ref().unwrap() {
            Modal::Properties(ps) => ps.scroll,
            _ => panic!("expected Properties modal"),
        };
        assert_eq!(scroll_after_j, 0, "j dropped");

        // Down still scrolls.
        update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
        let scroll_after_down = match s.modal.as_ref().unwrap() {
            Modal::Properties(ps) => ps.scroll,
            _ => panic!("expected Properties modal"),
        };
        assert!(scroll_after_down > 0, "Down still works");

        let before = scroll_after_down;
        // k is a no-op.
        update(&mut s, key(KeyCode::Char('k'), KeyModifiers::NONE), &c);
        let scroll_after_k = match s.modal.as_ref().unwrap() {
            Modal::Properties(ps) => ps.scroll,
            _ => panic!("expected Properties modal"),
        };
        assert_eq!(scroll_after_k, before, "k dropped");

        // Up still scrolls back.
        update(&mut s, key(KeyCode::Up, KeyModifiers::NONE), &c);
        let scroll_after_up = match s.modal.as_ref().unwrap() {
            Modal::Properties(ps) => ps.scroll,
            _ => panic!("expected Properties modal"),
        };
        assert!(scroll_after_up < before, "Up still works");
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
}
