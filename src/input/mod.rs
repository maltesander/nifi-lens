//! Typed input layer.
//!
//! Converts raw `crossterm::KeyEvent`s into `InputEvent` values by way of
//! a `KeyMap`. Each `InputEvent` variant carries a typed enum
//! (`FocusAction`, `HistoryAction`, `TabAction`, `AppAction`, or
//! `ViewVerb`) that the reducer dispatches.
//!
//! Every enum implements [`Verb`], which is the single source of truth
//! for the chord that triggers it, the human label shown in the help
//! modal, the short form shown in the hint bar, its enabled predicate,
//! and its truncation priority.

pub mod action;
pub mod modal_gate;
pub mod verb;

// Re-exports: downstream code imports from `crate::input`, not the
// submodules, so the module boundary can be moved later without
// touching callers.
pub use action::{AppAction, FocusAction, GoTarget, HistoryAction, TabAction};
pub use verb::{
    ActionHistoryModalVerb, BrowserPeekVerb, BrowserQueueVerb, BrowserVerb, BulletinsVerb,
    CommonVerb, ContentModalVerb, EventsVerb, FilterField, ParameterContextModalVerb, Severity,
    TracerVerb, VersionControlModalVerb, ViewVerb,
};

// ---------------------------------------------------------------------------
// Chord — a single key-combination the keymap can recognize
// ---------------------------------------------------------------------------

use crossterm::event::{KeyCode, KeyModifiers};

/// A key combination that triggers one `Verb`. May be a single key
/// (optionally with modifiers) or a two-key leader combo like `g b`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    pub key: KeyCode,
    pub mods: KeyModifiers,
    /// When `Some`, this chord is a two-key combo — the user must press
    /// `leader` first, then `key`.
    pub leader: Option<KeyCode>,
}

impl Chord {
    pub const fn simple(key: KeyCode) -> Self {
        Self {
            key,
            mods: KeyModifiers::NONE,
            leader: None,
        }
    }

    pub const fn shift(key: KeyCode) -> Self {
        Self {
            key,
            mods: KeyModifiers::SHIFT,
            leader: None,
        }
    }

    pub const fn ctrl(key: KeyCode) -> Self {
        Self {
            key,
            mods: KeyModifiers::CONTROL,
            leader: None,
        }
    }

    /// Render the chord as a human-readable string used by the hint bar
    /// and help modal. Example: `"Shift+←"`, `"g b"`, `"Ctrl+c"`,
    /// `"F3"`, `"↑"`.
    pub fn display(self) -> String {
        if let Some(leader) = self.leader {
            return format!(
                "{} {}",
                render_key(leader, KeyModifiers::NONE),
                render_key(self.key, self.mods),
            );
        }
        render_key(self.key, self.mods)
    }
}

fn render_key(key: KeyCode, mods: KeyModifiers) -> String {
    let key_str = match key {
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".into(),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::BackTab => "Shift+Tab".into(),
        KeyCode::Backspace => "Backspace".into(),
        KeyCode::Up => "\u{2191}".into(),    // ↑
        KeyCode::Down => "\u{2193}".into(),  // ↓
        KeyCode::Left => "\u{2190}".into(),  // ←
        KeyCode::Right => "\u{2192}".into(), // →
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::PageUp => "PgUp".into(),
        KeyCode::PageDown => "PgDn".into(),
        KeyCode::F(n) => format!("F{n}"),
        other => format!("{other:?}"),
    };
    // `KeyCode::BackTab` already carries the Shift semantics; don't prefix.
    if matches!(key, KeyCode::BackTab) {
        return key_str;
    }
    let mut prefix = String::new();
    if mods.contains(KeyModifiers::CONTROL) {
        prefix.push_str("Ctrl+");
    }
    // For `Char` keys the uppercase letter already conveys Shift, so we
    // skip the `Shift+` prefix there (`Shift+D` → `D`). Keep it for
    // non-Char keys like arrows, where `←` alone would collide with
    // plain arrow navigation.
    if mods.contains(KeyModifiers::SHIFT) && !matches!(key, KeyCode::Char(_)) {
        prefix.push_str("Shift+");
    }
    if mods.contains(KeyModifiers::ALT) {
        prefix.push_str("Alt+");
    }
    format!("{prefix}{key_str}")
}

// ---------------------------------------------------------------------------
// Verb trait — single source of truth per action variant
// ---------------------------------------------------------------------------

/// Context passed to `Verb::enabled()`. Holds a borrow of
/// `AppState` so implementations can inspect the active tab,
/// the current selection, and any transient modal state to
/// decide whether a verb should render enabled in the hint bar
/// and dispatch on keypress.
///
/// The `state` field is public by design — `enabled()` impls
/// read whatever fields they need.
#[derive(Clone, Copy, Debug)]
pub struct HintContext<'a> {
    pub state: &'a crate::app::state::AppState,
}

