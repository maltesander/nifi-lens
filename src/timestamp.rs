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
use time::format_description::well_known::Iso8601;
use time::macros::format_description;

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
///
/// Leading and trailing whitespace is trimmed before parsing; inner
/// whitespace is preserved as-is.
pub fn parse_nifi_timestamp(raw: &str) -> Option<OffsetDateTime> {
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
///
/// Preset output shapes:
/// - `Short`: `HH:MM:SS[.mmm]` when `dt` falls on today (per `now` and `cfg.tz`),
///   `Mon DD HH:MM:SS[.mmm]` otherwise.
/// - `Iso`: `YYYY-MM-DDTHH:MM:SS[.mmm]Z` (UTC) or with numeric offset (Local).
/// - `Human`: always `Mon DD HH:MM:SS[.mmm]`.
pub fn format(
    dt: OffsetDateTime,
    now: OffsetDateTime,
    cfg: &TimestampConfig,
    with_ms: bool,
) -> String {
    // Re-project both `dt` and `now` into the configured tz so that
    // date comparisons, hour displays, and offsets all agree.
    let offset = match cfg.tz {
        TimestampTz::Utc => time::UtcOffset::UTC,
        TimestampTz::Local => {
            time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC)
        }
    };
    let dt_local = dt.to_offset(offset);
    let now_local = now.to_offset(offset);

    match cfg.format {
        TimestampFormat::Short => {
            if dt_local.date() == now_local.date() {
                if with_ms {
                    let desc = format_description!("[hour]:[minute]:[second].[subsecond digits:3]");
                    dt_local.format(desc).unwrap_or_default()
                } else {
                    let desc = format_description!("[hour]:[minute]:[second]");
                    dt_local.format(desc).unwrap_or_default()
                }
            } else if with_ms {
                let desc = format_description!(
                    "[month repr:short] [day padding:zero] [hour]:[minute]:[second].[subsecond digits:3]"
                );
                dt_local.format(desc).unwrap_or_default()
            } else {
                let desc = format_description!(
                    "[month repr:short] [day padding:zero] [hour]:[minute]:[second]"
                );
                dt_local.format(desc).unwrap_or_default()
            }
        }
        TimestampFormat::Iso => {
            if with_ms {
                let desc = format_description!(
                    "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3][offset_hour sign:mandatory]:[offset_minute]"
                );
                let mut out = dt_local.format(desc).unwrap_or_default();
                // Collapse "+00:00" to "Z" for UTC to match conventional ISO output.
                if matches!(cfg.tz, TimestampTz::Utc) && out.ends_with("+00:00") {
                    out.truncate(out.len() - 6); // strip the trailing "+00:00" (6 bytes)
                    out.push('Z');
                }
                out
            } else {
                let desc = format_description!(
                    "[year]-[month]-[day]T[hour]:[minute]:[second][offset_hour sign:mandatory]:[offset_minute]"
                );
                let mut out = dt_local.format(desc).unwrap_or_default();
                if matches!(cfg.tz, TimestampTz::Utc) && out.ends_with("+00:00") {
                    out.truncate(out.len() - 6); // strip the trailing "+00:00" (6 bytes)
                    out.push('Z');
                }
                out
            }
        }
        TimestampFormat::Human => {
            if with_ms {
                let desc = format_description!(
                    "[month repr:short] [day padding:zero] [hour]:[minute]:[second].[subsecond digits:3]"
                );
                dt_local.format(desc).unwrap_or_default()
            } else {
                let desc = format_description!(
                    "[month repr:short] [day padding:zero] [hour]:[minute]:[second]"
                );
                dt_local.format(desc).unwrap_or_default()
            }
        }
    }
}

