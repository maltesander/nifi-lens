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
pub mod verb;

// Re-exports: downstream code imports from `crate::input`, not the
// submodules, so the module boundary can be moved later without
// touching callers.
pub use action::{AppAction, FocusAction, GoTarget, HistoryAction, TabAction};
pub use verb::{
    BrowserVerb, BulletinsVerb, EventsVerb, FilterField, Severity, TracerVerb, ViewVerb,
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
        &mut self,
        key: crossterm::event::KeyEvent,
        active_view: crate::app::state::ViewId,
    ) -> InputEvent {
        use crossterm::event::{KeyCode, KeyModifiers};

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
        use crate::app::state::ViewId;
        match active_view {
            ViewId::Bulletins => {
                for &v in BulletinsVerb::all() {
                    if chord_matches(v.chord(), key) {
                        return InputEvent::View(ViewVerb::Bulletins(v));
                    }
                }
            }
            ViewId::Browser => {
                for &v in BrowserVerb::all() {
                    if chord_matches(v.chord(), key) {
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
            AppAction, BrowserVerb, BulletinsVerb, EventsVerb, FocusAction, HistoryAction,
            TabAction, TracerVerb, Verb,
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
        out
    }
}

fn chord_matches(chord: Chord, key: crossterm::event::KeyEvent) -> bool {
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

    #[test]
    fn enter_translates_to_focus_descend() {
        let mut km = KeyMap::default();
        assert_eq!(
            km.translate(press(KeyCode::Enter), ViewId::Overview),
            InputEvent::Focus(FocusAction::Descend)
        );
    }

    #[test]
    fn esc_translates_to_focus_ascend() {
        let mut km = KeyMap::default();
        assert_eq!(
            km.translate(press(KeyCode::Esc), ViewId::Overview),
            InputEvent::Focus(FocusAction::Ascend)
        );
    }

    #[test]
    fn shift_left_is_history_back() {
        let mut km = KeyMap::default();
        assert_eq!(
            km.translate(
                press_mod(KeyCode::Left, KeyModifiers::SHIFT),
                ViewId::Overview
            ),
            InputEvent::History(HistoryAction::Back)
        );
    }

    #[test]
    fn bracket_still_unmapped_after_cleanup() {
        let mut km = KeyMap::default();
        assert_eq!(
            km.translate(press(KeyCode::Char('[')), ViewId::Overview),
            InputEvent::Unmapped
        );
        assert_eq!(
            km.translate(press(KeyCode::Char(']')), ViewId::Overview),
            InputEvent::Unmapped
        );
    }

    #[test]
    fn tab_is_focus_next_pane() {
        let mut km = KeyMap::default();
        assert_eq!(
            km.translate(press(KeyCode::Tab), ViewId::Overview),
            InputEvent::Focus(FocusAction::NextPane)
        );
    }

    #[test]
    fn back_tab_is_focus_prev_pane() {
        let mut km = KeyMap::default();
        assert_eq!(
            km.translate(press(KeyCode::BackTab), ViewId::Overview),
            InputEvent::Focus(FocusAction::PrevPane)
        );
    }

    #[test]
    fn f3_is_tab_goto_3() {
        let mut km = KeyMap::default();
        assert_eq!(
            km.translate(press(KeyCode::F(3)), ViewId::Overview),
            InputEvent::Tab(TabAction::Goto(3))
        );
    }

    #[test]
    fn q_is_quit() {
        let mut km = KeyMap::default();
        assert_eq!(
            km.translate(press(KeyCode::Char('q')), ViewId::Overview),
            InputEvent::App(AppAction::Quit)
        );
    }

    #[test]
    fn ctrl_c_is_quit() {
        let mut km = KeyMap::default();
        assert_eq!(
            km.translate(
                press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
                ViewId::Overview
            ),
            InputEvent::App(AppAction::Quit)
        );
    }

    #[test]
    fn bare_g_produces_app_goto() {
        let mut km = KeyMap::default();
        assert_eq!(
            km.translate(press(KeyCode::Char('g')), ViewId::Overview),
            InputEvent::App(AppAction::Goto)
        );
    }

    #[test]
    fn j_and_k_are_unmapped() {
        let mut km = KeyMap::default();
        assert_eq!(
            km.translate(press(KeyCode::Char('j')), ViewId::Overview),
            InputEvent::Unmapped
        );
        assert_eq!(
            km.translate(press(KeyCode::Char('k')), ViewId::Overview),
            InputEvent::Unmapped
        );
    }

    #[test]
    fn r_on_events_produces_events_refresh_not_bulletins_refresh() {
        // Cross-view chord collision: `r` is bound to both BulletinsVerb::Refresh
        // and EventsVerb::Refresh. With view-aware translate, the active view wins.
        let mut km = KeyMap::default();
        assert_eq!(
            km.translate(press(KeyCode::Char('r')), ViewId::Events),
            InputEvent::View(ViewVerb::Events(EventsVerb::Refresh))
        );
        assert_eq!(
            km.translate(press(KeyCode::Char('r')), ViewId::Bulletins),
            InputEvent::View(ViewVerb::Bulletins(BulletinsVerb::Refresh))
        );
    }

    #[test]
    fn shift_t_on_events_produces_events_edit_types_not_bulletins_cycle() {
        // Cross-view chord collision: Shift+T is bound to both
        // BulletinsVerb::CycleTypeFilter and EventsVerb::EditField(Types).
        use crate::input::{EventsVerb, FilterField};
        let mut km = KeyMap::default();
        assert_eq!(
            km.translate(
                press_mod(KeyCode::Char('T'), KeyModifiers::SHIFT),
                ViewId::Events
            ),
            InputEvent::View(ViewVerb::Events(EventsVerb::EditField(FilterField::Types)))
        );
        assert_eq!(
            km.translate(
                press_mod(KeyCode::Char('T'), KeyModifiers::SHIFT),
                ViewId::Bulletins
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
        check::<crate::input::FocusAction>("FocusAction");
        check::<crate::input::HistoryAction>("HistoryAction");
        check::<crate::input::TabAction>("TabAction");
        check::<crate::input::AppAction>("AppAction");
        // GoTarget removed — no longer implements Verb
        check::<crate::input::BulletinsVerb>("BulletinsVerb");
        check::<crate::input::BrowserVerb>("BrowserVerb");
        check::<crate::input::EventsVerb>("EventsVerb");
        check::<crate::input::TracerVerb>("TracerVerb");
    }
}
