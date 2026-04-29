//! Generic shadow-gate dispatch for modal verbs. Each modal implements
//! `ModalGate` once; `KeyMap::translate` chains them via `try_dispatch::<G>`.
//! Replaces the near-identical `if … modal_open && view == … { … }` branches
//! that lived inline in `translate()` before this refactor.

use crate::app::state::{AppState, ViewId};
use crate::input::{AppAction, FocusAction, InputEvent, Verb, ViewVerb, chord_matches};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Whether unmatched scroll keys should fall through to `FocusAction`
/// dispatch when a modal is active. Tracer content modal lets `↑/↓/←/→/
/// PgUp/PgDn/Home/End` reach `handle_focus` so the body scrolls; other
/// modals own all their own keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollPassthrough {
    /// Modal owns every key — return `Unmapped` after a verb-table miss.
    None,
    /// Modal lets the listed `FocusAction` keys through to the focus
    /// dispatcher.
    Allow(&'static [FocusAction]),
}

/// Per-modal gate. Each modal implements this trait once; `translate()`
/// queries each gate in order until one claims the key.
pub trait ModalGate {
    /// The view this modal belongs to. Gate only fires when
    /// `state.current_tab == host_view()`.
    fn host_view() -> ViewId;

    /// True when the modal is currently open. Gate only fires when this
    /// returns `true`. Implementors should also check
    /// `state.modal.is_none()` so an app-wide modal (e.g. Save) layered
    /// over the modal-under-gate doesn't double-shadow.
    fn is_active(state: &AppState) -> bool;

    /// The modal's verb enum.
    type V: Verb;

    /// Wrap a claimed verb into a `ViewVerb` for the dispatcher.
    fn to_view_verb(v: Self::V) -> ViewVerb;

    /// Whether unmatched scroll keys should fall through to `FocusAction`
    /// dispatch. Default: no passthrough.
    fn scroll_passthrough() -> ScrollPassthrough {
        ScrollPassthrough::None
    }
}

/// Try to dispatch `key` against `G`'s modal verb table. Returns `Some`
/// if the gate is active and claimed the key (or returned `Unmapped`);
/// returns `None` if the gate is inactive (caller chains to the next gate).
///
/// `Ctrl+c` / `Ctrl+q` / `Ctrl+Q` always quit, even when a modal is open.
pub fn try_dispatch<G: ModalGate>(state: &AppState, key: KeyEvent) -> Option<InputEvent> {
    if !G::is_active(state) || state.current_tab != G::host_view() {
        return None;
    }
    if matches!(
        key.code,
        KeyCode::Char('c') | KeyCode::Char('q') | KeyCode::Char('Q')
    ) && key.modifiers.contains(KeyModifiers::CONTROL)
    {
        return Some(InputEvent::App(AppAction::Quit));
    }
    for &v in G::V::all() {
        if chord_matches(v.chord(), key) {
            return Some(InputEvent::View(G::to_view_verb(v)));
        }
    }
    if let ScrollPassthrough::Allow(actions) = G::scroll_passthrough() {
        for &a in actions {
            if chord_matches(a.chord(), key) {
                return Some(InputEvent::Focus(a));
            }
        }
    }
    Some(InputEvent::Unmapped)
}

/// Tracer content viewer modal (`src/view/tracer/modal/`).
pub struct ContentModalGate;

impl ModalGate for ContentModalGate {
    type V = crate::input::ContentModalVerb;

    fn host_view() -> ViewId {
        ViewId::Tracer
    }

    fn is_active(state: &AppState) -> bool {
        state.tracer.content_modal.is_some() && state.modal.is_none()
    }

    fn to_view_verb(v: Self::V) -> ViewVerb {
        ViewVerb::ContentModal(v)
    }

    fn scroll_passthrough() -> ScrollPassthrough {
        ScrollPassthrough::Allow(&[
            FocusAction::Up,
            FocusAction::Down,
            FocusAction::Left,
            FocusAction::Right,
            FocusAction::PageUp,
            FocusAction::PageDown,
            FocusAction::First,
            FocusAction::Last,
        ])
    }
}

