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

/// Verbs shared across multiple views and modals: refresh, copy, search,
/// close. Each per-view verb enum that wants these chords embeds a
/// `Common(CommonVerb)` arm and lists which `CommonVerb` variants it
/// supports in its own `Verb::all()`. The chord/label/hint metadata is
/// defined exactly once, here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommonVerb {
    Refresh,
    Copy,
    OpenSearch,
    SearchNext,
    SearchPrev,
    Close,
}

impl Verb for CommonVerb {
    fn chord(self) -> Chord {
        match self {
            Self::Refresh => Chord::simple(KeyCode::Char('r')),
            Self::Copy => Chord::simple(KeyCode::Char('c')),
            Self::OpenSearch => Chord::simple(KeyCode::Char('/')),
            Self::SearchNext => Chord::simple(KeyCode::Char('n')),
            Self::SearchPrev => Chord::shift(KeyCode::Char('N')),
            Self::Close => Chord::simple(KeyCode::Esc),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Refresh => "refresh",
            Self::Copy => "copy",
            Self::OpenSearch => "open text search",
            Self::SearchNext => "next match",
            Self::SearchPrev => "previous match",
            Self::Close => "close",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::Refresh => "refresh",
            Self::Copy => "copy",
            Self::OpenSearch => "find",
            Self::SearchNext => "next",
            Self::SearchPrev => "prev",
            Self::Close => "close",
        }
    }
    fn priority(self) -> u8 {
        match self {
            Self::Close => 100,
            Self::OpenSearch => 80,
            _ => 50,
        }
    }
    fn all() -> &'static [Self] {
        &[
            Self::Refresh,
            Self::Copy,
            Self::OpenSearch,
            Self::SearchNext,
            Self::SearchPrev,
            Self::Close,
        ]
    }
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
    OpenDetail,
    Refresh,
    SearchNext,
    SearchPrev,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BrowserVerb {
    Refresh,
    Copy,
    OpenProperties,
    OpenParameterContext,
    OpenActionHistory,
    ShowVersionControl,
}

/// Listing-panel-scoped verbs. Active when focus is inside the
/// connection-detail right pane's flowfile listing (post-Tab from the tree).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BrowserQueueVerb {
    /// Tab — focus the listing rows from tree focus. Reused on Esc to
    /// detect when the active focus state should drop back to the tree.
    FocusListing,
    PeekAttributes,
    TraceLineage,
    CopyUuid,
    Refresh,
    Filter,
    /// Esc — cascades: clears the active filter prompt, then clears
    /// the committed filter, then drops listing focus.
    Cancel,
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
    OpenContentModal,
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
            Self::OpenDetail => Chord::simple(KeyCode::Char('i')),
            Self::Refresh => Chord::simple(KeyCode::Char('r')),
            Self::SearchNext => Chord::simple(KeyCode::Char('n')),
            Self::SearchPrev => Chord::shift(KeyCode::Char('N')),
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
            Self::OpenDetail => "open detail modal",
            Self::Refresh => "refresh",
            Self::SearchNext => "next match",
            Self::SearchPrev => "previous match",
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
            Self::OpenDetail => "info",
            Self::Refresh => "refresh",
            Self::SearchNext => "next",
            Self::SearchPrev => "prev",
        }
    }
    fn priority(self) -> u8 {
        match self {
            Self::OpenSearch | Self::TogglePause => 80,
            Self::ClearFilters => 60,
            _ => 40,
        }
    }
    fn show_in_hint_bar(self) -> bool {
        !matches!(self, Self::ToggleSeverity(_))
    }

    fn enabled(self, ctx: &HintContext<'_>) -> bool {
        match self {
            Self::OpenDetail => !ctx.state.bulletins.ring.is_empty(),
            Self::SearchNext | Self::SearchPrev => ctx
                .state
                .bulletins
                .detail_modal
                .as_ref()
                .and_then(|m| m.search.as_ref())
                .map(|s| s.committed)
                .unwrap_or(false),
            _ => true,
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
            Self::OpenDetail,
            Self::Refresh,
            Self::SearchNext,
            Self::SearchPrev,
        ]
    }
}

