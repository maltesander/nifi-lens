//! Timestamp parsing and formatting shared across Bulletins and Tracer.
//!
//! NiFi emits timestamps in two wire formats:
//!   - ISO-8601:  "2026-04-12T14:32:18.123Z"
//!   - NiFi human: "04/12/2026 14:32:18 UTC"
//!
//! This module parses both into [`time::OffsetDateTime`] and renders them
//! according to the user's `[ui]` config.

use serde::Deserialize;
use time::OffsetDateTime;

/// Preset timestamp display format, chosen via `[ui] timestamp_format`.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TimestampFormat {
    /// `HH:MM:SS` for events from today, `MMM DD HH:MM:SS` for older events.
    #[default]
    Short,
    /// `2026-04-12T14:32:18Z` (UTC) or `...+02:00` (local).
    Iso,
    /// `Apr 12 14:32:18`.
    Human,
}

/// Time zone preference, chosen via `[ui] timestamp_tz`.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TimestampTz {
    #[default]
    Utc,
    Local,
}

/// Resolved timestamp rendering config passed into format call sites.
#[derive(Debug, Clone, Copy, Default)]
pub struct TimestampConfig {
    pub format: TimestampFormat,
    pub tz: TimestampTz,
}

/// Parse NiFi's wire timestamp formats into an `OffsetDateTime` (UTC).
///
/// Returns `None` for any input that does not match one of the two
/// expected shapes — callers should fall back to rendering the raw
/// string.
pub fn parse_nifi_timestamp(raw: &str) -> Option<OffsetDateTime> {
    // Implemented in Task 3.
    let _ = raw;
    None
}

/// Format `dt` for display. `now` is used only by the `Short` preset
/// to decide "today vs older"; render call sites pass `OffsetDateTime::now_utc()`.
/// `with_ms` forces millisecond precision regardless of preset —
/// Tracer passes `true` for event detail rows.
pub fn format(
    dt: OffsetDateTime,
    now: OffsetDateTime,
    cfg: &TimestampConfig,
    with_ms: bool,
) -> String {
    // Implemented in Task 4.
    let _ = (dt, now, cfg, with_ms);
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_returns_none_on_garbage() {
        assert!(parse_nifi_timestamp("not a timestamp").is_none());
    }
}
