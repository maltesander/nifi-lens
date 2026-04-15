//! Jump-menu modal (full implementation in Task 11).
use crate::input::GoTarget;

#[derive(Debug, Clone)]
pub struct JumpMenuState {
    pub targets: Vec<GoTarget>,
    pub selected: usize,
}

impl JumpMenuState {
    pub fn new(targets: Vec<GoTarget>) -> Self {
        Self {
            targets,
            selected: 0,
        }
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.targets.len() {
            self.selected += 1;
        }
    }

    pub fn selected_target(&self) -> Option<GoTarget> {
        self.targets.get(self.selected).copied()
    }
}
