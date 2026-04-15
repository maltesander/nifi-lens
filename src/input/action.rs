//! Framework action enums.

use crate::input::{Chord, HintContext, Verb};
use crossterm::event::KeyCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FocusAction {
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    First,
    Last,
    Descend,
    Ascend,
    NextPane,
    PrevPane,
}

impl Verb for FocusAction {
    fn chord(self) -> Chord {
        match self {
            Self::Up => Chord::simple(KeyCode::Up),
            Self::Down => Chord::simple(KeyCode::Down),
            Self::Left => Chord::simple(KeyCode::Left),
            Self::Right => Chord::simple(KeyCode::Right),
            Self::PageUp => Chord::simple(KeyCode::PageUp),
            Self::PageDown => Chord::simple(KeyCode::PageDown),
            Self::First => Chord::simple(KeyCode::Home),
            Self::Last => Chord::simple(KeyCode::End),
            Self::Descend => Chord::simple(KeyCode::Enter),
            Self::Ascend => Chord::simple(KeyCode::Esc),
            Self::NextPane => Chord::simple(KeyCode::Tab),
            Self::PrevPane => Chord::simple(KeyCode::BackTab),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Up => "move selection up",
            Self::Down => "move selection down",
            Self::Left => "move left / collapse tree node",
            Self::Right => "move right / expand tree node",
            Self::PageUp => "page up",
            Self::PageDown => "page down",
            Self::First => "jump to first",
            Self::Last => "jump to last",
            Self::Descend => "drill / activate / submit",
            Self::Ascend => "leave focused pane / cancel",
            Self::NextPane => "focus next pane",
            Self::PrevPane => "focus previous pane",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::Up | Self::Down => "nav",
            Self::Left | Self::Right => "side",
            Self::PageUp | Self::PageDown => "page",
            Self::First | Self::Last => "jump",
            Self::Descend => "drill",
            Self::Ascend => "back",
            Self::NextPane | Self::PrevPane => "pane",
        }
    }

    fn enabled(self, _ctx: &HintContext<'_>) -> bool {
        true
    }

    fn priority(self) -> u8 {
        match self {
            Self::Descend | Self::Ascend => 100,
            Self::Up | Self::Down => 90,
            Self::Left | Self::Right => 70,
            Self::NextPane | Self::PrevPane => 60,
            _ => 40,
        }
    }

    fn all() -> &'static [Self] {
        &[
            Self::Up,
            Self::Down,
            Self::Left,
            Self::Right,
            Self::PageUp,
            Self::PageDown,
            Self::First,
            Self::Last,
            Self::Descend,
            Self::Ascend,
            Self::NextPane,
            Self::PrevPane,
        ]
    }
}

impl Verb for HistoryAction {
    fn chord(self) -> Chord {
        match self {
            Self::Back => Chord::shift(KeyCode::Left),
            Self::Forward => Chord::shift(KeyCode::Right),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Back => "history back",
            Self::Forward => "history forward",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::Back => "back",
            Self::Forward => "fwd",
        }
    }
    fn enabled(self, ctx: &HintContext<'_>) -> bool {
        match self {
            Self::Back => ctx.state.history.can_go_back(),
            Self::Forward => ctx.state.history.can_go_forward(),
        }
    }
    fn priority(self) -> u8 {
        30
    }
    fn all() -> &'static [Self] {
        &[Self::Back, Self::Forward]
    }
}

impl Verb for TabAction {
    fn chord(self) -> Chord {
        match self {
            Self::Jump(n) => Chord::simple(KeyCode::F(n)),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Jump(_) => "jump to tab",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::Jump(_) => "tab",
        }
    }
    fn priority(self) -> u8 {
        20
    }
    fn all() -> &'static [Self] {
        &[
            Self::Jump(1),
            Self::Jump(2),
            Self::Jump(3),
            Self::Jump(4),
            Self::Jump(5),
        ]
    }
}

