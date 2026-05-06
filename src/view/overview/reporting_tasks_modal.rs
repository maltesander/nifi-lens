//! Reporting Tasks modal — state, selection persistence, search.
//!
//! Layout / rendering live alongside but are added in Task 14+.

use crate::client::{ReportingTaskRow, ReportingTasksSnapshot};
use crate::widget::scroll::VerticalScrollState;
use crate::widget::search::SearchState;

/// Per-section row-cursor array size — matches the number of focusable
/// detail sections (Properties, ValidationErrors, RecentBulletins).
pub const MAX_DETAIL_SECTIONS: usize = 3;

/// Detail-pane focus model. `List` is the default; `Detail` carries an
/// index into `section_list(task)` and per-section row cursors.
///
/// Indexed by **position in the filtered section list**, not by
/// `DetailSection` value — when validation errors disappear, `rows[1]`
/// shifts meaning from "validation row" to "bulletin row". Mirrors
/// browser's `DetailFocus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModalFocus {
    #[default]
    List,
    Detail {
        idx: usize,
        rows: [usize; MAX_DETAIL_SECTIONS],
    },
}

/// Focusable detail-pane sub-section. Identity is non-focusable and
/// not in this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailSection {
    Properties,
    ValidationErrors,
    RecentBulletins,
}

#[derive(Debug, Default)]
pub struct ReportingTasksModalState {
    pub selected_id: Option<String>,
    /// Ordinal position within `filtered_indices`. Tracked alongside
    /// `selected_id` so that `reconcile_selection` can fall back to the
    /// same row position when the previously-selected id disappears.
    pub selected_ordinal: usize,
    pub list_scroll: VerticalScrollState,
    pub detail_scroll: VerticalScrollState,
    pub focus: ModalFocus,
    pub search: SearchState,
    pub filtered_indices: Vec<usize>,
}

impl ReportingTasksModalState {
    /// Constructs an open modal with cursor on the first row.
    pub fn open(snapshot: &ReportingTasksSnapshot) -> Self {
        let mut state = Self::default();
        state.refilter(snapshot);
        let first = state
            .filtered_indices
            .first()
            .and_then(|&i| snapshot.tasks.get(i));
        state.selected_id = first.map(|t| t.id.clone());
        state.selected_ordinal = 0;
        state
    }

    /// Re-applies `selected_id` against a possibly-mutated snapshot.
    /// Sticky-by-id; on disappearance falls back to the row at the same
    /// ordinal in the new filtered view, clamped to the new length.
    pub fn reconcile_selection(&mut self, snapshot: &ReportingTasksSnapshot) {
        // Remember the current ordinal before refiltering, so we can fall
        // back to "same position" when the id disappears.
        let prev_ordinal = self.selected_ordinal;

        self.refilter(snapshot);

        // If the selected id still exists, keep it and update the ordinal.
        if let Some((ord, _)) = self.selected_id.as_ref().and_then(|id| {
            self.filtered_indices
                .iter()
                .enumerate()
                .find(|&(_, &fi)| snapshot.tasks.get(fi).map(|t| &t.id) == Some(id))
        }) {
            self.selected_ordinal = ord;
            return;
        }

        // Id disappeared — fall back to same ordinal, clamped.
        if self.filtered_indices.is_empty() {
            self.selected_id = None;
            self.selected_ordinal = 0;
            return;
        }
        let new_ord = prev_ordinal.min(self.filtered_indices.len().saturating_sub(1));
        let raw_idx = self.filtered_indices[new_ord];
        self.selected_id = snapshot.tasks.get(raw_idx).map(|t| t.id.clone());
        self.selected_ordinal = new_ord;
    }

    /// Re-derive `filtered_indices` from the current search query against
    /// the supplied snapshot. Empty query matches everything.
    pub fn refilter(&mut self, snapshot: &ReportingTasksSnapshot) {
        let query = self.search.query.to_lowercase();
        self.filtered_indices = snapshot
            .tasks
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                if query.is_empty() {
                    return true;
                }
                let hay = format!(
                    "{} {} {:?}",
                    t.name.to_lowercase(),
                    short_type(&t.task_type).to_lowercase(),
                    t.state
                );
                hay.contains(&query)
            })
            .map(|(i, _)| i)
            .collect();
    }

    /// Look up the currently-selected `ReportingTaskRow` in the supplied
    /// snapshot. Returns `None` if the selection has been cleared (e.g.
    /// snapshot is empty) or has not yet been resolved post-reconcile.
    pub fn selected_row<'a>(
        &self,
        snapshot: &'a ReportingTasksSnapshot,
    ) -> Option<&'a ReportingTaskRow> {
        let id = self.selected_id.as_ref()?;
        snapshot.tasks.iter().find(|t| &t.id == id)
    }
}

/// Detects `#{name}` references in a NiFi property value (no import needed
/// from the render module — inlined here so the state module stays
/// renderer-independent).
pub fn contains_param_ref_raw(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' && i + 1 < bytes.len() && bytes[i + 1] == b'#' {
            // Escape: ##{...} — skip past the closing brace if present.
            if i + 2 < bytes.len()
                && bytes[i + 2] == b'{'
                && let Some(close) = bytes[i + 3..].iter().position(|&b| b == b'}')
            {
                i += 3 + close + 1;
                continue;
            }
            i += 2;
            continue;
        }
        if bytes[i] == b'#'
            && i + 1 < bytes.len()
            && bytes[i + 1] == b'{'
            && bytes[i + 2..].contains(&b'}')
        {
            return true;
        }
        i += 1;
    }
    false
}