impl Verb for BrowserVerb {
    fn chord(self) -> Chord {
        match self {
            Self::Refresh => Chord::simple(KeyCode::Char('r')),
            Self::Copy => Chord::simple(KeyCode::Char('c')),
            Self::OpenProperties => Chord::simple(KeyCode::Char('p')),
            Self::OpenParameterContext => Chord::simple(KeyCode::Char('p')),
            Self::OpenActionHistory => Chord::simple(KeyCode::Char('a')),
            Self::ShowVersionControl => Chord::simple(KeyCode::Char('m')),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Refresh => "refresh flow",
            Self::Copy => "copy id / row value",
            Self::OpenProperties => "open properties",
            Self::OpenParameterContext => "open parameter context",
            Self::OpenActionHistory => "open action history",
            Self::ShowVersionControl => "show version control",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::Refresh => "refresh",
            Self::Copy => "copy",
            Self::OpenProperties => "props",
            Self::OpenParameterContext => "param",
            Self::OpenActionHistory => "actions",
            Self::ShowVersionControl => "version",
        }
    }
    fn enabled(self, ctx: &HintContext<'_>) -> bool {
        use crate::app::state::ViewId;
        match self {
            Self::OpenProperties => {
                ctx.state.current_tab == ViewId::Browser
                    && ctx.state.browser_selection_has_properties()
            }
            Self::OpenParameterContext => {
                ctx.state.current_tab == ViewId::Browser
                    && ctx
                        .state
                        .browser_selection_pg_has_parameter_context_binding()
            }
            Self::OpenActionHistory => {
                ctx.state.current_tab == ViewId::Browser
                    && ctx.state.browser_selection_supports_action_history()
            }
            Self::ShowVersionControl => {
                ctx.state.current_tab == ViewId::Browser
                    && ctx.state.browser_selection_is_versioned_pg()
            }
            _ => true,
        }
    }
    fn priority(self) -> u8 {
        50
    }
    fn all() -> &'static [Self] {
        &[
            Self::Refresh,
            Self::Copy,
            Self::OpenProperties,
            Self::OpenParameterContext,
            Self::OpenActionHistory,
            Self::ShowVersionControl,
        ]
    }
}

impl Verb for BrowserQueueVerb {
    fn chord(self) -> Chord {
        match self {
            Self::FocusListing => Chord::simple(KeyCode::Tab),
            Self::PeekAttributes => Chord::simple(KeyCode::Char('i')),
            Self::TraceLineage => Chord::simple(KeyCode::Char('t')),
            Self::CopyUuid => Chord::simple(KeyCode::Char('c')),
            Self::Refresh => Chord::simple(KeyCode::Char('r')),
            Self::Filter => Chord::simple(KeyCode::Char('/')),
            Self::Cancel => Chord::simple(KeyCode::Esc),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::FocusListing => "focus listing",
            Self::PeekAttributes => "peek attributes",
            Self::TraceLineage => "trace flowfile lineage",
            Self::CopyUuid => "copy flowfile uuid",
            Self::Refresh => "refresh listing",
            Self::Filter => "filter by filename",
            Self::Cancel => "cancel filter / drop listing focus",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::FocusListing => "focus list",
            Self::PeekAttributes => "peek",
            Self::TraceLineage => "trace",
            Self::CopyUuid => "copy",
            Self::Refresh => "refresh",
            Self::Filter => "filter",
            Self::Cancel => "back",
        }
    }
    fn priority(self) -> u8 {
        50
    }
    fn show_in_hint_bar(self) -> bool {
        true
    }
    fn enabled(self, _ctx: &HintContext<'_>) -> bool {
        true
    }
    fn all() -> &'static [Self] {
        &[
            Self::FocusListing,
            Self::PeekAttributes,
            Self::TraceLineage,
            Self::CopyUuid,
            Self::Refresh,
            Self::Filter,
            Self::Cancel,
        ]
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
            Self::OpenContentModal => Chord::simple(KeyCode::Char('i')),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Refresh => "refresh lineage",
            Self::Copy => "copy UUID / attribute value",
            Self::Save => "save content to file",
            Self::ToggleDiff => "toggle attribute diff",
            Self::OpenContentModal => "open content viewer modal",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::Refresh => "refresh",
            Self::Copy => "copy",
            Self::Save => "save",
            Self::ToggleDiff => "diff",
            Self::OpenContentModal => "view",
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
            Self::OpenContentModal => {
                // The handler opens the modal from any sub-tab as long as a
                // lineage event with content is loaded — match that here so
                // the hint bar doesn't gray `i` while it's actually
                // dispatchable. The outer `current_tab != Tracer` guard above
                // already handles the tab gate.
                ctx.state.tracer.content_modal.is_none()
                    && ctx.state.tracer_has_any_side_available()
            }
            _ => true,
        }
    }
    fn priority(self) -> u8 {
        50
    }
    fn all() -> &'static [Self] {
        &[
            Self::Refresh,
            Self::Copy,
            Self::Save,
            Self::ToggleDiff,
            Self::OpenContentModal,
        ]
    }
}

/// Verbs that are only active when the content viewer modal is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContentModalVerb {
    SwitchTabNext,
    SwitchTabPrev,
    JumpInput,
    JumpOutput,
    JumpDiff,
    OpenSearch,
    SearchNext,
    SearchPrev,
    HunkNext,
    HunkPrev,
    Copy,
    Save,
    Close,
}

