//! Six-column role/status badge rendered before each node row in the
//! Overview Nodes panel and inside the detail modal header.
//!
//! Format: `"[XY]"` (4 visible chars + 2 side spaces) so adjacent rows
//! stay vertically aligned regardless of which roles a node holds.

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use crate::client::health::{ClusterMembership, ClusterNodeStatus};
use crate::theme;

/// Render a role/status badge for the given membership. Span has a
/// fixed visible width of 6 columns so rows align regardless of
/// content.
pub fn node_badge(cluster: &ClusterMembership) -> Span<'static> {
    let (text, style) =
        badge_text_and_style(cluster.status, cluster.is_primary, cluster.is_coordinator);
    Span::styled(format!(" {text} "), style)
}

fn badge_text_and_style(status: ClusterNodeStatus, primary: bool, coord: bool) -> (String, Style) {
    match status {
        ClusterNodeStatus::Connected => {
            let p = if primary { 'P' } else { '\u{00b7}' }; // middle dot
            let c = if coord { 'C' } else { '\u{00b7}' };
            let style = if primary || coord {
                theme::accent().add_modifier(Modifier::BOLD)
            } else {
                theme::muted()
            };
            (format!("[{p}{c}]"), style)
        }
        ClusterNodeStatus::Connecting | ClusterNodeStatus::Disconnecting => {
            ("[CON]".to_string(), theme::warning())
        }
        ClusterNodeStatus::Offloading | ClusterNodeStatus::Offloaded => {
            ("[OFF]".to_string(), theme::warning())
        }
        ClusterNodeStatus::Disconnected => ("[DIS]".to_string(), theme::error()),
        ClusterNodeStatus::Other => ("[?] ".to_string(), theme::muted()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(status: ClusterNodeStatus, primary: bool, coord: bool) -> ClusterMembership {
        ClusterMembership {
            node_id: String::new(),
            status,
            is_primary: primary,
            is_coordinator: coord,
            heartbeat_age: None,
            node_start_iso: None,
            active_thread_count: 0,
            flow_files_queued: 0,
            bytes_queued: 0,
            events: vec![],
        }
    }

    #[test]
    fn connected_primary_plus_coord() {
        let (t, _) = badge_text_and_style(ClusterNodeStatus::Connected, true, true);
        assert_eq!(t, "[PC]");
    }

    #[test]
    fn connected_primary_only() {
        let (t, _) = badge_text_and_style(ClusterNodeStatus::Connected, true, false);
        assert_eq!(t, "[P\u{00b7}]");
    }

    #[test]
    fn connected_coord_only() {
        let (t, _) = badge_text_and_style(ClusterNodeStatus::Connected, false, true);
        assert_eq!(t, "[\u{00b7}C]");
    }

    #[test]
    fn connected_neither_role() {
        let (t, _) = badge_text_and_style(ClusterNodeStatus::Connected, false, false);
        assert_eq!(t, "[\u{00b7}\u{00b7}]");
    }

    #[test]
    fn offloaded_is_off() {
        let (t, _) = badge_text_and_style(ClusterNodeStatus::Offloaded, false, false);
        assert_eq!(t, "[OFF]");
    }

    #[test]
    fn offloading_is_off() {
        let (t, _) = badge_text_and_style(ClusterNodeStatus::Offloading, false, false);
        assert_eq!(t, "[OFF]");
    }

    #[test]
    fn disconnected_is_dis() {
        let (t, _) = badge_text_and_style(ClusterNodeStatus::Disconnected, false, false);
        assert_eq!(t, "[DIS]");
    }

    #[test]
    fn connecting_is_con() {
        let (t, _) = badge_text_and_style(ClusterNodeStatus::Connecting, false, false);
        assert_eq!(t, "[CON]");
    }

    #[test]
    fn disconnecting_is_con() {
        let (t, _) = badge_text_and_style(ClusterNodeStatus::Disconnecting, false, false);
        assert_eq!(t, "[CON]");
    }

    #[test]
    fn other_is_question_mark() {
        let (t, _) = badge_text_and_style(ClusterNodeStatus::Other, false, false);
        assert_eq!(t, "[?] ");
    }

    #[test]
    fn node_badge_wraps_with_spaces_for_alignment() {
        let span = node_badge(&m(ClusterNodeStatus::Connected, true, false));
        assert_eq!(span.content, " [P\u{00b7}] ");
    }
}