impl<'a> HintContext<'a> {
    pub fn new(state: &'a crate::app::state::AppState) -> Self {
        Self { state }
    }
}

/// The contract every action enum implements. Lives in `input/` so that
/// adding a new variant forces the author to fill in every slot — no
/// string tables, no drift between binding and label.
pub trait Verb: Copy + 'static {
    /// The key or key combination that triggers this verb.
    fn chord(self) -> Chord;

    /// Long form shown in the help modal. Imperative phrase, lower
    /// case, no trailing punctuation — e.g. `"drill / activate /
    /// submit"`, `"toggle error filter"`, `"history back"`.
    fn label(self) -> &'static str;

    /// Short form used by the hint bar. Single word or short phrase.
    fn hint(self) -> &'static str;

    /// Whether the verb should render enabled for the given context.
    /// Disabled verbs still appear in the hint bar in muted style.
    fn enabled(self, _ctx: &HintContext<'_>) -> bool {
        true
    }

    /// Truncation priority for the hint bar. Higher verbs survive
    /// longer when the bar is narrow. Scale: `0..=255`. Suggested
    /// bands:
    ///
    /// - `100` — core verbs that must always be visible (`Enter`,
    ///   `Esc`, primary navigation).
    /// - `80` — frequently-used verbs (`?` help, search, submit
    ///   query).
    /// - `50` — default for most view-local verbs.
    /// - `20`..`40` — rarely-used controls (`RaiseCap`, debug flags).
    ///
    /// The buckets are advisory; the dispatcher never reads
    /// absolute numbers, only the relative ordering.
    fn priority(self) -> u8 {
        50
    }

    /// If false, this verb is advertised only in the help modal (`?`),
    /// not in the per-frame status-bar hint strip. Default: `true`.
    /// Use sparingly — only when a UI element adjacent to the hint bar
    /// already surfaces the same shortcut (e.g. the Bulletins
    /// `[E n] [W n] [I n]` chips surface `1/2/3`).
    fn show_in_hint_bar(self) -> bool {
        true
    }

    /// Canonical iteration order for the variants of this enum.
    /// Drives help-modal section layout and reverse-table
    /// construction.
    ///
    /// The `'static` slice means `Verb` is only suitable for
    /// unit-like enums or those with a small fixed set of
    /// parameterised variants (e.g. `TabAction::Goto(1..=5)`
    /// — each concrete `Jump(n)` is listed explicitly). Truly
    /// parametric actions (e.g. a future `GotoPg(Uuid)`) cannot
    /// implement `Verb` and must route through a different
    /// mechanism.
    fn all() -> &'static [Self];
}

// ---------------------------------------------------------------------------
// InputEvent — the output of KeyMap::translate
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    Focus(FocusAction),
    History(HistoryAction),
    Tab(TabAction),
    App(AppAction),
    View(ViewVerb),
    /// Key was recognized but doesn't map to anything.
    Unmapped,
}

// ---------------------------------------------------------------------------
// KeyMap — translates KeyEvent → InputEvent
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct KeyMap {}