impl Verb for ContentModalVerb {
    fn chord(self) -> Chord {
        match self {
            Self::SwitchTabNext => Chord::simple(KeyCode::Tab),
            Self::SwitchTabPrev => Chord::simple(KeyCode::BackTab),
            Self::JumpInput => Chord::simple(KeyCode::Char('1')),
            Self::JumpOutput => Chord::simple(KeyCode::Char('2')),
            Self::JumpDiff => Chord::simple(KeyCode::Char('3')),
            Self::OpenSearch => Chord::simple(KeyCode::Char('/')),
            Self::SearchNext => Chord::simple(KeyCode::Char('n')),
            Self::SearchPrev => Chord::shift(KeyCode::Char('N')),
            Self::HunkNext => Chord::ctrl(KeyCode::Down),
            Self::HunkPrev => Chord::ctrl(KeyCode::Up),
            Self::Copy => Chord::simple(KeyCode::Char('c')),
            Self::Save => Chord::simple(KeyCode::Char('s')),
            Self::Close => Chord::simple(KeyCode::Esc),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::SwitchTabNext => "switch tab forward",
            Self::SwitchTabPrev => "switch tab backward",
            Self::JumpInput => "jump to Input tab",
            Self::JumpOutput => "jump to Output tab",
            Self::JumpDiff => "jump to Diff tab",
            Self::OpenSearch => "open text search",
            Self::SearchNext => "next match",
            Self::SearchPrev => "previous match",
            Self::HunkNext => "next change",
            Self::HunkPrev => "previous change",
            Self::Copy => "copy visible body to clipboard",
            Self::Save => "save full content to file",
            Self::Close => "close modal",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::SwitchTabNext => "switch",
            Self::OpenSearch => "find",
            Self::SearchNext => "match",
            Self::HunkNext => "change",
            Self::Copy => "copy",
            Self::Save => "save",
            Self::Close => "close",
            _ => "",
        }
    }
    fn priority(self) -> u8 {
        50
    }
    fn show_in_hint_bar(self) -> bool {
        matches!(
            self,
            Self::SwitchTabNext
                | Self::OpenSearch
                | Self::SearchNext
                | Self::HunkNext
                | Self::Copy
                | Self::Save
                | Self::Close
        )
    }
    fn enabled(self, ctx: &HintContext<'_>) -> bool {
        let Some(modal) = &ctx.state.tracer.content_modal else {
            return false;
        };
        match self {
            Self::JumpDiff => {
                matches!(modal.diffable, crate::view::tracer::state::Diffable::Ok)
            }
            Self::SearchNext | Self::SearchPrev => {
                modal.search.as_ref().map(|s| s.committed).unwrap_or(false)
            }
            Self::HunkNext | Self::HunkPrev => {
                modal.active_tab == crate::view::tracer::state::ContentModalTab::Diff
                    && modal
                        .diff_cache
                        .as_ref()
                        .map(|d| !d.change_stops.is_empty())
                        .unwrap_or(false)
            }
            _ => true,
        }
    }
    fn all() -> &'static [Self] {
        &[
            Self::SwitchTabNext,
            Self::SwitchTabPrev,
            Self::JumpInput,
            Self::JumpOutput,
            Self::JumpDiff,
            Self::OpenSearch,
            Self::SearchNext,
            Self::SearchPrev,
            Self::HunkNext,
            Self::HunkPrev,
            Self::Copy,
            Self::Save,
            Self::Close,
        ]
    }
}

/// Verbs that are only active when the version-control modal is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VersionControlModalVerb {
    Close,
    OpenSearch,
    SearchNext,
    SearchPrev,
    Copy,
    ToggleEnvironmental,
    Refresh,
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    Home,
    End,
}

impl Verb for VersionControlModalVerb {
    fn chord(self) -> Chord {
        match self {
            Self::Close => Chord::simple(KeyCode::Esc),
            Self::OpenSearch => Chord::simple(KeyCode::Char('/')),
            Self::SearchNext => Chord::simple(KeyCode::Char('n')),
            Self::SearchPrev => Chord::shift(KeyCode::Char('N')),
            Self::Copy => Chord::simple(KeyCode::Char('c')),
            Self::ToggleEnvironmental => Chord::simple(KeyCode::Char('e')),
            Self::Refresh => Chord::simple(KeyCode::Char('r')),
            Self::ScrollUp => Chord::simple(KeyCode::Up),
            Self::ScrollDown => Chord::simple(KeyCode::Down),
            Self::PageUp => Chord::simple(KeyCode::PageUp),
            Self::PageDown => Chord::simple(KeyCode::PageDown),
            Self::Home => Chord::simple(KeyCode::Home),
            Self::End => Chord::simple(KeyCode::End),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Close => "close modal",
            Self::OpenSearch => "open text search",
            Self::SearchNext => "next match",
            Self::SearchPrev => "previous match",
            Self::Copy => "copy diff to clipboard",
            Self::ToggleEnvironmental => "toggle environmental differences",
            Self::Refresh => "refresh diff",
            Self::ScrollUp => "scroll up",
            Self::ScrollDown => "scroll down",
            Self::PageUp => "page up",
            Self::PageDown => "page down",
            Self::Home => "scroll to top",
            Self::End => "scroll to bottom",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::Close => "close",
            Self::OpenSearch => "find",
            Self::SearchNext => "match",
            Self::Copy => "copy",
            Self::ToggleEnvironmental => "env",
            Self::Refresh => "refresh",
            _ => "",
        }
    }
    fn priority(self) -> u8 {
        50
    }
    fn show_in_hint_bar(self) -> bool {
        matches!(
            self,
            Self::Close
                | Self::OpenSearch
                | Self::SearchNext
                | Self::Copy
                | Self::ToggleEnvironmental
                | Self::Refresh
        )
    }
    fn enabled(self, _ctx: &HintContext<'_>) -> bool {
        // Modal is the dispatch gate — chords only fire when the
        // keymap is in modal mode (Task 19 wires the gate). Always
        // return true here; dispatch suppresses outside the modal.
        true
    }
    fn all() -> &'static [Self] {
        &[
            Self::Close,
            Self::OpenSearch,
            Self::SearchNext,
            Self::SearchPrev,
            Self::Copy,
            Self::ToggleEnvironmental,
            Self::Refresh,
            Self::ScrollUp,
            Self::ScrollDown,
            Self::PageUp,
            Self::PageDown,
            Self::Home,
            Self::End,
        ]
    }
}

