//! Version-control modal state. Captured at open time from the cluster
//! snapshot for identity, then asynchronously populated with diff data
//! by the view-local worker (Task 19).

use crate::client::ComponentDiffSection;
use crate::cluster::snapshot::VersionControlSummary;
use crate::widget::scroll::BidirectionalScrollState;
use crate::widget::search::SearchState;

#[derive(Debug, Clone)]
pub struct VersionControlModalState {
    pub pg_id: String,
    pub pg_name: String,
    pub identity: Option<VersionControlSummary>,
    pub differences: VersionControlDifferenceLoad,
    pub show_environmental: bool,
    pub scroll: BidirectionalScrollState,
    /// `Option` matches the Bulletins detail modal pattern — `None`
    /// means "no search active"; presence with `input_active = true`
    /// means "user is typing the query"; presence with
    /// `committed = true` means "n/N cycle through matches".
    pub search: Option<SearchState>,
}

#[derive(Debug, Clone)]
pub enum VersionControlDifferenceLoad {
    Pending,
    Loaded(Vec<ComponentDiffSection>),
    Failed(String),
}

impl VersionControlModalState {
    pub fn pending(
        pg_id: String,
        pg_name: String,
        identity: Option<VersionControlSummary>,
    ) -> Self {
        Self {
            pg_id,
            pg_name,
            identity,
            differences: VersionControlDifferenceLoad::Pending,
            show_environmental: false,
            scroll: BidirectionalScrollState::default(),
            search: None,
        }
    }

    /// Build the flat string body the search primitives operate on.
    /// Output format MUST match `view::browser::render::version_control_modal`'s
    /// rendered diff body line-for-line so `MatchSpan` byte offsets align.
    /// Excludes environmental diffs when `show_environmental == false`.
    /// Sections whose remaining diffs are zero are collapsed entirely —
    /// this mirrors the renderer's `visible` filter.
    pub fn searchable_body(&self) -> String {
        let mut out = String::new();
        if let VersionControlDifferenceLoad::Loaded(sections) = &self.differences {
            let visible: Vec<_> = sections
                .iter()
                .filter_map(|s| {
                    let kept: Vec<_> = s
                        .differences
                        .iter()
                        .filter(|d| self.show_environmental || !d.environmental)
                        .collect();
                    if kept.is_empty() {
                        None
                    } else {
                        Some((s, kept))
                    }
                })
                .collect();
            for (section, diffs) in &visible {
                out.push_str(&format!(
                    "─ {} · {} · {} ─\n",
                    section.component_type,
                    section.component_name,
                    short_id(&section.component_id)
                ));
                for d in diffs {
                    out.push_str(&format!("{:<18} {}\n", d.kind, d.description));
                }
                out.push('\n');
            }
        }
        out
    }
}

/// Truncate an id to first 4 hex chars + `…`. Shared between this
/// state-side searchable body and the render-side header so the
/// search-match byte offsets line up.
pub(crate) fn short_id(id: &str) -> String {
    if id.len() <= 4 {
        id.to_string()
    } else {
        format!("{}…", &id[..4])
    }
}