impl KeyMap {
    pub fn translate(
        &self,
        mut key: crossterm::event::KeyEvent,
        state: &crate::app::state::AppState,
    ) -> InputEvent {
        use crossterm::event::{KeyCode, KeyModifiers};

        // Transport-level normalization: crossterm delivers Shift+Tab as
        // `KeyCode::BackTab` with `KeyModifiers::SHIFT` already set, but
        // `BackTab` *means* Shift+Tab — the extra SHIFT bit is redundant
        // and breaks the strict modifier equality in `chord_matches`.
        // Strip it so chords declared as `Chord::simple(BackTab)` match.
        if key.code == KeyCode::BackTab {
            key.modifiers.remove(KeyModifiers::SHIFT);
        }

        // Modal shadow gates: each modal owns its own keys while open.
        // Order doesn't matter — the gates' `is_active` predicates and
        // `host_view` checks are mutually exclusive (only one modal can
        // be open on a given tab at a time, and an app-wide modal blocks
        // all of them via `state.modal.is_none()` inside each gate).
        use crate::input::modal_gate::{
            self, ActionHistoryModalGate, BrowserPeekGate, ContentModalGate,
            ParameterContextModalGate, VersionControlModalGate,
        };
        if let Some(ev) = modal_gate::try_dispatch::<ContentModalGate>(state, key) {
            return ev;
        }
        if let Some(ev) = modal_gate::try_dispatch::<VersionControlModalGate>(state, key) {
            return ev;
        }
        if let Some(ev) = modal_gate::try_dispatch::<ParameterContextModalGate>(state, key) {
            return ev;
        }
        if let Some(ev) = modal_gate::try_dispatch::<ActionHistoryModalGate>(state, key) {
            return ev;
        }
        if let Some(ev) = modal_gate::try_dispatch::<BrowserPeekGate>(state, key) {
            return ev;
        }

        use crate::app::state::ViewId;

        // When the queue listing has focus on the Browser tab,
        // BrowserQueueVerb chords take priority — they shadow
        // BrowserVerb (`c`/`r` operate on the listing row, not the
        // tree). Vertical-scroll FocusAction chords pass through so
        // handle_focus can drive the row cursor (Up/Down/PgUp/PgDn/
        // Home/End). All other keys are blocked while listing focus
        // is active.
        if state.browser.listing_focused && state.current_tab == ViewId::Browser {
            if matches!(
                key.code,
                KeyCode::Char('c') | KeyCode::Char('q') | KeyCode::Char('Q')
            ) && key.modifiers.contains(KeyModifiers::CONTROL)
            {
                return InputEvent::App(AppAction::Quit);
            }
            // BrowserQueueVerb chords first (Tab/i/t/c/r/Esc) — these
            // own the verbs the listing reduces against.
            for &v in BrowserQueueVerb::all() {
                if chord_matches(v.chord(), key) {
                    return InputEvent::View(ViewVerb::BrowserQueue(v));
                }
            }
            // Scroll keys pass through as FocusAction — handle_focus's
            // listing-focus arm drives the row cursor.
            for &a in FocusAction::all() {
                if matches!(
                    a,
                    FocusAction::Up
                        | FocusAction::Down
                        | FocusAction::PageUp
                        | FocusAction::PageDown
                        | FocusAction::First
                        | FocusAction::Last
                ) && chord_matches(a.chord(), key)
                {
                    return InputEvent::Focus(a);
                }
            }
            // Allow the cross-tab Goto chord (`g`) through so the
            // listing-focused selection can hand off to Tracer / Events
            // via the standard goto menu. Help (`?`) similarly stays
            // reachable.
            if chord_matches(AppAction::Goto.chord(), key) {
                return InputEvent::App(AppAction::Goto);
            }
            if chord_matches(AppAction::Help.chord(), key) {
                return InputEvent::App(AppAction::Help);
            }
            return InputEvent::Unmapped;
        }

        // Reverse-lookup across framework enums (order matters: Focus
        // is highest priority so Esc/Enter always win).
        for &a in FocusAction::all() {
            if chord_matches(a.chord(), key) {
                return InputEvent::Focus(a);
            }
        }
        for &a in HistoryAction::all() {
            if chord_matches(a.chord(), key) {
                return InputEvent::History(a);
            }
        }
        for &a in TabAction::all() {
            if chord_matches(a.chord(), key) {
                return InputEvent::Tab(a);
            }
        }
        for &a in AppAction::all() {
            if chord_matches(a.chord(), key) {
                return InputEvent::App(a);
            }
        }
        // Ctrl+c and Ctrl+q alias to Quit.
        if matches!(
            key.code,
            KeyCode::Char('c') | KeyCode::Char('q') | KeyCode::Char('Q')
        ) && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            return InputEvent::App(AppAction::Quit);
        }

        // View-local verbs come last; only the active view's enum is
        // iterated to avoid cross-view chord collisions.
        match state.current_tab {
            ViewId::Bulletins => {
                for &v in BulletinsVerb::all() {
                    if chord_matches(v.chord(), key) {
                        return InputEvent::View(ViewVerb::Bulletins(v));
                    }
                }
            }
            ViewId::Browser => {
                use crate::input::Verb as _;
                let hint_ctx = HintContext::new(state);
                for &v in BrowserVerb::all() {
                    if chord_matches(v.chord(), key) && v.enabled(&hint_ctx) {
                        return InputEvent::View(ViewVerb::Browser(v));
                    }
                }
            }
            ViewId::Events => {
                for &v in EventsVerb::all() {
                    if chord_matches(v.chord(), key) {
                        return InputEvent::View(ViewVerb::Events(v));
                    }
                }
            }
            ViewId::Tracer => {
                for &v in TracerVerb::all() {
                    if chord_matches(v.chord(), key) {
                        return InputEvent::View(ViewVerb::Tracer(v));
                    }
                }
            }
            ViewId::Overview => {}
        }