/// Verbs that are only active when the parameter-context modal is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParameterContextModalVerb {
    Close,
    RowUp,
    RowDown,
    PageUp,
    PageDown,
    JumpTop,
    JumpBottom,
    /// `Enter` — when Sidebar focused: shift focus to Body.
    /// When Body focused: no-op (activation not required in v0.1).
    FocusBody,
    ToggleByContext,
    ToggleShadowed,
    ToggleUsedBy,
    Search,
    SearchNext,
    SearchPrev,
    Copy,
    Refresh,
}

impl Verb for ParameterContextModalVerb {
    fn chord(self) -> Chord {
        match self {
            Self::Close => Chord::simple(KeyCode::Esc),
            Self::RowUp => Chord::simple(KeyCode::Up),
            Self::RowDown => Chord::simple(KeyCode::Down),
            Self::PageUp => Chord::simple(KeyCode::PageUp),
            Self::PageDown => Chord::simple(KeyCode::PageDown),
            Self::JumpTop => Chord::simple(KeyCode::Home),
            Self::JumpBottom => Chord::simple(KeyCode::End),
            Self::FocusBody => Chord::simple(KeyCode::Enter),
            Self::ToggleByContext => Chord::simple(KeyCode::Char('t')),
            Self::ToggleShadowed => Chord::simple(KeyCode::Char('s')),
            Self::ToggleUsedBy => Chord::simple(KeyCode::Char('u')),
            Self::Search => Chord::simple(KeyCode::Char('/')),
            Self::SearchNext => Chord::simple(KeyCode::Char('n')),
            Self::SearchPrev => Chord::shift(KeyCode::Char('N')),
            Self::Copy => Chord::simple(KeyCode::Char('c')),
            Self::Refresh => Chord::simple(KeyCode::Char('r')),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Close => "close / unfocus",
            Self::RowUp => "row up",
            Self::RowDown => "row down",
            Self::PageUp => "page up",
            Self::PageDown => "page down",
            Self::JumpTop => "top",
            Self::JumpBottom => "bottom",
            Self::FocusBody => "focus body",
            Self::ToggleByContext => "by context",
            Self::ToggleShadowed => "show shadowed",
            Self::ToggleUsedBy => "used by",
            Self::Search => "search",
            Self::SearchNext => "next match",
            Self::SearchPrev => "prev match",
            Self::Copy => "copy",
            Self::Refresh => "refresh",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::Close => "close",
            Self::RowUp | Self::RowDown => "row",
            Self::PageUp | Self::PageDown => "page",
            Self::JumpTop => "top",
            Self::JumpBottom => "bottom",
            Self::FocusBody => "focus params",
            Self::ToggleByContext => "by-ctx",
            Self::ToggleShadowed => "shadowed",
            Self::ToggleUsedBy => "used-by",
            Self::Search => "search",
            Self::SearchNext => "next",
            Self::SearchPrev => "prev",
            Self::Copy => "copy",
            Self::Refresh => "refresh",
        }
    }

    fn show_in_hint_bar(self) -> bool {
        // Hide natural-navigation chords (arrows, paging, jump, focus)
        // and search-cycling chords from the footer strip — they're
        // intuitive and crowd out the meaningful verbs (search, copy,
        // refresh, toggles, close).
        !matches!(
            self,
            Self::RowUp
                | Self::RowDown
                | Self::PageUp
                | Self::PageDown
                | Self::JumpTop
                | Self::JumpBottom
                | Self::FocusBody
                | Self::SearchNext
                | Self::SearchPrev
        )
    }

    fn enabled(self, _ctx: &HintContext<'_>) -> bool {
        // Modal is the dispatch gate — chords only fire when the
        // keymap is in modal mode. Always return true here; dispatch
        // suppresses outside the modal.
        true
    }

    fn all() -> &'static [Self] {
        &[
            Self::Close,
            Self::RowUp,
            Self::RowDown,
            Self::PageUp,
            Self::PageDown,
            Self::JumpTop,
            Self::JumpBottom,
            Self::FocusBody,
            Self::ToggleByContext,
            Self::ToggleShadowed,
            Self::ToggleUsedBy,
            Self::Search,
            Self::SearchNext,
            Self::SearchPrev,
            Self::Copy,
            Self::Refresh,
        ]
    }
}

/// Verbs that are only active when the action-history modal is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActionHistoryModalVerb {
    Close,
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    JumpTop,
    JumpBottom,
    /// Enter — toggle inline expansion of the selected row.
    ToggleExpand,
    OpenSearch,
    SearchNext,
    SearchPrev,
    Copy,
    Refresh,
}

