//! AppState and the pure state reducer.
//!
//! The reducer folds AppEvent into AppState and returns whether a redraw
//! is needed. State is owned exclusively by the UI task.

use std::time::Instant;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use semver::Version;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::NifiLensError;
use crate::config::Config;
use crate::event::{AppEvent, IntentOutcome, ViewPayload};
use crate::intent::CrossLink;
use crate::view::browser::state::{
    BrowserState, FlowIndex, apply_tree_snapshot, build_flow_index, rebuild_visible,
};
use crate::view::bulletins::state::BulletinsState;
use crate::view::overview::{OverviewState, apply_payload as apply_overview_payload};
use crate::view::tracer::state::TracerState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewId {
    Overview,
    Bulletins,
    Browser,
    Tracer,
}

impl ViewId {
    pub fn next(self) -> Self {
        match self {
            Self::Overview => Self::Bulletins,
            Self::Bulletins => Self::Browser,
            Self::Browser => Self::Tracer,
            Self::Tracer => Self::Overview,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Overview => Self::Tracer,
            Self::Bulletins => Self::Overview,
            Self::Browser => Self::Bulletins,
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
    pub tracer: TracerState,
    pub flow_index: Option<FlowIndex>,
    pub status: StatusLine,
    pub error_detail: Option<String>,
    pub should_quit: bool,
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
            tracer: TracerState::new(),
            flow_index: None,
            status: StatusLine::default(),
            error_detail: None,
            should_quit: false,
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
}

/// An intent the reducer wants the caller to dispatch. The caller owns the
/// dispatcher (because it holds async state we can't touch inside the
/// reducer). The reducer just describes what it wants.
#[derive(Debug)]
pub enum PendingIntent {
    SwitchContext(String),
    JumpTo(CrossLink),
    Quit,
}

pub fn update(state: &mut AppState, event: AppEvent, config: &Config) -> UpdateResult {
    match event {
        AppEvent::Input(Event::Key(key)) => handle_key(state, key, config),
        AppEvent::Input(Event::Resize(_, _)) => UpdateResult {
            redraw: true,
            intent: None,
        },
        AppEvent::Input(_) => UpdateResult::default(),
        AppEvent::Tick => UpdateResult {
            redraw: false,
            intent: None,
        },
        AppEvent::Data(ViewPayload::Overview(payload)) => {
            apply_overview_payload(&mut state.overview, payload);
            state.last_refresh = Instant::now();
            UpdateResult {
                redraw: true,
                intent: None,
            }
        }
        AppEvent::Data(ViewPayload::Bulletins(payload)) => {
            crate::view::bulletins::state::apply_payload(&mut state.bulletins, payload);
            state.last_refresh = Instant::now();
            UpdateResult {
                redraw: true,
                intent: None,
            }
        }
        AppEvent::Data(ViewPayload::Browser(payload)) => {
            handle_browser_payload(state, payload);
            state.last_refresh = Instant::now();
            UpdateResult {
                redraw: true,
                intent: None,
            }
        }
        AppEvent::IntentOutcome(outcome) => handle_intent_outcome(state, outcome),
        AppEvent::Quit => {
            state.should_quit = true;
            UpdateResult {
                redraw: false,
                intent: Some(PendingIntent::Quit),
            }
        }
    }
}

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
                        };
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        cs.move_cursor_down();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                        };
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        cs.move_cursor_up();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
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
                        };
                    }
                    (KeyCode::Up, _) | (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
                        fs.move_up();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                        };
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
                        fs.move_down();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
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
                            };
                        }
                        return UpdateResult {
                            redraw: true,
                            intent: None,
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
                        };
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        ps.scroll_up();
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                        };
                    }
                    KeyCode::PageDown => {
                        ps.page_down(10, usize::MAX);
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                        };
                    }
                    KeyCode::PageUp => {
                        ps.page_up(10);
                        return UpdateResult {
                            redraw: true,
                            intent: None,
                        };
                    }
                    _ => return UpdateResult::default(),
                }
            }
        }
    }

    // Text-input mode captures character-level keys and edit keys (Esc,
    // Enter, Backspace). Keys with CONTROL modifiers (Ctrl+C, Ctrl+K, etc.)
    // skip this block so they reach the global handlers. Tab and other
    // unmodified keys are still suppressed to keep focus on text input.
    if state.current_tab == ViewId::Bulletins
        && state.bulletins.text_input.is_some()
        && matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT)
    {
        match key.code {
            KeyCode::Esc => {
                let prev = state.bulletins.selected_ring_index();
                state.bulletins.cancel_text_input(prev);
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::Enter => {
                let prev = state.bulletins.selected_ring_index();
                state.bulletins.commit_text_input(prev);
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::Backspace => {
                let prev = state.bulletins.selected_ring_index();
                state.bulletins.pop_text_input(prev);
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::Char(ch) => {
                let prev = state.bulletins.selected_ring_index();
                state.bulletins.push_text_input(ch, prev);
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            _ => return UpdateResult::default(),
        }
    }

    // Bulletins view-local keys take priority over global `e`. Accept
    // NONE or SHIFT modifiers so `G` and `T` (typed as Shift+g / Shift+t)
    // reach the handler — crossterm delivers them as
    // `KeyCode::Char('G')` with `KeyModifiers::SHIFT`.
    if state.current_tab == ViewId::Bulletins
        && matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT)
    {
        match key.code {
            KeyCode::Char('e') => {
                state.bulletins.toggle_error();
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::Char('w') => {
                state.bulletins.toggle_warning();
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::Char('i') => {
                state.bulletins.toggle_info();
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::Char('T') => {
                state.bulletins.cycle_component_type();
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::Char('c') => {
                state.bulletins.clear_filters();
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::Char('p') => {
                state.bulletins.toggle_pause();
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::Char('/') => {
                state.bulletins.enter_text_input_mode();
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::Char('g') | KeyCode::Home => {
                state.bulletins.jump_to_oldest();
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::Char('G') | KeyCode::End => {
                state.bulletins.jump_to_newest();
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::Up | KeyCode::Char('k') => {
                state.bulletins.move_selection_up();
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::Down | KeyCode::Char('j') => {
                state.bulletins.move_selection_down();
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::Enter => {
                if let Some(idx) = state.bulletins.selected_ring_index() {
                    let b = &state.bulletins.ring[idx];
                    let link = CrossLink::OpenInBrowser {
                        component_id: b.source_id.clone(),
                        group_id: b.group_id.clone(),
                    };
                    return UpdateResult {
                        redraw: true,
                        intent: Some(PendingIntent::JumpTo(link)),
                    };
                }
                return UpdateResult::default();
            }
            KeyCode::Char('t') => {
                if let Some(idx) = state.bulletins.selected_ring_index() {
                    let b = &state.bulletins.ring[idx];
                    let since = OffsetDateTime::parse(&b.timestamp_iso, &Rfc3339)
                        .ok()
                        .and_then(|dt| {
                            std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(
                                dt.unix_timestamp().max(0) as u64,
                            ))
                        })
                        .unwrap_or_else(std::time::SystemTime::now);
                    let link = CrossLink::TraceComponent {
                        component_id: b.source_id.clone(),
                        since,
                    };
                    return UpdateResult {
                        redraw: true,
                        intent: Some(PendingIntent::JumpTo(link)),
                    };
                }
                return UpdateResult::default();
            }
            _ => {}
        }
    }

    // Browser view-local keys. Active only when no modal is open and we
    // are on the Browser tab. Global keys (Tab, Ctrl+K, Ctrl+C, etc.)
    // continue to fall through to the global block. The `e`, `c`, `t`
    // keys land in Task 17.
    if state.current_tab == ViewId::Browser
        && state.modal.is_none()
        && matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT)
    {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                state.browser.move_up();
                state.browser.emit_detail_request_for_current_selection();
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::Down | KeyCode::Char('j') => {
                state.browser.move_down();
                state.browser.emit_detail_request_for_current_selection();
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::PageDown => {
                state.browser.page_down(10);
                state.browser.emit_detail_request_for_current_selection();
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::PageUp => {
                state.browser.page_up(10);
                state.browser.emit_detail_request_for_current_selection();
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::Home => {
                state.browser.jump_home();
                state.browser.emit_detail_request_for_current_selection();
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::End => {
                state.browser.jump_end();
                state.browser.emit_detail_request_for_current_selection();
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                state.browser.enter_selection();
                state.browser.emit_detail_request_for_current_selection();
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
                state.browser.backspace_selection();
                state.browser.emit_detail_request_for_current_selection();
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::Char('r') => {
                // Consume the force-tick oneshot. The worker is listening
                // and will fire an immediate tree fetch. Clearing the
                // sender prevents a second press from panicking.
                if let Some(tx) = state.browser.force_tick_tx.take() {
                    let _ = tx.send(());
                }
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::Char('e') => {
                // Open Properties modal only for Processor / CS with
                // detail loaded. No-op otherwise.
                let Some(&arena_idx) = state.browser.visible.get(state.browser.selected) else {
                    return UpdateResult::default();
                };
                let node_kind = state.browser.nodes[arena_idx].kind;
                let has_detail = state.browser.details.contains_key(&arena_idx);
                use crate::client::NodeKind as NK;
                if matches!(node_kind, NK::Processor | NK::ControllerService) && has_detail {
                    state.modal = Some(Modal::Properties(
                        crate::view::browser::state::PropertiesModalState::new(arena_idx),
                    ));
                    return UpdateResult {
                        redraw: true,
                        intent: None,
                    };
                }
                return UpdateResult::default();
            }
            KeyCode::Char('c') => {
                // Copy selected node's id to clipboard.
                let Some(&arena_idx) = state.browser.visible.get(state.browser.selected) else {
                    return UpdateResult::default();
                };
                let id = state.browser.nodes[arena_idx].id.clone();
                match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(id.clone())) {
                    Ok(()) => {
                        state.status.banner = Some(Banner {
                            severity: BannerSeverity::Info,
                            message: format!("copied id: {id}"),
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
                return UpdateResult {
                    redraw: true,
                    intent: None,
                };
            }
            KeyCode::Char('t') => {
                // Emit the Phase 4 TraceComponent cross-link for Processors only.
                let Some(&arena_idx) = state.browser.visible.get(state.browser.selected) else {
                    return UpdateResult::default();
                };
                let node = &state.browser.nodes[arena_idx];
                if !matches!(node.kind, crate::client::NodeKind::Processor) {
                    return UpdateResult::default();
                }
                let link = crate::intent::CrossLink::TraceComponent {
                    component_id: node.id.clone(),
                    since: std::time::SystemTime::now(),
                };
                return UpdateResult {
                    redraw: true,
                    intent: Some(PendingIntent::JumpTo(link)),
                };
            }
            _ => {}
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
            }
        }
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            state.should_quit = true;
            UpdateResult {
                redraw: false,
                intent: Some(PendingIntent::Quit),
            }
        }
        (KeyCode::Tab, _) => {
            state.current_tab = state.current_tab.next();
            UpdateResult {
                redraw: true,
                intent: None,
            }
        }
        (KeyCode::BackTab, _) => {
            state.current_tab = state.current_tab.prev();
            UpdateResult {
                redraw: true,
                intent: None,
            }
        }
        (KeyCode::F(1), _) => {
            state.current_tab = ViewId::Overview;
            UpdateResult {
                redraw: true,
                intent: None,
            }
        }
        (KeyCode::F(2), _) => {
            state.current_tab = ViewId::Bulletins;
            UpdateResult {
                redraw: true,
                intent: None,
            }
        }
        (KeyCode::F(3), _) => {
            state.current_tab = ViewId::Browser;
            UpdateResult {
                redraw: true,
                intent: None,
            }
        }
        (KeyCode::F(4), _) => {
            state.current_tab = ViewId::Tracer;
            UpdateResult {
                redraw: true,
                intent: None,
            }
        }
        (KeyCode::Char('?'), _) => {
            state.modal = Some(Modal::Help);
            UpdateResult {
                redraw: true,
                intent: None,
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
                };
            }
            UpdateResult::default()
        }
        _ => UpdateResult::default(),
    }
}

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
        Ok(IntentOutcome::ContextSwitched { new_version }) => {
            state.detected_version = new_version;
            state.last_refresh = Instant::now();
            state.modal = None;
            state.status.banner = None;
            UpdateResult {
                redraw: true,
                intent: None,
            }
        }
        Ok(IntentOutcome::ViewRefreshed { .. }) => {
            state.last_refresh = Instant::now();
            UpdateResult {
                redraw: true,
                intent: None,
            }
        }
        Ok(IntentOutcome::Quitting) => {
            state.should_quit = true;
            UpdateResult {
                redraw: false,
                intent: None,
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
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Context, Credentials, VersionStrategy};

    fn fresh_state() -> AppState {
        let c = tiny_config();
        AppState::new("dev".into(), Version::new(2, 9, 0), &c)
    }

    fn tiny_config() -> Config {
        Config {
            current_context: "dev".into(),
            bulletins: Default::default(),
            contexts: vec![
                Context {
                    name: "dev".into(),
                    url: "https://dev:8443".into(),
                    username: "admin".into(),
                    credentials: Credentials::Plain {
                        password: "x".into(),
                    },
                    version_strategy: VersionStrategy::Strict,
                    insecure_tls: false,
                    ca_cert_path: None,
                },
                Context {
                    name: "prod".into(),
                    url: "https://prod:8443".into(),
                    username: "admin".into(),
                    credentials: Credentials::Plain {
                        password: "y".into(),
                    },
                    version_strategy: VersionStrategy::Strict,
                    insecure_tls: false,
                    ca_cert_path: None,
                },
            ],
        }
    }

    fn key(code: KeyCode, mods: KeyModifiers) -> AppEvent {
        AppEvent::Input(Event::Key(KeyEvent::new(code, mods)))
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
        update(&mut s, key(KeyCode::F(1), KeyModifiers::NONE), &c);
        assert_eq!(s.current_tab, ViewId::Overview);
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
            new_version: Version::new(2, 7, 2),
        });
        update(&mut s, AppEvent::IntentOutcome(outcome), &c);
        assert_eq!(s.detected_version, Version::new(2, 7, 2));
        assert!(s.modal.is_none());
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
    fn bulletins_data_event_seeds_ring() {
        use crate::client::BulletinSnapshot;
        use crate::event::{BulletinsPayload, ViewPayload};
        use std::time::SystemTime;

        let mut s = fresh_state();
        let c = tiny_config();
        let payload = BulletinsPayload {
            bulletins: vec![BulletinSnapshot {
                id: 1,
                level: "ERROR".into(),
                message: "m".into(),
                source_id: "a".into(),
                source_name: "A".into(),
                source_type: "PROCESSOR".into(),
                group_id: "root".into(),
                timestamp_iso: "2026-04-11T10:14:22Z".into(),
            }],
            fetched_at: SystemTime::now(),
        };
        let r = update(&mut s, AppEvent::Data(ViewPayload::Bulletins(payload)), &c);
        assert!(r.redraw);
        assert_eq!(s.bulletins.ring.len(), 1);
    }

    #[test]
    fn on_bulletins_tab_e_toggles_error_chip() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        assert!(s.bulletins.filters.show_error);
        update(&mut s, key(KeyCode::Char('e'), KeyModifiers::NONE), &c);
        assert!(!s.bulletins.filters.show_error);
    }

    #[test]
    fn on_bulletins_tab_slash_enters_text_input_mode() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        update(&mut s, key(KeyCode::Char('/'), KeyModifiers::NONE), &c);
        assert!(s.bulletins.text_input.is_some());
    }

    #[test]
    fn bulletins_text_input_mode_consumes_chars_and_global_keys_are_suppressed() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        update(&mut s, key(KeyCode::Char('/'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('f'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('o'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('o'), KeyModifiers::NONE), &c);
        assert_eq!(s.bulletins.text_input.as_deref(), Some("foo"));
        // Tab should NOT cycle tabs while typing.
        update(&mut s, key(KeyCode::Tab, KeyModifiers::NONE), &c);
        assert_eq!(s.current_tab, ViewId::Bulletins);
        // Enter commits.
        update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        assert!(s.bulletins.text_input.is_none());
        assert_eq!(s.bulletins.filters.text, "foo");
    }

    #[test]
    fn on_bulletins_tab_enter_emits_jump_to_browser_intent() {
        use crate::client::BulletinSnapshot;
        use crate::event::{BulletinsPayload, ViewPayload};
        use std::time::SystemTime;

        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        // Seed one bulletin so there's a selection.
        let payload = BulletinsPayload {
            bulletins: vec![BulletinSnapshot {
                id: 1,
                level: "ERROR".into(),
                message: "m".into(),
                source_id: "proc-1".into(),
                source_name: "A".into(),
                source_type: "PROCESSOR".into(),
                group_id: "root".into(),
                timestamp_iso: "2026-04-11T10:14:22Z".into(),
            }],
            fetched_at: SystemTime::now(),
        };
        update(&mut s, AppEvent::Data(ViewPayload::Bulletins(payload)), &c);
        let r = update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        match r.intent {
            Some(PendingIntent::JumpTo(crate::intent::CrossLink::OpenInBrowser {
                component_id,
                group_id,
            })) => {
                assert_eq!(component_id, "proc-1");
                assert_eq!(group_id, "root");
            }
            other => panic!("expected JumpTo(OpenInBrowser), got {other:?}"),
        }
    }

    #[test]
    fn overview_data_event_updates_state_and_triggers_redraw() {
        use crate::client::{
            AboutSnapshot, BulletinBoardSnapshot, ControllerStatusSnapshot, RootPgStatusSnapshot,
        };
        use crate::event::{OverviewPayload, ViewPayload};
        use std::time::SystemTime;

        let mut s = fresh_state();
        let c = tiny_config();
        let payload = OverviewPayload {
            about: AboutSnapshot {
                version: "2.8.0".into(),
                title: "NiFi".into(),
            },
            controller: ControllerStatusSnapshot {
                running: 7,
                stopped: 3,
                invalid: 0,
                disabled: 1,
                active_threads: 0,
                flow_files_queued: 0,
                bytes_queued: 0,
            },
            root_pg: RootPgStatusSnapshot::default(),
            bulletin_board: BulletinBoardSnapshot::default(),
            fetched_at: SystemTime::now(),
        };
        let r = update(&mut s, AppEvent::Data(ViewPayload::Overview(payload)), &c);
        assert!(r.redraw);
        let snap = s.overview.snapshot.as_ref().unwrap();
        assert_eq!(snap.controller.running, 7);
    }

    #[test]
    fn text_input_mode_does_not_swallow_ctrl_c_quit() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        // Enter text-input mode.
        update(&mut s, key(KeyCode::Char('/'), KeyModifiers::NONE), &c);
        assert!(s.bulletins.text_input.is_some());
        // Type a character to verify normal input still works.
        update(&mut s, key(KeyCode::Char('f'), KeyModifiers::NONE), &c);
        assert_eq!(s.bulletins.text_input.as_deref(), Some("f"));
        // Ctrl+C should quit, NOT push 'c' into the buffer.
        let r = update(&mut s, key(KeyCode::Char('c'), KeyModifiers::CONTROL), &c);
        assert!(s.should_quit, "Ctrl+C should trigger quit");
        assert!(matches!(r.intent, Some(PendingIntent::Quit)));
        // The text buffer must not have been modified by the Ctrl+C keystroke.
        assert_eq!(
            s.bulletins.text_input.as_deref(),
            Some("f"),
            "Ctrl+C must not append 'c' to the filter buffer"
        );
    }

    #[test]
    fn text_input_mode_does_not_swallow_ctrl_k_context_switcher() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        update(&mut s, key(KeyCode::Char('/'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('f'), KeyModifiers::NONE), &c);
        // Ctrl+K should open the context switcher modal.
        update(&mut s, key(KeyCode::Char('k'), KeyModifiers::CONTROL), &c);
        assert!(
            matches!(s.modal, Some(Modal::ContextSwitcher(_))),
            "Ctrl+K should open the context switcher"
        );
        assert_eq!(
            s.bulletins.text_input.as_deref(),
            Some("f"),
            "Ctrl+K must not append 'k' to the filter buffer"
        );
    }

    #[test]
    fn browser_tree_payload_populates_browser_state_and_flow_index() {
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
                    name: "NiFi".into(),
                    status_summary: NodeStatusSummary::ProcessGroup {
                        running: 1,
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
            ],
            fetched_at: SystemTime::now(),
        };
        let r = update(
            &mut s,
            AppEvent::Data(ViewPayload::Browser(BrowserPayload::Tree(snap))),
            &c,
        );
        assert!(r.redraw);
        assert_eq!(s.browser.nodes.len(), 2);
        assert_eq!(s.browser.visible.len(), 2); // root expanded -> 1 child visible
        let idx = s.flow_index.as_ref().expect("FlowIndex built");
        assert_eq!(idx.entries.len(), 2);
    }

    #[test]
    fn open_in_browser_target_switches_tab_and_expands_ancestors() {
        use crate::client::{NodeKind, NodeStatusSummary, RawNode, RecursiveSnapshot};
        use crate::event::{BrowserPayload, ViewPayload};
        use std::time::SystemTime;

        let mut s = fresh_state();
        let c = tiny_config();
        // Seed a small tree: root → ingest → upd.
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
                RawNode {
                    parent_idx: Some(1),
                    kind: NodeKind::Processor,
                    id: "upd".into(),
                    group_id: "ingest".into(),
                    name: "UpdateAttribute".into(),
                    status_summary: NodeStatusSummary::Processor {
                        run_status: "Running".into(),
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

        // Jump to "upd".
        let outcome = Ok(IntentOutcome::OpenInBrowserTarget {
            component_id: "upd".into(),
            group_id: "ingest".into(),
        });
        update(&mut s, AppEvent::IntentOutcome(outcome), &c);
        assert_eq!(s.current_tab, ViewId::Browser);
        let arena = s.browser.nodes.iter().position(|n| n.id == "upd").unwrap();
        let visible = s.browser.visible.iter().position(|&i| i == arena).unwrap();
        assert_eq!(s.browser.selected, visible);
        // Ancestor expanded: "ingest" (arena 1) ∈ expanded.
        assert!(s.browser.expanded.contains(&1));
    }

    #[test]
    fn open_in_browser_target_warns_when_id_not_in_arena() {
        let mut s = fresh_state();
        let c = tiny_config();
        let outcome = Ok(IntentOutcome::OpenInBrowserTarget {
            component_id: "ghost".into(),
            group_id: "root".into(),
        });
        update(&mut s, AppEvent::IntentOutcome(outcome), &c);
        assert_eq!(s.current_tab, ViewId::Browser);
        let banner = s.status.banner.as_ref().unwrap();
        assert_eq!(banner.severity, BannerSeverity::Warning);
        assert!(banner.message.contains("ghost"));
    }

    fn seeded_browser_state() -> (AppState, Config) {
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
    fn on_browser_tab_j_moves_selection_down() {
        let (mut s, c) = seeded_browser_state();
        assert_eq!(s.browser.selected, 0);
        update(&mut s, key(KeyCode::Char('j'), KeyModifiers::NONE), &c);
        assert_eq!(s.browser.selected, 1);
    }

    #[test]
    fn on_browser_tab_enter_on_collapsed_pg_drills_in() {
        let (mut s, c) = seeded_browser_state();
        // Move selection to "ingest" (visible row 2 in a seeded tree with
        // root expanded and "gen" as first child).
        s.browser.selected = 2;
        update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        assert!(s.browser.expanded.contains(&2));
    }

    #[test]
    fn on_browser_tab_backspace_on_expanded_pg_collapses() {
        let (mut s, c) = seeded_browser_state();
        s.browser.expanded.insert(2);
        crate::view::browser::state::rebuild_visible(&mut s.browser);
        s.browser.selected = 2;
        update(&mut s, key(KeyCode::Backspace, KeyModifiers::NONE), &c);
        assert!(!s.browser.expanded.contains(&2));
    }

    #[test]
    fn on_browser_tab_r_fires_force_tick() {
        let (mut s, c) = seeded_browser_state();
        let (tx, _rx) = tokio::sync::oneshot::channel::<()>();
        s.browser.force_tick_tx = Some(tx);
        update(&mut s, key(KeyCode::Char('r'), KeyModifiers::NONE), &c);
        // Sender consumed; force_tick_tx is cleared.
        assert!(s.browser.force_tick_tx.is_none());
    }

    #[test]
    fn ctrl_f_with_no_index_shows_warning_banner_and_does_not_open_modal() {
        let mut s = fresh_state();
        let c = tiny_config();
        update(&mut s, key(KeyCode::Char('f'), KeyModifiers::CONTROL), &c);
        assert!(s.modal.is_none());
        assert!(
            s.status
                .banner
                .as_ref()
                .map(|b| b.severity == BannerSeverity::Warning)
                .unwrap_or(false)
        );
    }

    #[test]
    fn ctrl_f_with_index_opens_fuzzy_find_modal() {
        use crate::client::NodeKind;
        use crate::view::browser::state::{FlowIndex, FlowIndexEntry};
        let mut s = fresh_state();
        let c = tiny_config();
        s.flow_index = Some(FlowIndex {
            entries: vec![FlowIndexEntry {
                id: "p".into(),
                group_id: "root".into(),
                kind: NodeKind::Processor,
                display: "P   Processor   root".into(),
                haystack: "p   processor   root".into(),
            }],
        });
        update(&mut s, key(KeyCode::Char('f'), KeyModifiers::CONTROL), &c);
        assert!(matches!(s.modal, Some(Modal::FuzzyFind(_))));
    }

    #[test]
    fn fuzzy_find_modal_enter_emits_open_in_browser_intent() {
        use crate::client::NodeKind;
        use crate::intent::CrossLink;
        use crate::view::browser::state::{FlowIndex, FlowIndexEntry};

        let mut s = fresh_state();
        let c = tiny_config();
        s.flow_index = Some(FlowIndex {
            entries: vec![FlowIndexEntry {
                id: "target".into(),
                group_id: "g".into(),
                kind: NodeKind::Processor,
                display: "PutKafka   Processor   root".into(),
                haystack: "putkafka   processor   root".into(),
            }],
        });
        update(&mut s, key(KeyCode::Char('f'), KeyModifiers::CONTROL), &c);
        update(&mut s, key(KeyCode::Char('p'), KeyModifiers::NONE), &c);
        let r = update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        match r.intent {
            Some(PendingIntent::JumpTo(CrossLink::OpenInBrowser { component_id, .. })) => {
                assert_eq!(component_id, "target");
            }
            other => panic!("expected JumpTo(OpenInBrowser), got {other:?}"),
        }
        assert!(s.modal.is_none());
    }

    #[test]
    fn fuzzy_find_modal_esc_closes_without_jumping() {
        use crate::client::NodeKind;
        use crate::view::browser::state::{FlowIndex, FlowIndexEntry};

        let mut s = fresh_state();
        let c = tiny_config();
        s.flow_index = Some(FlowIndex {
            entries: vec![FlowIndexEntry {
                id: "x".into(),
                group_id: "g".into(),
                kind: NodeKind::Processor,
                display: "X   Processor   root".into(),
                haystack: "x   processor   root".into(),
            }],
        });
        update(&mut s, key(KeyCode::Char('f'), KeyModifiers::CONTROL), &c);
        let r = update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
        assert!(s.modal.is_none());
        assert!(r.intent.is_none());
    }

    #[test]
    fn e_on_processor_with_detail_opens_properties_modal() {
        use crate::client::ProcessorDetail;
        use crate::view::browser::state::NodeDetail;

        let (mut s, c) = seeded_browser_state();
        // Seed detail for "gen" (arena 1).
        s.browser.details.insert(
            1,
            NodeDetail::Processor(ProcessorDetail {
                id: "gen".into(),
                name: "Gen".into(),
                type_name: "x".into(),
                bundle: String::new(),
                run_status: "Running".into(),
                scheduling_strategy: String::new(),
                scheduling_period: String::new(),
                concurrent_tasks: 1,
                run_duration_ms: 0,
                penalty_duration: String::new(),
                yield_duration: String::new(),
                bulletin_level: String::new(),
                properties: vec![("k".into(), "v".into())],
                validation_errors: vec![],
            }),
        );
        s.browser.selected = 1; // visible row for arena 1
        update(&mut s, key(KeyCode::Char('e'), KeyModifiers::NONE), &c);
        assert!(matches!(s.modal, Some(Modal::Properties(_))));
    }

    #[test]
    fn e_on_processor_without_detail_is_noop() {
        let (mut s, c) = seeded_browser_state();
        s.browser.selected = 1;
        update(&mut s, key(KeyCode::Char('e'), KeyModifiers::NONE), &c);
        assert!(s.modal.is_none());
    }

    #[test]
    fn e_on_pg_is_noop() {
        let (mut s, c) = seeded_browser_state();
        s.browser.selected = 0; // root PG
        update(&mut s, key(KeyCode::Char('e'), KeyModifiers::NONE), &c);
        assert!(s.modal.is_none());
    }

    #[test]
    fn esc_closes_properties_modal() {
        use crate::view::browser::state::PropertiesModalState;
        let (mut s, c) = seeded_browser_state();
        s.modal = Some(Modal::Properties(PropertiesModalState::new(1)));
        update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
        assert!(s.modal.is_none());
    }

    #[test]
    fn t_on_processor_emits_trace_component_crosslink() {
        use crate::intent::CrossLink;
        let (mut s, c) = seeded_browser_state();
        s.browser.selected = 1; // "gen" processor
        let r = update(&mut s, key(KeyCode::Char('t'), KeyModifiers::NONE), &c);
        match r.intent {
            Some(PendingIntent::JumpTo(CrossLink::TraceComponent { component_id, .. })) => {
                assert_eq!(component_id, "gen");
            }
            other => panic!("expected TraceComponent, got {other:?}"),
        }
    }
}