        InputEvent::Unmapped
    }

    /// Iterate every registered chord and its symbolic source. Used by
    /// the F12 debug dump; never called at runtime except on that
    /// shortcut.
    pub fn reverse_table(&self) -> Vec<(String, String)> {
        use crate::input::{
            AppAction, BrowserVerb, BulletinsVerb, ContentModalVerb, EventsVerb, FocusAction,
            HistoryAction, ParameterContextModalVerb, TabAction, TracerVerb, Verb,
            VersionControlModalVerb,
        };
        let mut out: Vec<(String, String)> = Vec::new();
        for &v in FocusAction::all() {
            out.push((v.chord().display(), format!("FocusAction::{v:?}")));
        }
        for &v in HistoryAction::all() {
            out.push((v.chord().display(), format!("HistoryAction::{v:?}")));
        }
        for &v in TabAction::all() {
            out.push((v.chord().display(), format!("TabAction::{v:?}")));
        }
        for &v in AppAction::all() {
            out.push((v.chord().display(), format!("AppAction::{v:?}")));
        }
        for &v in BulletinsVerb::all() {
            out.push((v.chord().display(), format!("BulletinsVerb::{v:?}")));
        }
        for &v in BrowserVerb::all() {
            out.push((v.chord().display(), format!("BrowserVerb::{v:?}")));
        }
        for &v in EventsVerb::all() {
            out.push((v.chord().display(), format!("EventsVerb::{v:?}")));
        }
        for &v in TracerVerb::all() {
            out.push((v.chord().display(), format!("TracerVerb::{v:?}")));
        }
        for &v in ContentModalVerb::all() {
            out.push((v.chord().display(), format!("ContentModalVerb::{v:?}")));
        }
        for &v in VersionControlModalVerb::all() {
            out.push((
                v.chord().display(),
                format!("VersionControlModalVerb::{v:?}"),
            ));
        }
        for &v in ParameterContextModalVerb::all() {
            out.push((
                v.chord().display(),
                format!("ParameterContextModalVerb::{v:?}"),
            ));
        }
        out
    }
}

pub(crate) fn chord_matches(chord: Chord, key: crossterm::event::KeyEvent) -> bool {
    if chord.leader.is_some() {
        return false; // leader combos are not dispatched directly
    }
    chord.key == key.code && chord.mods == key.modifiers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chord_display_simple_letter() {
        assert_eq!(Chord::simple(KeyCode::Char('q')).display(), "q");
    }

    #[test]
    fn chord_display_ctrl_c() {
        assert_eq!(Chord::ctrl(KeyCode::Char('c')).display(), "Ctrl+c");
    }

    #[test]
    fn chord_display_shift_left() {
        assert_eq!(Chord::shift(KeyCode::Left).display(), "Shift+\u{2190}");
    }

    #[test]
    fn chord_display_function_key() {
        assert_eq!(Chord::simple(KeyCode::F(3)).display(), "F3");
    }

    #[test]
    fn chord_display_arrow_up() {
        assert_eq!(Chord::simple(KeyCode::Up).display(), "\u{2191}");
    }

    #[test]
    fn chord_display_backtab_is_shift_tab_without_extra_prefix() {
        // crossterm delivers Shift+Tab as KeyCode::BackTab with no
        // modifier bits set. The render_key short-circuit prevents us
        // from emitting "Shift+Shift+Tab" — this test locks it in.
        assert_eq!(
            Chord::simple(crossterm::event::KeyCode::BackTab).display(),
            "Shift+Tab"
        );
    }

    #[test]
    fn chord_display_enter_and_esc() {
        assert_eq!(Chord::simple(KeyCode::Enter).display(), "Enter");
        assert_eq!(Chord::simple(KeyCode::Esc).display(), "Esc");
    }
}

#[cfg(test)]
mod keymap_tests {
    use super::*;
    use crate::app::state::ViewId;
    use crate::input::{AppAction, FocusAction, HistoryAction, TabAction};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn press_mod(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }
    fn dummy_state() -> crate::app::state::AppState {
        crate::test_support::fresh_state()
    }
    /// Build a fresh test state with `current_tab` set to `view`. Used by
    /// translate tests that exercise per-view chord routing.
    fn state_for(view: ViewId) -> crate::app::state::AppState {
        let mut s = dummy_state();
        s.current_tab = view;
        s
    }
    /// Build a fresh Tracer-tab state with `tracer.content_modal` populated
    /// so `ContentModalGate::is_active` returns true.
    fn state_with_content_modal() -> crate::app::state::AppState {
        use crate::view::tracer::modal_state::{
            ContentModalHeader, ContentModalState, ContentModalTab, Diffable, SideBuffer,
        };
        use crate::widget::scroll::BidirectionalScrollState;
        let mut s = state_for(ViewId::Tracer);
        s.tracer.content_modal = Some(ContentModalState {
            event_id: 1,
            header: ContentModalHeader {
                event_type: "DROP".into(),
                event_timestamp_iso: String::new(),
                component_name: "x".into(),
                pg_path: "pg".into(),
                input_size: Some(0),
                output_size: Some(0),
                input_mime: None,
                output_mime: None,
                input_available: true,
                output_available: true,
            },
            active_tab: ContentModalTab::Input,
            last_nondiff_tab: ContentModalTab::Input,
            diffable: Diffable::Pending,
            input: SideBuffer::default(),
            output: SideBuffer::default(),
            diff_cache: None,
            scroll: BidirectionalScrollState::default(),
            search: None,
        });
        s
    }
    /// Build a fresh Browser-tab state with `browser.action_history_modal`
    /// populated so `ActionHistoryModalGate::is_active` returns true.
    fn state_with_action_history_modal() -> crate::app::state::AppState {
        use crate::view::browser::state::action_history_modal::ActionHistoryModalState;
        let mut s = state_for(ViewId::Browser);
        s.browser.action_history_modal = Some(ActionHistoryModalState::pending(
            "src".into(),
            "label".into(),
        ));
        s
    }
    /// Build a fresh Browser-tab state with `browser.queue_listing.peek`
    /// populated so `BrowserPeekGate::is_active` returns true.
    fn state_with_peek_modal() -> crate::app::state::AppState {
        use crate::view::browser::state::queue_listing::{
            PeekIdentity, QueueListingPeekState, QueueListingState,
        };
        use crate::widget::scroll::VerticalScrollState;
        use std::time::Duration;
        let mut s = state_for(ViewId::Browser);
        let mut listing = QueueListingState::pending("q".into(), "queue".into());
        listing.peek = Some(QueueListingPeekState {
            uuid: "ff".into(),
            queue_id: "q".into(),
            cluster_node_id: None,
            identity: PeekIdentity {
                uuid: "ff".into(),
                filename: None,
                size: 0,
                mime_type: None,
                content_claim: None,
                cluster_node_id: None,
                lineage_duration: Duration::from_secs(0),
                penalized: false,
            },
            attrs: None,
            error: None,
            scroll: VerticalScrollState::default(),
            search: None,
            fetch_handle: None,
        });
        s.browser.queue_listing = Some(listing);
        s
    }