impl Verb for ActionHistoryModalVerb {
    fn chord(self) -> Chord {
        match self {
            Self::Close => Chord::simple(KeyCode::Esc),
            Self::ScrollUp => Chord::simple(KeyCode::Up),
            Self::ScrollDown => Chord::simple(KeyCode::Down),
            Self::PageUp => Chord::simple(KeyCode::PageUp),
            Self::PageDown => Chord::simple(KeyCode::PageDown),
            Self::JumpTop => Chord::simple(KeyCode::Home),
            Self::JumpBottom => Chord::simple(KeyCode::End),
            Self::ToggleExpand => Chord::simple(KeyCode::Enter),
            Self::OpenSearch => Chord::simple(KeyCode::Char('/')),
            Self::SearchNext => Chord::simple(KeyCode::Char('n')),
            Self::SearchPrev => Chord::shift(KeyCode::Char('N')),
            Self::Copy => Chord::simple(KeyCode::Char('c')),
            Self::Refresh => Chord::simple(KeyCode::Char('r')),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Close => "close modal",
            Self::ScrollUp => "scroll up",
            Self::ScrollDown => "scroll down",
            Self::PageUp => "page up",
            Self::PageDown => "page down",
            Self::JumpTop => "scroll to top",
            Self::JumpBottom => "scroll to bottom",
            Self::ToggleExpand => "expand / collapse selected action",
            Self::OpenSearch => "open text search",
            Self::SearchNext => "next match",
            Self::SearchPrev => "previous match",
            Self::Copy => "copy selected row as TSV",
            Self::Refresh => "refresh from offset 0",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::Close => "close",
            Self::ToggleExpand => "expand",
            Self::OpenSearch => "find",
            Self::SearchNext => "next",
            Self::SearchPrev => "prev",
            Self::Copy => "copy",
            Self::Refresh => "refresh",
            _ => "",
        }
    }
    fn priority(self) -> u8 {
        50
    }
    fn show_in_hint_bar(self) -> bool {
        // Hide arrow/page/jump nav and search-cycling chords; show
        // the meaningful verbs.
        matches!(
            self,
            Self::Close | Self::OpenSearch | Self::Copy | Self::Refresh | Self::ToggleExpand
        )
    }
    fn enabled(self, _ctx: &HintContext<'_>) -> bool {
        // Modal is the dispatch gate — chords only fire when the
        // keymap is in modal mode (Task 8 wires the gate).
        true
    }
    fn all() -> &'static [Self] {
        &[
            Self::Close,
            Self::ScrollUp,
            Self::ScrollDown,
            Self::PageUp,
            Self::PageDown,
            Self::JumpTop,
            Self::JumpBottom,
            Self::ToggleExpand,
            Self::OpenSearch,
            Self::SearchNext,
            Self::SearchPrev,
            Self::Copy,
            Self::Refresh,
        ]
    }
}

/// Per-flowfile peek modal-scoped verbs. Shadows outer-tab keys via
/// the keymap shadow gate while the modal is open (Task 9 wires the gate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BrowserPeekVerb {
    /// Esc — cascades: closes search prompt first, then closes the modal.
    Close,
    OpenSearch,
    SearchNext,
    SearchPrev,
    /// `c` — copy the loaded attributes table as pretty-printed JSON.
    CopyAsJson,
}

