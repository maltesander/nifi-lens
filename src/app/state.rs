//! AppState and the pure state reducer.
//!
//! The reducer folds AppEvent into AppState and returns whether a redraw
//! is needed. State is owned exclusively by the UI task.

use std::time::Instant;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use semver::Version;

use crate::NifiLensError;
use crate::config::Config;
use crate::event::{AppEvent, IntentOutcome};

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
    pub status: StatusLine,
    pub error_detail: Option<String>,
    pub should_quit: bool,
}

impl AppState {
    pub fn new(context_name: String, detected_version: Version) -> Self {
        Self {
            current_tab: ViewId::Overview,
            context_name,
            detected_version,
            last_refresh: Instant::now(),
            modal: None,
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
        AppEvent::Data(_) => UpdateResult::default(),
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
            state.status.banner = Some(Banner {
                severity: BannerSeverity::Info,
                message: "fuzzy find: not yet implemented".into(),
                detail: None,
            });
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
        Ok(IntentOutcome::NotImplementedInPhase0 { intent_name }) => {
            state.status.banner = Some(Banner {
                severity: BannerSeverity::Info,
                message: format!("{intent_name}: not yet implemented in Phase 0"),
                detail: None,
            });
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
        AppState::new("dev".into(), Version::new(2, 9, 0))
    }

    fn tiny_config() -> Config {
        Config {
            current_context: "dev".into(),
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
}
