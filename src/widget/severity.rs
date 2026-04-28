//! Consolidated severity label + style helpers.
//!
//! Three render leaves (`view::browser::render::pg`,
//! `view::browser::render::processor`, `view::bulletins::render`)
//! previously carried identical copies of these two helpers; this
//! module consolidates them.

use ratatui::style::Style;

use crate::client::Severity;

/// Uppercase severity label for the count-rendered chip.
pub fn format_severity_label(level: &str) -> String {
    match Severity::parse(level) {
        Severity::Error => "ERROR".to_string(),
        Severity::Warning => "WARN ".to_string(),
        Severity::Info => "INFO ".to_string(),
        Severity::Unknown => level.to_ascii_uppercase(),
    }
}

/// [`Style`] for a severity label — color only, no modifiers.
pub fn severity_style(level: &str) -> Style {
    Severity::parse(level).style()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_stable() {
        assert_eq!(format_severity_label("ERROR"), "ERROR");
        assert_eq!(format_severity_label("WARN"), "WARN ");
        assert_eq!(format_severity_label("INFO"), "INFO ");
        assert_eq!(format_severity_label("bogus"), "BOGUS");
    }

    #[test]
    fn styles_are_stable() {
        let e = severity_style("ERROR");
        let w = severity_style("WARN");
        let i = severity_style("INFO");
        let u = severity_style("bogus");
        assert_ne!(e, w);
        assert_ne!(e, i);
        assert_ne!(e, u);
        assert_ne!(w, i);
        assert_ne!(w, u);
        assert_ne!(i, u);
    }
}