impl Verb for BrowserPeekVerb {
    fn chord(self) -> Chord {
        match self {
            Self::Close => Chord::simple(KeyCode::Esc),
            Self::OpenSearch => Chord::simple(KeyCode::Char('/')),
            Self::SearchNext => Chord::simple(KeyCode::Char('n')),
            Self::SearchPrev => Chord::shift(KeyCode::Char('N')),
            Self::CopyAsJson => Chord::simple(KeyCode::Char('c')),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Close => "close peek modal",
            Self::OpenSearch => "open text search",
            Self::SearchNext => "next match",
            Self::SearchPrev => "previous match",
            Self::CopyAsJson => "copy attributes as JSON",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::Close => "close",
            Self::OpenSearch => "find",
            Self::SearchNext => "next",
            Self::SearchPrev => "prev",
            Self::CopyAsJson => "copy json",
        }
    }
    fn priority(self) -> u8 {
        50
    }
    fn show_in_hint_bar(self) -> bool {
        // Show all five — the modal hint strip has room.
        true
    }
    fn enabled(self, _ctx: &HintContext<'_>) -> bool {
        // Modal is the dispatch gate — chords only fire when the
        // keymap is in peek-modal mode (Task 9 wires the gate).
        true
    }
    fn all() -> &'static [Self] {
        &[
            Self::Close,
            Self::OpenSearch,
            Self::SearchNext,
            Self::SearchPrev,
            Self::CopyAsJson,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewVerb {
    Bulletins(BulletinsVerb),
    Browser(BrowserVerb),
    BrowserQueue(BrowserQueueVerb),
    BrowserPeek(BrowserPeekVerb),
    Events(EventsVerb),
    Tracer(TracerVerb),
    ContentModal(ContentModalVerb),
    VersionControlModal(VersionControlModalVerb),
    ParameterContextModal(ParameterContextModalVerb),
    ActionHistoryModal(ActionHistoryModalVerb),
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
            .chain(TracerVerb::all().iter().map(|v| v.chord()))
            .chain(ContentModalVerb::all().iter().map(|v| v.chord()))
            .chain(VersionControlModalVerb::all().iter().map(|v| v.chord()))
            .chain(ParameterContextModalVerb::all().iter().map(|v| v.chord()))
            .chain(ActionHistoryModalVerb::all().iter().map(|v| v.chord()));
        for c in chords {
            assert_ne!(c.key, KeyCode::Char('j'), "no view verb may bind j");
            assert_ne!(c.key, KeyCode::Char('k'), "no view verb may bind k");
        }
    }

    #[test]
    fn raise_cap_label_matches_constants() {
        use crate::view::events::state::{DEFAULT_RESULT_CAP, EXPANDED_RESULT_CAP};
        let label = EventsVerb::RaiseCap.label();
        let default_s = DEFAULT_RESULT_CAP.to_string();
        let expanded_s = EXPANDED_RESULT_CAP.to_string();
        assert!(
            label.contains(&default_s),
            "RaiseCap label {label:?} does not mention DEFAULT_RESULT_CAP={default_s}; \
             update the label or the constant",
        );
        assert!(
            label.contains(&expanded_s),
            "RaiseCap label {label:?} does not mention EXPANDED_RESULT_CAP={expanded_s}; \
             update the label or the constant",
        );
    }

    #[test]
    fn show_version_control_chord_is_m() {
        use crate::input::Verb;
        let chord = BrowserVerb::ShowVersionControl.chord();
        assert_eq!(chord, Chord::simple(KeyCode::Char('m')));
    }

    #[test]
    fn show_version_control_label_and_hint() {
        use crate::input::Verb;
        assert_eq!(
            BrowserVerb::ShowVersionControl.label(),
            "show version control"
        );
        assert_eq!(BrowserVerb::ShowVersionControl.hint(), "version");
    }

    #[test]
    fn show_version_control_in_all() {
        use crate::input::Verb;
        assert!(BrowserVerb::all().contains(&BrowserVerb::ShowVersionControl));
    }

    #[test]
    fn version_control_modal_verb_chords() {
        use crate::input::Verb;
        use VersionControlModalVerb as V;
        assert_eq!(V::Close.chord(), Chord::simple(KeyCode::Esc));
        assert_eq!(V::OpenSearch.chord(), Chord::simple(KeyCode::Char('/')));
        assert_eq!(V::SearchNext.chord(), Chord::simple(KeyCode::Char('n')));
        assert_eq!(V::SearchPrev.chord(), Chord::shift(KeyCode::Char('N')));
        assert_eq!(V::Copy.chord(), Chord::simple(KeyCode::Char('c')));
        assert_eq!(
            V::ToggleEnvironmental.chord(),
            Chord::simple(KeyCode::Char('e'))
        );
        assert_eq!(V::Refresh.chord(), Chord::simple(KeyCode::Char('r')));
        assert_eq!(V::ScrollUp.chord(), Chord::simple(KeyCode::Up));
        assert_eq!(V::ScrollDown.chord(), Chord::simple(KeyCode::Down));
        assert_eq!(V::PageUp.chord(), Chord::simple(KeyCode::PageUp));
        assert_eq!(V::PageDown.chord(), Chord::simple(KeyCode::PageDown));
        assert_eq!(V::Home.chord(), Chord::simple(KeyCode::Home));
        assert_eq!(V::End.chord(), Chord::simple(KeyCode::End));
    }

    #[test]
    fn version_control_modal_verb_in_all() {
        use crate::input::Verb;
        let all = VersionControlModalVerb::all();
        assert!(all.contains(&VersionControlModalVerb::Close));
        assert_eq!(all.len(), 13);
    }

    #[test]
    fn no_view_verb_binds_j_or_k_includes_version_control_modal() {
        let chords = VersionControlModalVerb::all().iter().map(|v| v.chord());
        for c in chords {
            assert_ne!(c.key, KeyCode::Char('j'));
            assert_ne!(c.key, KeyCode::Char('k'));
        }
    }

    #[test]
    fn open_parameter_context_chord_is_p() {
        let chord = BrowserVerb::OpenParameterContext.chord();
        assert_eq!(chord.display(), "p");
    }

    #[test]
    fn open_parameter_context_label_and_hint() {
        assert_eq!(
            BrowserVerb::OpenParameterContext.label(),
            "open parameter context"
        );
        assert_eq!(BrowserVerb::OpenParameterContext.hint(), "param");
    }

    #[test]
    fn open_parameter_context_in_all() {
        assert!(BrowserVerb::all().contains(&BrowserVerb::OpenParameterContext));
    }

    #[test]
    fn open_action_history_chord_is_a() {
        assert_eq!(
            BrowserVerb::OpenActionHistory.chord(),
            Chord::simple(KeyCode::Char('a'))
        );
        assert_eq!(
            BrowserVerb::OpenActionHistory.label(),
            "open action history"
        );
        assert_eq!(BrowserVerb::OpenActionHistory.hint(), "actions");
        assert!(BrowserVerb::all().contains(&BrowserVerb::OpenActionHistory));
    }

    #[test]
    fn open_parameter_context_enabled_only_on_pg_rows_with_binding() {
        use crate::app::state::ViewId;
        use crate::client::{NodeKind, NodeStatusSummary, RawNode, RecursiveSnapshot};
        use crate::cluster::snapshot::ParameterContextRef;
        use crate::input::{HintContext, Verb};
        use crate::test_support::fresh_state;
        use crate::view::browser::state::apply_tree_snapshot;
        use std::time::SystemTime;

        // Build a seeded tree: root PG (idx 0) + one Processor child (idx 1).
        let make_snap = || RecursiveSnapshot {
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
            ],
            fetched_at: SystemTime::now(),
        };

        // PG row with a bound context: enabled.
        let mut pg_state = fresh_state();
        apply_tree_snapshot(&mut pg_state.browser, make_snap());
        pg_state.browser.nodes[0].parameter_context_ref = Some(ParameterContextRef {
            id: "ctx-x".into(),
            name: "ctx-x".into(),
        });
        pg_state.current_tab = ViewId::Browser;
        pg_state.browser.selected = 0;
        let ctx = HintContext::new(&pg_state);
        assert!(
            BrowserVerb::OpenParameterContext.enabled(&ctx),
            "OpenParameterContext must be enabled on a PG row with a binding"
        );

        // PG row without a binding: disabled.
        let mut pg_unbound = fresh_state();
        apply_tree_snapshot(&mut pg_unbound.browser, make_snap());
        // nodes[0] has no parameter_context_ref by default.
        pg_unbound.current_tab = ViewId::Browser;
        pg_unbound.browser.selected = 0;
        let ctx = HintContext::new(&pg_unbound);
        assert!(
            !BrowserVerb::OpenParameterContext.enabled(&ctx),
            "OpenParameterContext must be disabled on a PG row without a binding"
        );

        // Processor row (visible row 1 is 'Gen'): disabled.
        let mut proc_state = fresh_state();
        apply_tree_snapshot(&mut proc_state.browser, make_snap());
        proc_state.current_tab = ViewId::Browser;
        proc_state.browser.selected = 1;
        let ctx = HintContext::new(&proc_state);
        assert!(
            !BrowserVerb::OpenParameterContext.enabled(&ctx),
            "OpenParameterContext must be disabled on a Processor row"
        );

        // Off-tab: disabled even on a PG row with a binding.
        let off_tab = fresh_state(); // current_tab is Overview by default
        let ctx = HintContext::new(&off_tab);
        assert!(
            !BrowserVerb::OpenParameterContext.enabled(&ctx),
            "OpenParameterContext must be disabled when not on the Browser tab"
        );
    }