    #[test]
    fn enter_translates_to_focus_descend() {
        let km = KeyMap::default();
        assert_eq!(
            km.translate(press(KeyCode::Enter), &dummy_state()),
            InputEvent::Focus(FocusAction::Descend)
        );
    }

    #[test]
    fn esc_translates_to_focus_ascend() {
        let km = KeyMap::default();
        assert_eq!(
            km.translate(press(KeyCode::Esc), &dummy_state()),
            InputEvent::Focus(FocusAction::Ascend)
        );
    }

    #[test]
    fn shift_left_is_history_back() {
        let km = KeyMap::default();
        assert_eq!(
            km.translate(
                press_mod(KeyCode::Left, KeyModifiers::SHIFT),
                &dummy_state(),
            ),
            InputEvent::History(HistoryAction::Back)
        );
    }

    #[test]
    fn bracket_still_unmapped_after_cleanup() {
        let km = KeyMap::default();
        assert_eq!(
            km.translate(press(KeyCode::Char('[')), &dummy_state()),
            InputEvent::Unmapped
        );
        assert_eq!(
            km.translate(press(KeyCode::Char(']')), &dummy_state()),
            InputEvent::Unmapped
        );
    }

    #[test]
    fn tab_is_focus_next_pane() {
        let km = KeyMap::default();
        assert_eq!(
            km.translate(press(KeyCode::Tab), &dummy_state()),
            InputEvent::Focus(FocusAction::NextPane)
        );
    }

    #[test]
    fn back_tab_is_focus_prev_pane() {
        let km = KeyMap::default();
        assert_eq!(
            km.translate(press(KeyCode::BackTab), &dummy_state()),
            InputEvent::Focus(FocusAction::PrevPane)
        );
    }

    #[test]
    fn shift_back_tab_is_focus_prev_pane() {
        // crossterm delivers Shift+Tab as KeyCode::BackTab with the SHIFT
        // modifier bit set — not with KeyModifiers::NONE. The chord table
        // must translate both the "bare" BackTab used internally by tests
        // and the SHIFT-decorated BackTab emitted by real terminals.
        let km = KeyMap::default();
        assert_eq!(
            km.translate(
                press_mod(KeyCode::BackTab, KeyModifiers::SHIFT),
                &dummy_state(),
            ),
            InputEvent::Focus(FocusAction::PrevPane)
        );
    }

    #[test]
    fn f3_is_tab_goto_3() {
        let km = KeyMap::default();
        assert_eq!(
            km.translate(press(KeyCode::F(3)), &dummy_state()),
            InputEvent::Tab(TabAction::Goto(3))
        );
    }

    #[test]
    fn q_is_quit() {
        let km = KeyMap::default();
        assert_eq!(
            km.translate(press(KeyCode::Char('q')), &dummy_state()),
            InputEvent::App(AppAction::Quit)
        );
    }

    #[test]
    fn ctrl_c_is_quit() {
        let km = KeyMap::default();
        assert_eq!(
            km.translate(
                press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
                &dummy_state(),
            ),
            InputEvent::App(AppAction::Quit)
        );
    }

