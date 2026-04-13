//! The single bordered-box style for every section in nifi-lens.
//!
//! Every render leaf that draws a titled chrome box routes through
//! `Panel`. Centralising the style (border type, border color, title
//! placement) means theme tuning touches one file.

use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType};

use crate::theme;

pub struct Panel<'a> {
    title: Line<'a>,
    right_title: Option<Line<'a>>,
    focused: bool,
}

impl<'a> Panel<'a> {
    pub fn new(title: impl Into<Line<'a>>) -> Self {
        Self {
            title: title.into(),
            right_title: None,
            focused: false,
        }
    }

    pub fn right(mut self, content: impl Into<Line<'a>>) -> Self {
        self.right_title = Some(content.into());
        self
    }

    pub fn focused(mut self, yes: bool) -> Self {
        self.focused = yes;
        self
    }

    pub fn into_block(self) -> Block<'a> {
        let border_type = if self.focused {
            BorderType::Thick
        } else {
            BorderType::Plain
        };
        let border_style = if self.focused {
            theme::accent()
        } else {
            theme::border_dim()
        };

        let mut block = Block::bordered()
            .border_type(border_type)
            .border_style(border_style)
            .title(self.title);
        if let Some(right) = self.right_title {
            block = block.title_top(right.right_aligned());
        }
        block
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;

    fn draw(title: &str, focused: bool, right: Option<&str>) -> String {
        let mut term = Terminal::new(TestBackend::new(32, 5)).unwrap();
        term.draw(|f| {
            let mut p = Panel::new(title.to_string());
            if let Some(r) = right {
                p = p.right(r.to_string());
            }
            p = p.focused(focused);
            let block = p.into_block();
            let area = Rect {
                x: 0,
                y: 0,
                width: 32,
                height: 5,
            };
            f.render_widget(block, area);
        })
        .unwrap();
        format!("{}", term.backend())
    }

    #[test]
    fn unfocused_plain_borders() {
        assert_snapshot!("panel_unfocused", draw(" Nodes ", false, None));
    }

    #[test]
    fn focused_thick_borders() {
        assert_snapshot!("panel_focused", draw(" Properties ", true, None));
    }

    #[test]
    fn right_title_renders() {
        assert_snapshot!(
            "panel_with_right_title",
            draw(" Properties ", false, Some(" 10/14 "))
        );
    }
}