    #[test]
    fn parameter_context_modal_verb_chords() {
        use ParameterContextModalVerb as V;
        assert_eq!(V::Close.chord().display(), "Esc");
        assert_eq!(V::ToggleByContext.chord().display(), "t");
        assert_eq!(V::ToggleShadowed.chord().display(), "s");
        assert_eq!(V::ToggleUsedBy.chord().display(), "u");
        assert_eq!(V::Search.chord().display(), "/");
        assert_eq!(V::Refresh.chord().display(), "r");
        assert_eq!(V::Copy.chord().display(), "c");
    }

    #[test]
    fn parameter_context_modal_verb_in_all() {
        let all = ParameterContextModalVerb::all();
        assert!(all.contains(&ParameterContextModalVerb::Close));
        assert!(all.contains(&ParameterContextModalVerb::ToggleByContext));
        assert!(all.contains(&ParameterContextModalVerb::ToggleUsedBy));
    }

    #[test]
    fn parameter_context_modal_search_next_prev_hidden_from_hint_bar() {
        assert!(!ParameterContextModalVerb::SearchNext.show_in_hint_bar());
        assert!(!ParameterContextModalVerb::SearchPrev.show_in_hint_bar());
    }

    #[test]
    fn open_properties_disabled_on_pg_rows_so_p_only_resolves_to_one_verb() {
        use crate::app::state::ViewId;
        use crate::client::{NodeKind, NodeStatusSummary, RawNode, RecursiveSnapshot};
        use crate::input::{HintContext, Verb};
        use crate::test_support::fresh_state;
        use crate::view::browser::state::apply_tree_snapshot;
        use std::time::SystemTime;

        let mut pg_state = fresh_state();
        apply_tree_snapshot(
            &mut pg_state.browser,
            RecursiveSnapshot {
                nodes: vec![RawNode {
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
                }],
                fetched_at: SystemTime::now(),
            },
        );
        pg_state.current_tab = ViewId::Browser;
        pg_state.browser.selected = 0;
        let ctx = HintContext::new(&pg_state);
        assert!(
            !BrowserVerb::OpenProperties.enabled(&ctx),
            "OpenProperties must be disabled on a PG row (chord-collision invariant)"
        );
    }