    #[test]
    fn bare_g_produces_app_goto() {
        let km = KeyMap::default();
        assert_eq!(
            km.translate(press(KeyCode::Char('g')), &dummy_state()),
            InputEvent::App(AppAction::Goto)
        );
    }

    #[test]
    fn j_and_k_are_unmapped() {
        let km = KeyMap::default();
        assert_eq!(
            km.translate(press(KeyCode::Char('j')), &dummy_state()),
            InputEvent::Unmapped
        );
        assert_eq!(
            km.translate(press(KeyCode::Char('k')), &dummy_state()),
            InputEvent::Unmapped
        );
    }

    #[test]
    fn r_on_events_produces_events_refresh_not_bulletins_refresh() {
        // Cross-view chord collision: `r` is bound to both BulletinsVerb::Common(CommonVerb::Refresh)
        // and EventsVerb::Common(CommonVerb::Refresh). With view-aware translate, the active view wins.
        let km = KeyMap::default();
        assert_eq!(
            km.translate(press(KeyCode::Char('r')), &state_for(ViewId::Events)),
            InputEvent::View(ViewVerb::Events(EventsVerb::Common(CommonVerb::Refresh)))
        );
        assert_eq!(
            km.translate(press(KeyCode::Char('r')), &state_for(ViewId::Bulletins)),
            InputEvent::View(ViewVerb::Bulletins(BulletinsVerb::Common(
                CommonVerb::Refresh
            )))
        );
    }

    #[test]
    fn shift_t_on_events_produces_events_edit_types_not_bulletins_cycle() {
        // Cross-view chord collision: Shift+T is bound to both
        // BulletinsVerb::CycleTypeFilter and EventsVerb::EditField(Types).
        use crate::input::{EventsVerb, FilterField};
        let km = KeyMap::default();
        assert_eq!(
            km.translate(
                press_mod(KeyCode::Char('T'), KeyModifiers::SHIFT),
                &state_for(ViewId::Events),
            ),
            InputEvent::View(ViewVerb::Events(EventsVerb::EditField(FilterField::Types)))
        );
        assert_eq!(
            km.translate(
                press_mod(KeyCode::Char('T'), KeyModifiers::SHIFT),
                &state_for(ViewId::Bulletins),
            ),
            InputEvent::View(ViewVerb::Bulletins(BulletinsVerb::CycleTypeFilter))
        );
    }

    #[test]
    fn no_verb_binds_j_or_k_anywhere() {
        use crate::input::{BrowserVerb, BulletinsVerb, EventsVerb, TracerVerb, Verb};
        use crossterm::event::KeyCode;
        let all_chords = BulletinsVerb::all()
            .iter()
            .map(|v| v.chord())
            .chain(BrowserVerb::all().iter().map(|v| v.chord()))
            .chain(EventsVerb::all().iter().map(|v| v.chord()))
            .chain(TracerVerb::all().iter().map(|v| v.chord()));
        for c in all_chords {
            assert_ne!(c.key, KeyCode::Char('j'));
            assert_ne!(c.key, KeyCode::Char('k'));
        }
    }

    #[test]
    fn bracket_keys_are_never_bound() {
        use crate::input::{
            AppAction, BrowserVerb, BulletinsVerb, EventsVerb, FocusAction, HistoryAction,
            TabAction, TracerVerb, Verb,
        };
        use crossterm::event::KeyCode;
        let chords: Vec<Chord> = FocusAction::all()
            .iter()
            .map(|v| v.chord())
            .chain(HistoryAction::all().iter().map(|v| v.chord()))
            .chain(TabAction::all().iter().map(|v| v.chord()))
            .chain(AppAction::all().iter().map(|v| v.chord()))
            // GoTarget removed — no longer implements Verb
            .chain(BulletinsVerb::all().iter().map(|v| v.chord()))
            .chain(BrowserVerb::all().iter().map(|v| v.chord()))
            .chain(EventsVerb::all().iter().map(|v| v.chord()))
            .chain(TracerVerb::all().iter().map(|v| v.chord()))
            .collect();
        for c in chords {
            assert_ne!(c.key, KeyCode::Char('['));
            assert_ne!(c.key, KeyCode::Char(']'));
        }
    }

