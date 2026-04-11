use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Paragraph;

use crate::app::state::{AppState, BannerSeverity};
use crate::theme;

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let seconds = state.last_refresh.elapsed().as_secs();
    let middle = if let Some(banner) = &state.status.banner {
        banner.message.clone()
    } else {
        format!("last refresh {seconds}s ago")
    };

    let text = format!(
        "[{}] NiFi {} · {} · ? for help",
        state.context_name, state.detected_version, middle
    );

    let style = match state.status.banner.as_ref().map(|b| b.severity) {
        Some(BannerSeverity::Error) => theme::error(),
        Some(BannerSeverity::Warning) => theme::warning(),
        Some(BannerSeverity::Info) => theme::info(),
        None => theme::muted(),
    };

    frame.render_widget(Paragraph::new(text).style(style), area);
}
