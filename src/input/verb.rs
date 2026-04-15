//! Per-view verb enums. Each implements `Verb` in Task 6+.

use crate::input::{Chord, HintContext, Verb};
use crossterm::event::KeyCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilterField {
    Time,
    Types,
    Source,
    Uuid,
    Attr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BulletinsVerb {
    ToggleSeverity(Severity),
    CycleTypeFilter,
    CycleGroupBy,
    TogglePause,
    MuteSource,
    CopyMessage,
    ClearFilters,
    OpenSearch,
    Refresh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BrowserVerb {
    Refresh,
    Copy,
    OpenProperties,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventsVerb {
    EditField(FilterField),
    NewQuery,
    Reset,
    RaiseCap,
    Refresh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TracerVerb {
    Refresh,
    Copy,
    Save,
    ToggleDiff,
}

impl Verb for BulletinsVerb {
    fn chord(self) -> Chord {
        match self {
            Self::ToggleSeverity(Severity::Error) => Chord::simple(KeyCode::Char('1')),
            Self::ToggleSeverity(Severity::Warning) => Chord::simple(KeyCode::Char('2')),
            Self::ToggleSeverity(Severity::Info) => Chord::simple(KeyCode::Char('3')),
            Self::CycleTypeFilter => Chord::shift(KeyCode::Char('T')),
            Self::CycleGroupBy => Chord::shift(KeyCode::Char('G')),
            Self::TogglePause => Chord::shift(KeyCode::Char('P')),
            Self::MuteSource => Chord::shift(KeyCode::Char('M')),
            Self::CopyMessage => Chord::simple(KeyCode::Char('c')),
            Self::ClearFilters => Chord::shift(KeyCode::Char('R')),
            Self::OpenSearch => Chord::simple(KeyCode::Char('/')),
            Self::Refresh => Chord::simple(KeyCode::Char('r')),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::ToggleSeverity(Severity::Error) => "toggle error filter",
            Self::ToggleSeverity(Severity::Warning) => "toggle warning filter",
            Self::ToggleSeverity(Severity::Info) => "toggle info filter",
            Self::CycleTypeFilter => "cycle component-type filter",
            Self::CycleGroupBy => "cycle group-by mode",
            Self::TogglePause => "pause / resume auto-scroll",
            Self::MuteSource => "mute selected source",
            Self::CopyMessage => "copy raw message to clipboard",
            Self::ClearFilters => "clear all filters",
            Self::OpenSearch => "open text search",
            Self::Refresh => "refresh",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::ToggleSeverity(Severity::Error) => "err",
            Self::ToggleSeverity(Severity::Warning) => "warn",
            Self::ToggleSeverity(Severity::Info) => "info",
            Self::CycleTypeFilter => "type",
            Self::CycleGroupBy => "group",
            Self::TogglePause => "pause",
            Self::MuteSource => "mute",
            Self::CopyMessage => "copy",
            Self::ClearFilters => "clear",
            Self::OpenSearch => "find",
            Self::Refresh => "refresh",
        }
    }
    fn priority(self) -> u8 {
        match self {
            Self::OpenSearch | Self::TogglePause => 80,
            Self::ClearFilters => 60,
            _ => 40,
        }
    }
    fn all() -> &'static [Self] {
        &[
            Self::ToggleSeverity(Severity::Error),
            Self::ToggleSeverity(Severity::Warning),
            Self::ToggleSeverity(Severity::Info),
            Self::CycleTypeFilter,
            Self::CycleGroupBy,
            Self::TogglePause,
            Self::MuteSource,
            Self::CopyMessage,
            Self::ClearFilters,
            Self::OpenSearch,
            Self::Refresh,
        ]
    }
}

impl Verb for BrowserVerb {
    fn chord(self) -> Chord {
        match self {
            Self::Refresh => Chord::simple(KeyCode::Char('r')),
            Self::Copy => Chord::simple(KeyCode::Char('c')),
            Self::OpenProperties => Chord::simple(KeyCode::Char('p')),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Refresh => "refresh flow",
            Self::Copy => "copy id / row value",
            Self::OpenProperties => "open properties",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::Refresh => "refresh",
            Self::Copy => "copy",
            Self::OpenProperties => "props",
        }
    }
    fn enabled(self, ctx: &HintContext<'_>) -> bool {
        use crate::app::state::ViewId;
        match self {
            Self::OpenProperties => {
                ctx.state.current_tab == ViewId::Browser
                    && ctx.state.browser_selection_has_properties()
            }
            _ => true,
        }
    }
    fn priority(self) -> u8 {
        50
    }
    fn all() -> &'static [Self] {
        &[Self::Refresh, Self::Copy, Self::OpenProperties]
    }
}

impl Verb for EventsVerb {
    fn chord(self) -> Chord {
        match self {
            Self::EditField(FilterField::Time) => Chord::shift(KeyCode::Char('D')),
            Self::EditField(FilterField::Types) => Chord::shift(KeyCode::Char('T')),
            Self::EditField(FilterField::Source) => Chord::shift(KeyCode::Char('S')),
            Self::EditField(FilterField::Uuid) => Chord::shift(KeyCode::Char('U')),
            Self::EditField(FilterField::Attr) => Chord::shift(KeyCode::Char('A')),
            Self::NewQuery => Chord::shift(KeyCode::Char('N')),
            Self::Reset => Chord::shift(KeyCode::Char('R')),
            Self::RaiseCap => Chord::shift(KeyCode::Char('L')),
            Self::Refresh => Chord::simple(KeyCode::Char('r')),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::EditField(FilterField::Time) => "edit Time filter",
            Self::EditField(FilterField::Types) => "edit Types filter",
            Self::EditField(FilterField::Source) => "edit Source filter",
            Self::EditField(FilterField::Uuid) => "edit UUID filter",
            Self::EditField(FilterField::Attr) => "edit Attributes filter",
            Self::NewQuery => "clear filters and submit new query",
            Self::Reset => "reset filters (no submit)",
            Self::RaiseCap => "raise result cap 500 -> 5000",
            Self::Refresh => "re-run current query",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::EditField(FilterField::Time) => "time",
            Self::EditField(FilterField::Types) => "types",
            Self::EditField(FilterField::Source) => "src",
            Self::EditField(FilterField::Uuid) => "uuid",
            Self::EditField(FilterField::Attr) => "attr",
            Self::NewQuery => "new",
            Self::Reset => "reset",
            Self::RaiseCap => "cap",
            Self::Refresh => "refresh",
        }
    }
    fn priority(self) -> u8 {
        match self {
            Self::EditField(_) | Self::NewQuery | Self::Reset | Self::RaiseCap => 10,
            Self::Refresh => 50,
        }
    }
    fn all() -> &'static [Self] {
        &[
            Self::EditField(FilterField::Time),
            Self::EditField(FilterField::Types),
            Self::EditField(FilterField::Source),
            Self::EditField(FilterField::Uuid),
            Self::EditField(FilterField::Attr),
            Self::NewQuery,
            Self::Reset,
            Self::RaiseCap,
            Self::Refresh,
        ]
    }
}

impl Verb for TracerVerb {
    fn chord(self) -> Chord {
        match self {
            Self::Refresh => Chord::simple(KeyCode::Char('r')),
            Self::Copy => Chord::simple(KeyCode::Char('c')),
            Self::Save => Chord::simple(KeyCode::Char('s')),
            Self::ToggleDiff => Chord::simple(KeyCode::Char('d')),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Refresh => "refresh lineage",
            Self::Copy => "copy UUID / attribute value",
            Self::Save => "save content to file",
            Self::ToggleDiff => "toggle attribute diff",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::Refresh => "refresh",
            Self::Copy => "copy",
            Self::Save => "save",
            Self::ToggleDiff => "diff",
        }
    }
    fn enabled(self, ctx: &HintContext<'_>) -> bool {
        use crate::app::state::ViewId;
        if ctx.state.current_tab != ViewId::Tracer {
            return false;
        }
        match self {
            Self::Save => ctx.state.tracer_content_tab_is_active(),
            Self::ToggleDiff => ctx.state.tracer_attributes_tab_is_active(),
            _ => true,
        }
    }
    fn priority(self) -> u8 {
        50
    }
    fn all() -> &'static [Self] {
        &[Self::Refresh, Self::Copy, Self::Save, Self::ToggleDiff]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewVerb {
    Bulletins(BulletinsVerb),
    Browser(BrowserVerb),
    Events(EventsVerb),
    Tracer(TracerVerb),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{Chord, Verb};
    use crossterm::event::KeyCode;

    #[test]
    fn bulletins_severity_uses_number_keys() {
        assert_eq!(
            BulletinsVerb::ToggleSeverity(Severity::Error).chord(),
            Chord::simple(KeyCode::Char('1'))
        );
        assert_eq!(
            BulletinsVerb::ToggleSeverity(Severity::Warning).chord(),
            Chord::simple(KeyCode::Char('2'))
        );
        assert_eq!(
            BulletinsVerb::ToggleSeverity(Severity::Info).chord(),
            Chord::simple(KeyCode::Char('3'))
        );
    }

    #[test]
    fn bulletins_group_by_is_shift_g() {
        assert_eq!(
            BulletinsVerb::CycleGroupBy.chord(),
            Chord::shift(KeyCode::Char('G'))
        );
    }

    #[test]
    fn bulletins_pause_is_shift_p() {
        assert_eq!(
            BulletinsVerb::TogglePause.chord(),
            Chord::shift(KeyCode::Char('P'))
        );
    }

    #[test]
    fn bulletins_mute_is_shift_m() {
        assert_eq!(
            BulletinsVerb::MuteSource.chord(),
            Chord::shift(KeyCode::Char('M'))
        );
    }

    #[test]
    fn bulletins_clear_is_shift_r() {
        assert_eq!(
            BulletinsVerb::ClearFilters.chord(),
            Chord::shift(KeyCode::Char('R'))
        );
    }

    #[test]
    fn bulletins_does_not_bind_g() {
        for v in BulletinsVerb::all() {
            let c = v.chord();
            assert_ne!(
                c.key,
                KeyCode::Char('g'),
                "Bulletins verb {v:?} must not bind bare `g` — that's the go-leader"
            );
        }
    }

    #[test]
    fn browser_properties_moved_off_e() {
        assert_eq!(
            BrowserVerb::OpenProperties.chord(),
            Chord::simple(KeyCode::Char('p'))
        );
    }

    #[test]
    fn events_filter_fields_use_shift_variants() {
        use EventsVerb::EditField;
        assert_eq!(
            EditField(FilterField::Time).chord(),
            Chord::shift(KeyCode::Char('D'))
        );
        assert_eq!(
            EditField(FilterField::Types).chord(),
            Chord::shift(KeyCode::Char('T'))
        );
        assert_eq!(
            EditField(FilterField::Source).chord(),
            Chord::shift(KeyCode::Char('S'))
        );
        assert_eq!(
            EditField(FilterField::Uuid).chord(),
            Chord::shift(KeyCode::Char('U'))
        );
        assert_eq!(
            EditField(FilterField::Attr).chord(),
            Chord::shift(KeyCode::Char('A'))
        );
    }

    #[test]
    fn events_new_query_is_shift_n() {
        assert_eq!(
            EventsVerb::NewQuery.chord(),
            Chord::shift(KeyCode::Char('N'))
        );
    }

    #[test]
    fn events_reset_is_shift_r() {
        assert_eq!(EventsVerb::Reset.chord(), Chord::shift(KeyCode::Char('R')));
    }

    #[test]
    fn events_refresh_is_r() {
        assert_eq!(
            EventsVerb::Refresh.chord(),
            Chord::simple(KeyCode::Char('r'))
        );
    }

    #[test]
    fn events_filter_verbs_have_priority_10() {
        use EventsVerb::EditField;
        assert_eq!(EditField(FilterField::Time).priority(), 10);
        assert_eq!(EditField(FilterField::Source).priority(), 10);
        assert_eq!(EventsVerb::Reset.priority(), 10);
    }

    #[test]
    fn tracer_diff_moved_off_a() {
        assert_eq!(
            TracerVerb::ToggleDiff.chord(),
            Chord::simple(KeyCode::Char('d'))
        );
    }

    #[test]
    fn no_view_verb_binds_j_or_k() {
        let chords = BulletinsVerb::all()
            .iter()
            .map(|v| v.chord())
            .chain(BrowserVerb::all().iter().map(|v| v.chord()))
            .chain(EventsVerb::all().iter().map(|v| v.chord()))
            .chain(TracerVerb::all().iter().map(|v| v.chord()));
        for c in chords {
            assert_ne!(c.key, KeyCode::Char('j'), "no view verb may bind j");
            assert_ne!(c.key, KeyCode::Char('k'), "no view verb may bind k");
        }
    }
}