    #[test]
    fn all_chords_are_unique_within_namespace() {
        use crate::input::Verb;
        use std::collections::HashSet;

        fn check<V: Verb>(name: &str) {
            let mut seen: HashSet<(
                crossterm::event::KeyCode,
                crossterm::event::KeyModifiers,
                Option<crossterm::event::KeyCode>,
            )> = HashSet::new();
            for &v in V::all() {
                let c = v.chord();
                assert!(
                    seen.insert((c.key, c.mods, c.leader)),
                    "duplicate chord in {name}"
                );
            }
        }
        // BrowserVerb intentionally has two verbs on chord `p`:
        // `OpenProperties` (Processor/CS rows) and `OpenParameterContext`
        // (PG rows). The dispatcher resolves the collision via `enabled()`,
        // so uniqueness is by (chord, selection-kind), not chord alone.
        // We therefore skip the global uniqueness check for BrowserVerb
        // and instead assert the intentional pair explicitly.
        fn check_browser() {
            use crate::input::BrowserVerb;
            use crossterm::event::KeyCode;
            let p_chord = crate::input::Chord::simple(KeyCode::Char('p'));
            let p_verbs: Vec<BrowserVerb> = BrowserVerb::all()
                .iter()
                .copied()
                .filter(|v| v.chord() == p_chord)
                .collect();
            assert_eq!(
                p_verbs,
                vec![
                    BrowserVerb::OpenProperties,
                    BrowserVerb::OpenParameterContext
                ],
                "exactly OpenProperties and OpenParameterContext share chord `p`"
            );
            // All OTHER BrowserVerb chords are unique.
            let mut seen: HashSet<(
                crossterm::event::KeyCode,
                crossterm::event::KeyModifiers,
                Option<crossterm::event::KeyCode>,
            )> = HashSet::new();
            for &v in BrowserVerb::all() {
                if v.chord() == p_chord {
                    continue; // intentional shared chord
                }
                let c = v.chord();
                assert!(
                    seen.insert((c.key, c.mods, c.leader)),
                    "unexpected duplicate chord in BrowserVerb (non-p)"
                );
            }
        }
        check::<crate::input::FocusAction>("FocusAction");
        check::<crate::input::HistoryAction>("HistoryAction");
        check::<crate::input::TabAction>("TabAction");
        check::<crate::input::AppAction>("AppAction");
        // GoTarget removed — no longer implements Verb
        check::<crate::input::BulletinsVerb>("BulletinsVerb");
        check_browser();
        check::<crate::input::EventsVerb>("EventsVerb");
        check::<crate::input::TracerVerb>("TracerVerb");
        check::<crate::input::ContentModalVerb>("ContentModalVerb");
        check::<crate::input::VersionControlModalVerb>("VersionControlModalVerb");
        check::<crate::input::ParameterContextModalVerb>("ParameterContextModalVerb");
    }

    #[test]
    fn content_modal_open_shadows_tracer_verb_on_tracer_tab() {
        // When the content modal is open, ContentModalVerb chords win.
        // `i` is TracerVerb::OpenContentModal when modal is closed,
        // but there is no ContentModalVerb on `i`, so it should become Unmapped.
        // `c` is TracerVerb::Common(CommonVerb::Copy) when modal is closed;
        // it should become ContentModal(Copy) when the modal is open.
        let km = KeyMap::default();
        let modal_open = state_with_content_modal();
        // Modal open: `c` → ContentModal(Copy)
        assert_eq!(
            km.translate(press(KeyCode::Char('c')), &modal_open),
            InputEvent::View(ViewVerb::ContentModal(ContentModalVerb::Common(
                CommonVerb::Copy
            )))
        );
        // Modal open: `i` is not a ContentModalVerb chord → Unmapped
        assert_eq!(
            km.translate(press(KeyCode::Char('i')), &modal_open),
            InputEvent::Unmapped
        );
        // Modal closed: `i` → TracerVerb::OpenContentModal
        assert_eq!(
            km.translate(press(KeyCode::Char('i')), &state_for(ViewId::Tracer)),
            InputEvent::View(ViewVerb::Tracer(TracerVerb::OpenContentModal))
        );
    }

    #[test]
    fn content_modal_esc_becomes_close_not_focus_ascend() {
        // Esc is normally FocusAction::Ascend (priority slot), but
        // ContentModalVerb::Common(CommonVerb::Close) binds Esc and the
        // modal-open path checks ContentModalVerb BEFORE falling through to
        // FocusAction.
        let km = KeyMap::default();
        assert_eq!(
            km.translate(press(KeyCode::Esc), &state_with_content_modal()),
            InputEvent::View(ViewVerb::ContentModal(ContentModalVerb::Common(
                CommonVerb::Close
            )))
        );
    }

    #[test]
    fn content_modal_tab_becomes_switch_tab_next() {
        let km = KeyMap::default();
        assert_eq!(
            km.translate(press(KeyCode::Tab), &state_with_content_modal()),
            InputEvent::View(ViewVerb::ContentModal(ContentModalVerb::SwitchTabNext))
        );
    }

    #[test]
    fn ctrl_c_quits_even_when_modal_open() {
        let km = KeyMap::default();
        let modal_open = state_with_content_modal();
        assert_eq!(
            km.translate(
                press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
                &modal_open,
            ),
            InputEvent::App(AppAction::Quit)
        );
        assert_eq!(
            km.translate(
                press_mod(KeyCode::Char('q'), KeyModifiers::CONTROL),
                &modal_open,
            ),
            InputEvent::App(AppAction::Quit)
        );
    }

