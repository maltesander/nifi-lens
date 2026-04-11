//! Color and style constants. No runtime theming in Phase 0.

use ratatui::style::{Color, Modifier, Style};

pub fn muted() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub fn accent() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

pub fn error() -> Style {
    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
}

pub fn warning() -> Style {
    Style::default().fg(Color::Yellow)
}

pub fn info() -> Style {
    Style::default().fg(Color::Blue)
}

pub fn cursor_row() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}
