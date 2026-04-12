//! Pure state reducer for the Health tab.

use crate::app::navigation::ListNavigation;
use crate::client::health::{
    self, FullPgStatusSnapshot, NodesState, ProcessorThreadState, QueuePressureState,
    RepositoryState, SystemDiagSnapshot, TOP_N, build_per_node_for_row,
};

/// All mutable state for the Health tab.
#[derive(Debug, Default)]
pub struct HealthState {
    pub selected_category: HealthCategory,
    pub queues: QueuePressureState,
    pub repositories: RepositoryState,
    pub processors: ProcessorThreadState,
    pub nodes: NodesState,
    pub last_pg_refresh: Option<std::time::Instant>,
    pub last_sysdiag_refresh: Option<std::time::Instant>,
}

impl HealthState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Which detail category is shown on the right pane.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HealthCategory {
    #[default]
    Queues,
    Repositories,
    Nodes,
    Processors,
}

impl HealthCategory {
    /// Advance to the next category, wrapping around.
    pub fn next(self) -> Self {
        match self {
            Self::Queues => Self::Repositories,
            Self::Repositories => Self::Nodes,
            Self::Nodes => Self::Processors,
            Self::Processors => Self::Queues,
        }
    }

    /// Retreat to the previous category, wrapping around.
    pub fn prev(self) -> Self {
        match self {
            Self::Queues => Self::Processors,
            Self::Repositories => Self::Queues,
            Self::Nodes => Self::Repositories,
            Self::Processors => Self::Nodes,
        }
    }

    /// Map a 1-based index to a category (`1` = Queues … `4` = Processors).
    pub fn from_index(i: u8) -> Option<Self> {
        match i {
            1 => Some(Self::Queues),
            2 => Some(Self::Repositories),
            3 => Some(Self::Nodes),
            4 => Some(Self::Processors),
            _ => None,
        }
    }
}

/// Fold a [`FullPgStatusSnapshot`] into the queue-pressure and
/// processor-thread sub-states.
pub fn apply_pg_status(state: &mut HealthState, snapshot: FullPgStatusSnapshot) {
    state.queues.rows = health::compute_queue_pressure(&snapshot, TOP_N);
    state.queues.selected = state
        .queues
        .selected
        .min(state.queues.rows.len().saturating_sub(1));
    state.processors.rows = health::compute_processor_threads(&snapshot, TOP_N);
    state.processors.selected = state
        .processors
        .selected
        .min(state.processors.rows.len().saturating_sub(1));
    state.last_pg_refresh = Some(snapshot.fetched_at);
}

/// Fold a [`SystemDiagSnapshot`] into the repository and node sub-states.
pub fn apply_system_diagnostics(state: &mut HealthState, diag: SystemDiagSnapshot) {
    // Preserve the previous selection index (clamped later).
    let prev_selected = state.repositories.selected;
    state.repositories = health::extract_repositories(&diag);
    // Restore selection, clamped to the new row count.
    if !state.repositories.rows.is_empty() {
        state.repositories.selected = prev_selected.min(state.repositories.rows.len() - 1);
        // Rebuild per_node for the restored selection.
        state.repositories.per_node = build_per_node_for_row(
            &state.repositories.rows[state.repositories.selected],
            &diag.nodes,
        );
    }
    // Stash node diagnostics for later per_node rebuilds on selection change.
    state.repositories.node_diagnostics = diag.nodes.clone();
    health::update_nodes(&mut state.nodes, &diag);
    state.last_sysdiag_refresh = Some(diag.fetched_at);
}

impl ListNavigation for RepositoryState {
    fn list_len(&self) -> usize {
        self.rows.len()
    }

    fn selected(&self) -> Option<usize> {
        if self.rows.is_empty() {
            None
        } else {
            Some(self.selected)
        }
    }

    fn set_selected(&mut self, index: Option<usize>) {
        self.selected = index.unwrap_or(0);
    }

    fn wraps(&self) -> bool {
        true
    }
}

impl ListNavigation for QueuePressureState {
    fn list_len(&self) -> usize {
        self.rows.len()
    }

    fn selected(&self) -> Option<usize> {
        if self.rows.is_empty() {
            None
        } else {
            Some(self.selected)
        }
    }

    fn set_selected(&mut self, index: Option<usize>) {
        self.selected = index.unwrap_or(0);
    }

    fn wraps(&self) -> bool {
        true
    }
}

impl ListNavigation for NodesState {
    fn list_len(&self) -> usize {
        self.nodes.len()
    }

    fn selected(&self) -> Option<usize> {
        if self.nodes.is_empty() {
            None
        } else {
            Some(self.selected)
        }
    }

    fn set_selected(&mut self, index: Option<usize>) {
        self.selected = index.unwrap_or(0);
    }

    fn wraps(&self) -> bool {
        true
    }
}

impl ListNavigation for ProcessorThreadState {
    fn list_len(&self) -> usize {
        self.rows.len()
    }

    fn selected(&self) -> Option<usize> {
        if self.rows.is_empty() {
            None
        } else {
            Some(self.selected)
        }
    }

    fn set_selected(&mut self, index: Option<usize>) {
        self.selected = index.unwrap_or(0);
    }

    fn wraps(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_selects_queues() {
        let state = HealthState::new();
        assert_eq!(state.selected_category, HealthCategory::Queues);
    }

    #[test]
    fn category_cycles_correctly() {
        // forward
        assert_eq!(HealthCategory::Queues.next(), HealthCategory::Repositories);
        assert_eq!(HealthCategory::Repositories.next(), HealthCategory::Nodes);
        assert_eq!(HealthCategory::Nodes.next(), HealthCategory::Processors);
        assert_eq!(HealthCategory::Processors.next(), HealthCategory::Queues);

        // backward
        assert_eq!(HealthCategory::Queues.prev(), HealthCategory::Processors);
        assert_eq!(HealthCategory::Processors.prev(), HealthCategory::Nodes);
        assert_eq!(HealthCategory::Nodes.prev(), HealthCategory::Repositories);
        assert_eq!(HealthCategory::Repositories.prev(), HealthCategory::Queues);
    }

    #[test]
    fn category_from_index() {
        assert_eq!(HealthCategory::from_index(1), Some(HealthCategory::Queues));
        assert_eq!(
            HealthCategory::from_index(2),
            Some(HealthCategory::Repositories)
        );
        assert_eq!(HealthCategory::from_index(3), Some(HealthCategory::Nodes));
        assert_eq!(
            HealthCategory::from_index(4),
            Some(HealthCategory::Processors)
        );
        assert_eq!(HealthCategory::from_index(0), None);
        assert_eq!(HealthCategory::from_index(5), None);
    }

    #[test]
    fn repository_state_list_navigation_wraps() {
        use crate::client::health::{RepoKind, RepoRow, RepositoryState, Severity};

        let mut repos = RepositoryState {
            rows: vec![
                RepoRow {
                    kind: RepoKind::Content,
                    identifier: "a".into(),
                    fill_percent: 50,
                    severity: Severity::Green,
                },
                RepoRow {
                    kind: RepoKind::FlowFile,
                    identifier: "b".into(),
                    fill_percent: 30,
                    severity: Severity::Green,
                },
            ],
            ..RepositoryState::default()
        };
        assert_eq!(repos.selected(), Some(0));

        repos.move_down();
        assert_eq!(repos.selected(), Some(1));

        // Wrap around.
        repos.move_down();
        assert_eq!(repos.selected(), Some(0));

        // Wrap backwards.
        repos.move_up();
        assert_eq!(repos.selected(), Some(1));
    }
}