impl Verb for AppAction {
    fn chord(self) -> Chord {
        match self {
            Self::Quit => Chord::simple(KeyCode::Char('q')),
            Self::Help => Chord::simple(KeyCode::Char('?')),
            Self::ContextSwitcher => Chord::shift(KeyCode::Char('K')),
            Self::FuzzyFind => Chord::shift(KeyCode::Char('F')),
            Self::Jump => Chord::simple(KeyCode::Char('g')),
            Self::Paste => Chord::simple(KeyCode::Char('v')),
            Self::Cut => Chord::simple(KeyCode::Char('x')),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Quit => "quit",
            Self::Help => "help",
            Self::ContextSwitcher => "switch cluster context",
            Self::FuzzyFind => "fuzzy find component",
            Self::Jump => "jump to related tab",
            Self::Paste => "paste from clipboard",
            Self::Cut => "cut to clipboard",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::Quit => "quit",
            Self::Help => "help",
            Self::ContextSwitcher => "ctx",
            Self::FuzzyFind => "find",
            Self::Jump => "jump",
            Self::Paste => "paste",
            Self::Cut => "cut",
        }
    }
    fn enabled(self, ctx: &HintContext<'_>) -> bool {
        match self {
            Self::Jump => !ctx.state.selection_cross_links().is_empty(),
            Self::Paste | Self::Cut => ctx.state.text_input_is_active(),
            _ => true,
        }
    }
    fn priority(self) -> u8 {
        match self {
            Self::Help => 80,
            Self::Jump => 60,
            Self::Paste | Self::Cut => 50,
            _ => 30,
        }
    }
    fn all() -> &'static [Self] {
        &[
            Self::Quit,
            Self::Help,
            Self::ContextSwitcher,
            Self::FuzzyFind,
            Self::Jump,
            Self::Paste,
            Self::Cut,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HistoryAction {
    Back,
    Forward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TabAction {
    Jump(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppAction {
    Quit,
    Help,
    ContextSwitcher,
    FuzzyFind,
    Jump,
    Paste,
    Cut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GoTarget {
    Browser,
    Events,
    Tracer,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{Chord, Verb};
    use crossterm::event::KeyCode;

    #[test]
    fn focus_all_covers_every_variant() {
        // Lexicographic-ish canonical order; this is also the order the
        // help modal renders.
        let expected = [
            FocusAction::Up,
            FocusAction::Down,
            FocusAction::Left,
            FocusAction::Right,
            FocusAction::PageUp,
            FocusAction::PageDown,
            FocusAction::First,
            FocusAction::Last,
            FocusAction::Descend,
            FocusAction::Ascend,
            FocusAction::NextPane,
            FocusAction::PrevPane,
        ];
        assert_eq!(FocusAction::all(), &expected);
    }

    #[test]
    fn focus_chords_match_spec() {
        assert_eq!(FocusAction::Up.chord(), Chord::simple(KeyCode::Up));
        assert_eq!(FocusAction::Down.chord(), Chord::simple(KeyCode::Down));
        assert_eq!(FocusAction::Left.chord(), Chord::simple(KeyCode::Left));
        assert_eq!(FocusAction::Right.chord(), Chord::simple(KeyCode::Right));
        assert_eq!(FocusAction::PageUp.chord(), Chord::simple(KeyCode::PageUp));
        assert_eq!(
            FocusAction::PageDown.chord(),
            Chord::simple(KeyCode::PageDown)
        );
        assert_eq!(FocusAction::First.chord(), Chord::simple(KeyCode::Home));
        assert_eq!(FocusAction::Last.chord(), Chord::simple(KeyCode::End));
        assert_eq!(FocusAction::Descend.chord(), Chord::simple(KeyCode::Enter));
        assert_eq!(FocusAction::Ascend.chord(), Chord::simple(KeyCode::Esc));
        assert_eq!(FocusAction::NextPane.chord(), Chord::simple(KeyCode::Tab));
        assert_eq!(
            FocusAction::PrevPane.chord(),
            Chord::simple(KeyCode::BackTab)
        );
    }

    #[test]
    fn focus_priority_ladder_is_locked() {
        // Lock in the full priority ladder so a future edit can't
        // silently demote a core motion into the rare bucket.
        assert_eq!(FocusAction::Descend.priority(), 100);
        assert_eq!(FocusAction::Ascend.priority(), 100);
        assert_eq!(FocusAction::Up.priority(), 90);
        assert_eq!(FocusAction::Down.priority(), 90);
        assert_eq!(FocusAction::Left.priority(), 70);
        assert_eq!(FocusAction::Right.priority(), 70);
        assert_eq!(FocusAction::PageUp.priority(), 40);
        assert_eq!(FocusAction::PageDown.priority(), 40);
        assert_eq!(FocusAction::First.priority(), 40);
        assert_eq!(FocusAction::Last.priority(), 40);
        assert_eq!(FocusAction::NextPane.priority(), 60);
        assert_eq!(FocusAction::PrevPane.priority(), 60);
    }

    #[test]
    fn history_chords() {
        assert_eq!(HistoryAction::Back.chord(), Chord::shift(KeyCode::Left));
        assert_eq!(HistoryAction::Forward.chord(), Chord::shift(KeyCode::Right));
    }

    #[test]
    fn tab_chords() {
        assert_eq!(TabAction::Jump(1).chord(), Chord::simple(KeyCode::F(1)));
        assert_eq!(TabAction::Jump(5).chord(), Chord::simple(KeyCode::F(5)));
    }

    #[test]
    fn tab_all_is_jumps_1_through_5() {
        let all = TabAction::all();
        assert_eq!(all[0], TabAction::Jump(1));
        assert_eq!(all[4], TabAction::Jump(5));
        assert_eq!(all.len(), 5);
    }

    #[test]
    fn app_chords() {
        assert_eq!(AppAction::Quit.chord(), Chord::simple(KeyCode::Char('q')));
        assert_eq!(AppAction::Help.chord(), Chord::simple(KeyCode::Char('?')));
        assert_eq!(
            AppAction::ContextSwitcher.chord(),
            Chord::shift(KeyCode::Char('K'))
        );
        assert_eq!(
            AppAction::FuzzyFind.chord(),
            Chord::shift(KeyCode::Char('F'))
        );
        assert_eq!(AppAction::Jump.chord(), Chord::simple(KeyCode::Char('g')));
        assert_eq!(AppAction::Paste.chord(), Chord::simple(KeyCode::Char('v')));
        assert_eq!(AppAction::Cut.chord(), Chord::simple(KeyCode::Char('x')));
    }

    #[test]
    fn app_all_covers_every_variant() {
        let all = AppAction::all();
        assert_eq!(all.len(), 7);
        assert!(all.contains(&AppAction::Quit));
        assert!(all.contains(&AppAction::Help));
        assert!(all.contains(&AppAction::ContextSwitcher));
        assert!(all.contains(&AppAction::FuzzyFind));
        assert!(all.contains(&AppAction::Jump));
        assert!(all.contains(&AppAction::Paste));
        assert!(all.contains(&AppAction::Cut));
    }

    #[test]
    fn fuzzy_find_chord_is_shift_f() {
        assert_eq!(
            AppAction::FuzzyFind.chord(),
            Chord::shift(KeyCode::Char('F'))
        );
    }

    #[test]
    fn jump_chord_is_bare_g() {
        assert_eq!(AppAction::Jump.chord(), Chord::simple(KeyCode::Char('g')));
    }

    #[test]
    fn paste_chord_is_v() {
        assert_eq!(AppAction::Paste.chord(), Chord::simple(KeyCode::Char('v')));
    }

    #[test]
    fn cut_chord_is_x() {
        assert_eq!(AppAction::Cut.chord(), Chord::simple(KeyCode::Char('x')));
    }
}
