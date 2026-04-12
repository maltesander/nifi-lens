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
    use time::format_description::well_known::Iso8601;
    use time::macros::format_description;

    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    // Try RFC-3339 / ISO-8601 first (covers the `T...Z` shape with or without ms).
    if let Ok(dt) = OffsetDateTime::parse(raw, &Iso8601::DEFAULT) {
        return Some(dt);
    }

    // NiFi human format: `MM/DD/YYYY HH:MM:SS[.mmm] UTC`.
    // The trailing `UTC` tag is how NiFi indicates the offset; treat it as +00:00.
    let (ts_part, tz_part) = raw.rsplit_once(' ')?;
    if tz_part != "UTC" {
        return None;
    }

    let with_ms =
        format_description!("[month]/[day]/[year] [hour]:[minute]:[second].[subsecond digits:3]");
    let without_ms = format_description!("[month]/[day]/[year] [hour]:[minute]:[second]");

    let primitive = time::PrimitiveDateTime::parse(ts_part, with_ms)
        .or_else(|_| time::PrimitiveDateTime::parse(ts_part, without_ms))
        .ok()?;
    Some(primitive.assume_utc())
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
    use time::macros::datetime;

    #[test]
    fn parse_returns_none_on_garbage() {
        assert!(parse_nifi_timestamp("not a timestamp").is_none());
    }

    #[test]
    fn parse_returns_none_on_empty() {
        assert!(parse_nifi_timestamp("").is_none());
    }

    #[test]
    fn parse_iso_8601_with_millis() {
        let got = parse_nifi_timestamp("2026-04-12T14:32:18.123Z").unwrap();
        assert_eq!(got, datetime!(2026-04-12 14:32:18.123 UTC));
    }

    #[test]
    fn parse_iso_8601_without_millis() {
        let got = parse_nifi_timestamp("2026-04-12T14:32:18Z").unwrap();
        assert_eq!(got, datetime!(2026-04-12 14:32:18 UTC));
    }

    #[test]
    fn parse_nifi_human_format() {
        let got = parse_nifi_timestamp("04/12/2026 14:32:18 UTC").unwrap();
        assert_eq!(got, datetime!(2026-04-12 14:32:18 UTC));
    }

    #[test]
    fn parse_nifi_human_format_with_millis() {
        let got = parse_nifi_timestamp("04/12/2026 14:32:18.456 UTC").unwrap();
        assert_eq!(got, datetime!(2026-04-12 14:32:18.456 UTC));
    }
}