/// Browser version-control modal (`src/view/browser/render/version_control_modal.rs`).
pub struct VersionControlModalGate;

impl ModalGate for VersionControlModalGate {
    type V = crate::input::VersionControlModalVerb;

    fn host_view() -> ViewId {
        ViewId::Browser
    }

    fn is_active(state: &AppState) -> bool {
        state.browser.version_modal.is_some() && state.modal.is_none()
    }

    fn to_view_verb(v: Self::V) -> ViewVerb {
        ViewVerb::VersionControlModal(v)
    }
}

/// Browser parameter-context modal (`src/view/browser/render/parameter_context_modal.rs`).
pub struct ParameterContextModalGate;

impl ModalGate for ParameterContextModalGate {
    type V = crate::input::ParameterContextModalVerb;

    fn host_view() -> ViewId {
        ViewId::Browser
    }

    fn is_active(state: &AppState) -> bool {
        state.browser.parameter_modal.is_some() && state.modal.is_none()
    }

    fn to_view_verb(v: Self::V) -> ViewVerb {
        ViewVerb::ParameterContextModal(v)
    }
}

/// Browser action-history modal (`src/view/browser/render/action_history_modal.rs`).
pub struct ActionHistoryModalGate;

impl ModalGate for ActionHistoryModalGate {
    type V = crate::input::ActionHistoryModalVerb;

    fn host_view() -> ViewId {
        ViewId::Browser
    }

    fn is_active(state: &AppState) -> bool {
        state.browser.action_history_modal.is_some() && state.modal.is_none()
    }

    fn to_view_verb(v: Self::V) -> ViewVerb {
        ViewVerb::ActionHistoryModal(v)
    }
}

/// Browser queue-listing peek modal (`src/view/browser/render/queue_listing_peek.rs`).
pub struct BrowserPeekGate;

impl ModalGate for BrowserPeekGate {
    type V = crate::input::BrowserPeekVerb;

    fn host_view() -> ViewId {
        ViewId::Browser
    }

    fn is_active(state: &AppState) -> bool {
        state
            .browser
            .queue_listing
            .as_ref()
            .and_then(|s| s.peek.as_ref())
            .is_some()
            && state.modal.is_none()
    }

    fn to_view_verb(v: Self::V) -> ViewVerb {
        ViewVerb::BrowserPeek(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Compile-time test: `ModalGate` is object-safe-adjacent (associated
    // types prevent dyn use, but the bounds chain compiles).
    fn _trait_compiles<G: ModalGate>(_p: std::marker::PhantomData<G>) {}

    #[test]
    fn content_modal_gate_inactive_on_fresh_state() {
        let state = crate::test_support::fresh_state();
        assert!(!ContentModalGate::is_active(&state));
    }

    #[test]
    fn content_modal_gate_host_view_is_tracer() {
        assert_eq!(ContentModalGate::host_view(), ViewId::Tracer);
    }

    #[test]
    fn version_control_modal_gate_inactive_on_fresh_state() {
        let state = crate::test_support::fresh_state();
        assert!(!VersionControlModalGate::is_active(&state));
    }

    #[test]
    fn parameter_context_modal_gate_inactive_on_fresh_state() {
        let state = crate::test_support::fresh_state();
        assert!(!ParameterContextModalGate::is_active(&state));
    }

    #[test]
    fn action_history_modal_gate_inactive_on_fresh_state() {
        let state = crate::test_support::fresh_state();
        assert!(!ActionHistoryModalGate::is_active(&state));
    }

    #[test]
    fn browser_peek_gate_inactive_on_fresh_state() {
        let state = crate::test_support::fresh_state();
        assert!(!BrowserPeekGate::is_active(&state));
    }

    #[test]
    fn all_browser_gates_host_browser() {
        assert_eq!(VersionControlModalGate::host_view(), ViewId::Browser);
        assert_eq!(ParameterContextModalGate::host_view(), ViewId::Browser);
        assert_eq!(ActionHistoryModalGate::host_view(), ViewId::Browser);
        assert_eq!(BrowserPeekGate::host_view(), ViewId::Browser);
    }
}