    #[test]
    fn action_history_modal_verb_chords_complete() {
        use crate::input::ActionHistoryModalVerb as V;
        use crate::input::Verb;
        // Required chords.
        assert_eq!(V::Close.chord(), Chord::simple(KeyCode::Esc));
        assert_eq!(V::OpenSearch.chord(), Chord::simple(KeyCode::Char('/')));
        assert_eq!(V::SearchNext.chord(), Chord::simple(KeyCode::Char('n')));
        assert_eq!(V::SearchPrev.chord(), Chord::shift(KeyCode::Char('N')));
        assert_eq!(V::Copy.chord(), Chord::simple(KeyCode::Char('c')));
        assert_eq!(V::Refresh.chord(), Chord::simple(KeyCode::Char('r')));
        assert_eq!(V::ToggleExpand.chord(), Chord::simple(KeyCode::Enter));

        // Hint-bar visibility: hide nav + search-cycling chords.
        assert!(!V::ScrollUp.show_in_hint_bar());
        assert!(!V::ScrollDown.show_in_hint_bar());
        assert!(!V::SearchNext.show_in_hint_bar());
        assert!(!V::SearchPrev.show_in_hint_bar());
        assert!(V::Close.show_in_hint_bar());
        assert!(V::OpenSearch.show_in_hint_bar());
        assert!(V::Copy.show_in_hint_bar());
        assert!(V::Refresh.show_in_hint_bar());

        // all() coverage.
        let all = V::all();
        for v in [
            V::Close,
            V::OpenSearch,
            V::SearchNext,
            V::SearchPrev,
            V::Copy,
            V::Refresh,
            V::ToggleExpand,
        ] {
            assert!(all.contains(&v), "missing {v:?}");
        }
    }

    #[test]
    fn browser_queue_verb_all_returns_seven_verbs() {
        let all = BrowserQueueVerb::all();
        assert_eq!(all.len(), 7);
    }

    #[test]
    fn browser_queue_verb_chord_set_matches_spec() {
        let chords: Vec<Chord> = BrowserQueueVerb::all()
            .iter()
            .copied()
            .map(Verb::chord)
            .collect();
        assert!(chords.contains(&Chord::simple(KeyCode::Tab)));
        assert!(chords.contains(&Chord::simple(KeyCode::Char('i'))));
        assert!(chords.contains(&Chord::simple(KeyCode::Char('t'))));
        assert!(chords.contains(&Chord::simple(KeyCode::Char('c'))));
        assert!(chords.contains(&Chord::simple(KeyCode::Char('r'))));
        assert!(chords.contains(&Chord::simple(KeyCode::Char('/'))));
        assert!(chords.contains(&Chord::simple(KeyCode::Esc)));
    }

    #[test]
    fn browser_peek_verb_chord_set_matches_spec() {
        let chords: Vec<Chord> = BrowserPeekVerb::all()
            .iter()
            .copied()
            .map(Verb::chord)
            .collect();
        assert!(chords.contains(&Chord::simple(KeyCode::Esc)));
        assert!(chords.contains(&Chord::simple(KeyCode::Char('/'))));
        assert!(chords.contains(&Chord::simple(KeyCode::Char('n'))));
        assert!(chords.contains(&Chord::shift(KeyCode::Char('N'))));
        assert!(chords.contains(&Chord::simple(KeyCode::Char('c'))));
    }

    #[test]
    fn common_verb_chords_match_documented_bindings() {
        use crossterm::event::KeyCode;
        assert_eq!(
            CommonVerb::Refresh.chord(),
            Chord::simple(KeyCode::Char('r'))
        );
        assert_eq!(CommonVerb::Copy.chord(), Chord::simple(KeyCode::Char('c')));
        assert_eq!(
            CommonVerb::OpenSearch.chord(),
            Chord::simple(KeyCode::Char('/'))
        );
        assert_eq!(
            CommonVerb::SearchNext.chord(),
            Chord::simple(KeyCode::Char('n'))
        );
        assert_eq!(
            CommonVerb::SearchPrev.chord(),
            Chord::shift(KeyCode::Char('N'))
        );
        assert_eq!(CommonVerb::Close.chord(), Chord::simple(KeyCode::Esc));
    }

    #[test]
    fn common_verb_all_lists_every_variant() {
        let all = CommonVerb::all();
        assert_eq!(
            all.len(),
            6,
            "if you add a CommonVerb variant, list it in all()"
        );
    }

    #[test]
    fn common_verb_close_has_higher_priority_than_search() {
        // Close (Esc) is a core escape verb (priority 100). OpenSearch is
        // promoted to 80 because it's frequently used. The rest default to 50.
        assert!(CommonVerb::Close.priority() > CommonVerb::OpenSearch.priority());
        assert!(CommonVerb::OpenSearch.priority() > CommonVerb::Refresh.priority());
    }

    #[test]
    fn common_verb_show_in_hint_bar_default() {
        // CommonVerb has no hint-bar exclusions today.
        for &v in CommonVerb::all() {
            assert!(v.show_in_hint_bar(), "{v:?} should appear in hint bar");
        }
    }
}