    #[test]
    fn modal_scroll_keys_pass_through_as_focus_action() {
        use crate::input::{FocusAction, InputEvent};
        let km = KeyMap::default();
        let modal_open = state_with_content_modal();
        assert_eq!(
            km.translate(press(KeyCode::Up), &modal_open),
            InputEvent::Focus(FocusAction::Up)
        );
        assert_eq!(
            km.translate(press(KeyCode::Down), &modal_open),
            InputEvent::Focus(FocusAction::Down)
        );
        assert_eq!(
            km.translate(press(KeyCode::PageUp), &modal_open),
            InputEvent::Focus(FocusAction::PageUp)
        );
        assert_eq!(
            km.translate(press(KeyCode::PageDown), &modal_open),
            InputEvent::Focus(FocusAction::PageDown)
        );
        assert_eq!(
            km.translate(press(KeyCode::Home), &modal_open),
            InputEvent::Focus(FocusAction::First)
        );
        assert_eq!(
            km.translate(press(KeyCode::End), &modal_open),
            InputEvent::Focus(FocusAction::Last)
        );
    }

    #[test]
    fn action_history_modal_open_shadows_browser_verb() {
        use crate::input::{ActionHistoryModalVerb, InputEvent, KeyMap, ViewVerb};
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let km = KeyMap::default();
        let state = state_with_action_history_modal();

        // 'c' inside the modal must route to ActionHistoryModalVerb::Common(CommonVerb::Copy)
        // (not BrowserVerb::Common(CommonVerb::Copy)).
        let ev = km.translate(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::empty()),
            &state,
        );
        assert!(matches!(
            ev,
            InputEvent::View(ViewVerb::ActionHistoryModal(
                ActionHistoryModalVerb::Common(CommonVerb::Copy)
            ))
        ));

        // Esc inside the modal routes to ActionHistoryModalVerb::Common(CommonVerb::Close)
        // (not FocusAction::Ascend).
        let ev = km.translate(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()), &state);
        assert!(matches!(
            ev,
            InputEvent::View(ViewVerb::ActionHistoryModal(
                ActionHistoryModalVerb::Common(CommonVerb::Close)
            ))
        ));

        // Ctrl+C must always quit even with the modal open.
        let ev = km.translate(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            &state,
        );
        assert!(matches!(ev, InputEvent::App(crate::input::AppAction::Quit)));
    }

    #[test]
    fn action_history_modal_only_shadows_on_browser_tab() {
        use crate::input::{InputEvent, KeyMap};
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let km = KeyMap::default();
        // Modal populated but current_tab forced to Tracer → gate must
        // not fire (host_view check); Esc falls through to FocusAction.
        let mut state = state_with_action_history_modal();
        state.current_tab = ViewId::Tracer;

        let ev = km.translate(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()), &state);
        // Esc on a non-Browser tab routes through FocusAction::Ascend.
        assert!(matches!(
            ev,
            InputEvent::Focus(crate::input::FocusAction::Ascend)
        ));
    }

    #[test]
    fn peek_modal_open_shadows_browser_verb() {
        use crate::input::{BrowserPeekVerb, InputEvent, KeyMap, ViewVerb};
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let km = KeyMap::default();
        let state = state_with_peek_modal();

        // 'c' inside the peek modal must route to BrowserPeekVerb::CopyAsJson
        // (not BrowserVerb::Common(CommonVerb::Copy)).
        let ev = km.translate(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::empty()),
            &state,
        );
        assert!(matches!(
            ev,
            InputEvent::View(ViewVerb::BrowserPeek(BrowserPeekVerb::CopyAsJson))
        ));

        // Esc inside the peek modal routes to BrowserPeekVerb::Common(CommonVerb::Close)
        // (not FocusAction::Ascend).
        let ev = km.translate(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()), &state);
        assert!(matches!(
            ev,
            InputEvent::View(ViewVerb::BrowserPeek(BrowserPeekVerb::Common(
                CommonVerb::Close
            )))
        ));

        // Ctrl+C must always quit even with the peek modal open.
        let ev = km.translate(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            &state,
        );
        assert!(matches!(ev, InputEvent::App(crate::input::AppAction::Quit)));
    }

    #[test]
    fn peek_modal_only_shadows_on_browser_tab() {
        use crate::input::{InputEvent, KeyMap};
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let km = KeyMap::default();
        // Peek modal populated but current_tab forced to Tracer → gate
        // must not fire (host_view check); Esc falls through to FocusAction.
        let mut state = state_with_peek_modal();
        state.current_tab = ViewId::Tracer;

        let ev = km.translate(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()), &state);
        // Esc on a non-Browser tab routes through FocusAction::Ascend.
        assert!(matches!(
            ev,
            InputEvent::Focus(crate::input::FocusAction::Ascend)
        ));
    }
}
