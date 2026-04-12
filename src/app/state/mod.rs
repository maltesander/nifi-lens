//! AppState and the pure state reducer.
//!
//! The reducer folds AppEvent into AppState and returns whether a redraw
//! is needed. State is owned exclusively by the UI task.

mod browser;
mod bulletins;
mod health;
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
use crate::view::health::state::HealthState;
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
    Health,
    Browser,
    Tracer,
}

impl ViewId {
    pub fn next(self) -> Self {
        match self {
            Self::Overview => Self::Bulletins,
            Self::Bulletins => Self::Health,
            Self::Health => Self::Browser,
            Self::Browser => Self::Tracer,
            Self::Tracer => Self::Overview,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Overview => Self::Tracer,
            Self::Bulletins => Self::Overview,
            Self::Health => Self::Bulletins,
            Self::Browser => Self::Health,
            Self::Tracer => Self::Browser,
        }
    }
}

#[derive(Debug)]
pub struct AppState {
    pub current_tab: ViewId,
    pub context_name: String,
    pub detected_version: Version,
    pub last_refresh: Instant,
    pub modal: Option<Modal>,
    pub overview: OverviewState,
    pub bulletins: BulletinsState,
    pub browser: BrowserState,
    pub health: HealthState,
    pub tracer: TracerState,
    pub flow_index: Option<FlowIndex>,
    pub status: StatusLine,
    pub error_detail: Option<String>,
    pub should_quit: bool,
    /// Set by the context-switch handler so the app loop can force-restart
    /// the current view worker (the registry no-ops when the view matches).
    pub pending_worker_restart: bool,
}

impl AppState {
    pub fn new(context_name: String, detected_version: Version, config: &Config) -> Self {
        Self {
            current_tab: ViewId::Overview,
            context_name,
            detected_version,
            last_refresh: Instant::now(),
            modal: None,
            overview: OverviewState::new(),
            bulletins: BulletinsState::with_capacity(config.bulletins.ring_size),
            browser: BrowserState::new(),
            health: HealthState::new(),
            tracer: TracerState::new(),
            flow_index: None,
            status: StatusLine::default(),
            error_detail: None,
            should_quit: false,
            pending_worker_restart: false,
        }
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
                    key: "j/k",
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
                    key: "j/k",
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
        ViewId::Health => health::HealthHandler::hints(state),
        ViewId::Browser => browser::BrowserHandler::hints(state),
        ViewId::Tracer => tracer::TracerHandler::hints(state),
    };

    // Global hints appended
    // TODO: wire history in Task 5
    // if state.history.can_go_back() {
    //     hints.push(HintSpan { key: "Alt+\u{2190}", action: "back" });
    // }
    // if state.history.can_go_forward() {
    //     hints.push(HintSpan { key: "Alt+\u{2192}", action: "fwd" });
    // }
    hints.push(HintSpan {
        key: "?",
        action: "help",
    });

    hints
}

// ---------------------------------------------------------------------------
// Top-level update() entry point
// ---------------------------------------------------------------------------

pub fn update(state: &mut AppState, event: AppEvent, config: &Config) -> UpdateResult {
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
        AppEvent::Data(ViewPayload::Health(payload)) => {
            match payload {
                crate::event::HealthPayload::PgStatus(snap) => {
                    crate::view::health::state::apply_pg_status(&mut state.health, snap);
                }
                crate::event::HealthPayload::SystemDiag(diag) => {
                    crate::view::health::state::apply_system_diagnostics(&mut state.health, diag);
                }
                crate::event::HealthPayload::SystemDiagFallback { diag, warning } => {
                    crate::view::health::state::apply_system_diagnostics(&mut state.health, diag);
                    state.status.banner = Some(Banner {
                        severity: BannerSeverity::Warning,
                        message: warning,
                        detail: None,
                    });
                }
            }
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
                    KeyCode::Down | KeyCode::Char('j') => {
                        cs.move_cursor_down();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                        };
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
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
                    (KeyCode::Esc, _) | (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
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
                    KeyCode::Down | KeyCode::Char('j') => {
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
                    KeyCode::Up | KeyCode::Char('k') => {
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
            ViewId::Health => health::HealthHandler::handle_key(state, key),
            ViewId::Browser => browser::BrowserHandler::handle_key(state, key),
            ViewId::Tracer => tracer::TracerHandler::handle_key(state, key),
        };
        if let Some(r) = consumed {
            return r;
        }
    }

    // Global key handling.
    match (key.code, key.modifiers) {
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
            state.current_tab = ViewId::Health;
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }
        (KeyCode::F(4), _) => {
            state.current_tab = ViewId::Browser;
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
        (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
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
        (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
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
            state.health = HealthState::new();
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
fn clipboard_copy(state: &mut AppState, text: &str) {
    match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text.to_string())) {
        Ok(()) => {
            state.status.banner = Some(Banner {
                severity: BannerSeverity::Info,
                message: format!("copied: {text}"),
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
        assert_eq!(s.current_tab, ViewId::Health);
        update(&mut s, key(KeyCode::Tab, KeyModifiers::NONE), &c);
        assert_eq!(s.current_tab, ViewId::Browser);
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
        assert_eq!(s.current_tab, ViewId::Health);
        update(&mut s, key(KeyCode::F(4), KeyModifiers::NONE), &c);
        assert_eq!(s.current_tab, ViewId::Browser);
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
    fn ctrl_k_opens_context_switcher() {
        let mut s = fresh_state();
        let c = tiny_config();
        update(&mut s, key(KeyCode::Char('k'), KeyModifiers::CONTROL), &c);
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
        update(&mut s, key(KeyCode::Char('k'), KeyModifiers::CONTROL), &c);
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
        update(&mut s, key(KeyCode::Char('k'), KeyModifiers::CONTROL), &c);
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
}