/// Compact human-readable age. `< 60s → "Ns"`, `< 60m → "Nm"`,
/// otherwise `"Nh"`. `None` → em-dash.
pub fn format_age(d: Option<std::time::Duration>) -> String {
    let Some(d) = d else {
        return "\u{2014}".to_string();
    };
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
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

    #[test]
    fn parse_trims_whitespace_around_valid_input() {
        let got = parse_nifi_timestamp("  2026-04-12T14:32:18Z  ").unwrap();
        assert_eq!(got, datetime!(2026-04-12 14:32:18 UTC));
    }

    fn utc_cfg(fmt: TimestampFormat) -> TimestampConfig {
        TimestampConfig {
            format: fmt,
            tz: TimestampTz::Utc,
        }
    }

    #[test]
    fn format_short_same_day() {
        let dt = datetime!(2026-04-12 14:32:18 UTC);
        let now = datetime!(2026-04-12 20:00:00 UTC);
        assert_eq!(
            format(dt, now, &utc_cfg(TimestampFormat::Short), false),
            "14:32:18"
        );
    }

    #[test]
    fn format_short_different_day() {
        let dt = datetime!(2026-04-11 14:32:18 UTC);
        let now = datetime!(2026-04-12 20:00:00 UTC);
        assert_eq!(
            format(dt, now, &utc_cfg(TimestampFormat::Short), false),
            "Apr 11 14:32:18"
        );
    }

    #[test]
    fn format_short_with_ms() {
        let dt = datetime!(2026-04-12 14:32:18.456 UTC);
        let now = datetime!(2026-04-12 20:00:00 UTC);
        assert_eq!(
            format(dt, now, &utc_cfg(TimestampFormat::Short), true),
            "14:32:18.456"
        );
    }

    #[test]
    fn format_iso_utc() {
        let dt = datetime!(2026-04-12 14:32:18 UTC);
        let now = datetime!(2026-04-12 20:00:00 UTC);
        assert_eq!(
            format(dt, now, &utc_cfg(TimestampFormat::Iso), false),
            "2026-04-12T14:32:18Z"
        );
    }

    #[test]
    fn format_iso_with_ms() {
        let dt = datetime!(2026-04-12 14:32:18.456 UTC);
        let now = datetime!(2026-04-12 20:00:00 UTC);
        assert_eq!(
            format(dt, now, &utc_cfg(TimestampFormat::Iso), true),
            "2026-04-12T14:32:18.456Z"
        );
    }

    #[test]
    fn format_human() {
        let dt = datetime!(2026-04-12 14:32:18 UTC);
        let now = datetime!(2026-04-12 20:00:00 UTC);
        assert_eq!(
            format(dt, now, &utc_cfg(TimestampFormat::Human), false),
            "Apr 12 14:32:18"
        );
    }

    #[test]
    fn format_human_with_ms() {
        let dt = datetime!(2026-04-12 14:32:18.456 UTC);
        let now = datetime!(2026-04-12 20:00:00 UTC);
        assert_eq!(
            format(dt, now, &utc_cfg(TimestampFormat::Human), true),
            "Apr 12 14:32:18.456"
        );
    }

    #[test]
    fn format_age_under_minute() {
        assert_eq!(format_age(Some(std::time::Duration::from_secs(0))), "0s");
        assert_eq!(format_age(Some(std::time::Duration::from_secs(3))), "3s");
        assert_eq!(format_age(Some(std::time::Duration::from_secs(59))), "59s");
    }

    #[test]
    fn format_age_minutes() {
        assert_eq!(format_age(Some(std::time::Duration::from_secs(60))), "1m");
        assert_eq!(format_age(Some(std::time::Duration::from_secs(125))), "2m");
        assert_eq!(
            format_age(Some(std::time::Duration::from_secs(3599))),
            "59m"
        );
    }

    #[test]
    fn format_age_hours() {
        assert_eq!(format_age(Some(std::time::Duration::from_secs(3600))), "1h");
        assert_eq!(format_age(Some(std::time::Duration::from_secs(7200))), "2h");
        assert_eq!(
            format_age(Some(std::time::Duration::from_secs(86_400))),
            "24h"
        );
    }

    #[test]
    fn format_age_none() {
        assert_eq!(format_age(None), "\u{2014}"); // em-dash
    }
}
