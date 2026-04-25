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
    /// Mirrors `BulletinsState`'s use of `modal.details.raw_message` —
    /// `widget::search::compute_matches` expects a `&str`. Excludes
    /// environmental diffs when `show_environmental == false`.
    pub fn searchable_body(&self) -> String {
        let mut out = String::new();
        if let VersionControlDifferenceLoad::Loaded(sections) = &self.differences {
            for s in sections {
                out.push_str(&format!(
                    "{} · {} · {}\n",
                    s.component_type, s.component_name, s.component_id
                ));
                for d in &s.differences {
                    if !self.show_environmental && d.environmental {
                        continue;
                    }
                    out.push_str(&format!("{}: {}\n", d.kind, d.description));
                }
                out.push('\n');
            }
        }
        out
    }
}
