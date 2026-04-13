//! Cross-link tab history with selection restore.

use crate::app::state::ViewId;

const MAX_HISTORY: usize = 20;

/// Anchor describing the selection state to restore when returning to a tab.
#[derive(Debug, Clone)]
pub enum SelectionAnchor {
    /// Browser: node's component_id.
    ComponentId(String),
    /// Bulletins: best-effort row index.
    RowIndex(usize),
}

/// A single entry in the navigation history.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub tab: ViewId,
    pub anchor: Option<SelectionAnchor>,
}

/// Browser-style back/forward navigation history for cross-link jumps.
#[derive(Debug, Default)]
pub struct TabHistory {
    back: Vec<HistoryEntry>,
    forward: Vec<HistoryEntry>,
}

impl TabHistory {
    /// Push a new entry onto the back stack, clearing the forward stack.
    ///
    /// If the back stack exceeds `MAX_HISTORY`, the oldest entry is removed.
    pub fn push(&mut self, entry: HistoryEntry) {
        self.forward.clear();
        self.back.push(entry);
        if self.back.len() > MAX_HISTORY {
            self.back.remove(0);
        }
    }

    /// Pop the most recent entry from the back stack, pushing `current` onto forward.
    ///
    /// Returns `None` if the back stack is empty.
    pub fn pop_back(&mut self, current: HistoryEntry) -> Option<HistoryEntry> {
        let entry = self.back.pop()?;
        self.forward.push(current);
        if self.forward.len() > MAX_HISTORY {
            self.forward.remove(0);
        }
        Some(entry)
    }

    /// Pop the most recent entry from the forward stack, pushing `current` onto back.
    ///
    /// Returns `None` if the forward stack is empty.
    pub fn pop_forward(&mut self, current: HistoryEntry) -> Option<HistoryEntry> {
        let entry = self.forward.pop()?;
        self.back.push(current);
        if self.back.len() > MAX_HISTORY {
            self.back.remove(0);
        }
        Some(entry)
    }

    /// Returns `true` if there is at least one entry in the back stack.
    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    /// Returns `true` if there is at least one entry in the forward stack.
    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_history_cannot_go_back() {
        let history = TabHistory::default();
        assert!(!history.can_go_back());
        assert!(!history.can_go_forward());
    }

    #[test]
    fn push_and_pop_back() {
        let mut history = TabHistory::default();
        history.push(HistoryEntry {
            tab: ViewId::Bulletins,
            anchor: None,
        });
        assert!(history.can_go_back());

        let current = HistoryEntry {
            tab: ViewId::Browser,
            anchor: None,
        };
        let popped = history.pop_back(current).unwrap();
        assert!(matches!(popped.tab, ViewId::Bulletins));
        assert!(!history.can_go_back());
        assert!(history.can_go_forward());
    }

    #[test]
    fn pop_back_pushes_current_onto_forward() {
        let mut history = TabHistory::default();
        history.push(HistoryEntry {
            tab: ViewId::Bulletins,
            anchor: None,
        });

        let current = HistoryEntry {
            tab: ViewId::Browser,
            anchor: None,
        };
        let _popped = history.pop_back(current).unwrap();

        // Now forward should have Browser (the current we passed in)
        let current2 = HistoryEntry {
            tab: ViewId::Bulletins,
            anchor: None,
        };
        let forward = history.pop_forward(current2).unwrap();
        assert!(matches!(forward.tab, ViewId::Browser));
    }

    #[test]
    fn push_clears_forward() {
        let mut history = TabHistory::default();
        history.push(HistoryEntry {
            tab: ViewId::Bulletins,
            anchor: None,
        });

        let current = HistoryEntry {
            tab: ViewId::Browser,
            anchor: None,
        };
        let _popped = history.pop_back(current).unwrap();
        assert!(history.can_go_forward());

        // Push again should clear forward
        history.push(HistoryEntry {
            tab: ViewId::Browser,
            anchor: None,
        });
        assert!(!history.can_go_forward());
    }

    #[test]
    fn cap_at_max_entries() {
        let mut history = TabHistory::default();
        for i in 0..25 {
            history.push(HistoryEntry {
                tab: if i % 2 == 0 {
                    ViewId::Bulletins
                } else {
                    ViewId::Browser
                },
                anchor: Some(SelectionAnchor::RowIndex(i)),
            });
        }

        // Only MAX_HISTORY (20) entries should remain
        let mut count = 0;
        loop {
            let current = HistoryEntry {
                tab: ViewId::Overview,
                anchor: None,
            };
            if history.pop_back(current).is_none() {
                break;
            }
            count += 1;
        }
        assert_eq!(count, MAX_HISTORY);
    }

    #[test]
    fn anchor_preserved_through_round_trip() {
        let mut history = TabHistory::default();
        history.push(HistoryEntry {
            tab: ViewId::Browser,
            anchor: Some(SelectionAnchor::ComponentId("abc-123".to_string())),
        });

        let current = HistoryEntry {
            tab: ViewId::Tracer,
            anchor: None,
        };
        let popped = history.pop_back(current).unwrap();
        assert!(matches!(popped.tab, ViewId::Browser));
        match popped.anchor {
            Some(SelectionAnchor::ComponentId(ref id)) => assert_eq!(id, "abc-123"),
            _ => panic!("Expected ComponentId anchor"),
        }
    }
}