/// Last `.`-separated segment of a fully-qualified class name. Used by
/// the modal list pane and search haystack.
pub fn short_type(fqcn: &str) -> &str {
    fqcn.rsplit('.').next().unwrap_or(fqcn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{ReportingTaskState, ValidationStatus};
    use std::collections::BTreeMap;
    use std::time::Instant;

    fn snap(ids: &[&str]) -> ReportingTasksSnapshot {
        ReportingTasksSnapshot {
            tasks: ids
                .iter()
                .map(|&id| ReportingTaskRow {
                    id: id.into(),
                    name: format!("name-{id}"),
                    task_type: "org.x.Y".into(),
                    state: ReportingTaskState::Running,
                    scheduling_strategy: "TIMER_DRIVEN".into(),
                    scheduling_period: "30s".into(),
                    active_thread_count: 0,
                    validation_status: ValidationStatus::Valid,
                    validation_errors: vec![],
                    comments: None,
                    properties: BTreeMap::new(),
                    descriptors: BTreeMap::new(),
                })
                .collect(),
            fetched_at: Instant::now(),
        }
    }

    #[test]
    fn open_selects_first_row() {
        let s = snap(&["a", "b", "c"]);
        let m = ReportingTasksModalState::open(&s);
        assert_eq!(m.selected_id.as_deref(), Some("a"));
    }

    #[test]
    fn open_starts_in_list_focus() {
        let s = snap(&["a"]);
        let m = ReportingTasksModalState::open(&s);
        assert_eq!(m.focus, ModalFocus::List);
    }

    #[test]
    fn detail_focus_carries_zeroed_row_array() {
        let focus = ModalFocus::Detail {
            idx: 0,
            rows: [0; 3],
        };
        if let ModalFocus::Detail { idx, rows } = focus {
            assert_eq!(idx, 0);
            assert_eq!(rows, [0, 0, 0]);
        } else {
            panic!("expected Detail variant");
        }
    }

    #[test]
    fn reconcile_keeps_selected_when_present() {
        let s1 = snap(&["a", "b", "c"]);
        let mut m = ReportingTasksModalState::open(&s1);
        m.selected_id = Some("b".into());
        m.selected_ordinal = 1;
        let s2 = snap(&["a", "b", "c", "d"]);
        m.reconcile_selection(&s2);
        assert_eq!(m.selected_id.as_deref(), Some("b"));
    }

    #[test]
    fn reconcile_falls_back_to_same_index_when_missing() {
        let s1 = snap(&["a", "b", "c"]);
        let mut m = ReportingTasksModalState::open(&s1);
        m.selected_id = Some("b".into()); // ordinal 1
        m.selected_ordinal = 1;
        let s2 = snap(&["a", "x", "c"]); // "b" gone, ordinal 1 is "x"
        m.reconcile_selection(&s2);
        assert_eq!(m.selected_id.as_deref(), Some("x"));
    }

    #[test]
    fn reconcile_clamps_to_last_row_on_shrink() {
        let s1 = snap(&["a", "b", "c", "d"]);
        let mut m = ReportingTasksModalState::open(&s1);
        m.selected_id = Some("d".into()); // ordinal 3
        m.selected_ordinal = 3;
        let s2 = snap(&["a", "b"]); // shrunk to 2 rows
        m.reconcile_selection(&s2);
        assert_eq!(m.selected_id.as_deref(), Some("b"));
    }

    #[test]
    fn reconcile_handles_empty() {
        let mut m = ReportingTasksModalState::open(&snap(&["a"]));
        m.reconcile_selection(&snap(&[]));
        assert_eq!(m.selected_id, None);
    }

    #[test]
    fn short_type_picks_last_segment() {
        assert_eq!(short_type("org.apache.nifi.foo.Bar"), "Bar");
        assert_eq!(short_type("noDot"), "noDot");
    }

    #[test]
    fn search_query_narrows_filtered_indices() {
        use crate::client::{ReportingTaskState, ValidationStatus};
        let s = ReportingTasksSnapshot {
            tasks: vec![
                ReportingTaskRow {
                    id: "1".into(),
                    name: "Prometheus exporter".into(),
                    task_type: "org.x.PrometheusReportingTask".into(),
                    state: ReportingTaskState::Running,
                    scheduling_strategy: "TIMER_DRIVEN".into(),
                    scheduling_period: "30s".into(),
                    active_thread_count: 0,
                    validation_status: ValidationStatus::Valid,
                    validation_errors: vec![],
                    comments: None,
                    properties: BTreeMap::new(),
                    descriptors: BTreeMap::new(),
                },
                ReportingTaskRow {
                    id: "2".into(),
                    name: "Disk monitor".into(),
                    task_type: "org.x.MonitorDiskUsage".into(),
                    state: ReportingTaskState::Stopped,
                    scheduling_strategy: "TIMER_DRIVEN".into(),
                    scheduling_period: "1m".into(),
                    active_thread_count: 0,
                    validation_status: ValidationStatus::Valid,
                    validation_errors: vec![],
                    comments: None,
                    properties: BTreeMap::new(),
                    descriptors: BTreeMap::new(),
                },
            ],
            fetched_at: Instant::now(),
        };
        let mut m = ReportingTasksModalState::open(&s);
        m.search.query = "disk".to_string();
        m.refilter(&s);
        assert_eq!(m.filtered_indices, vec![1]);

        m.search.query = "nonexistent".to_string();
        m.refilter(&s);
        assert!(m.filtered_indices.is_empty());

        m.search.query = String::new();
        m.refilter(&s);
        assert_eq!(m.filtered_indices.len(), 2);
    }
}
