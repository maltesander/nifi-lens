//! Out-of-band warnings for `[polling]` values in `config.toml`.
//!
//! Recommended ranges:
//!
//! | Knob                    | min   | max    |
//! |-------------------------|-------|--------|
//! | `overview.pg_status`    | 5s    | 5 min  |
//! | `overview.sysdiag`      | 10s   | 10 min |
//! | `browser.interval`      | 5s    | 5 min  |
//! | `bulletins.interval`    | 1s    | 5 min  |

use std::time::Duration;

use super::PollingConfig;

/// Collect out-of-band warnings for every knob in `cfg`. Pure; does
/// not touch any global state. Production code calls
/// `warn_if_out_of_band`, which wraps this and emits each warning
/// through `tracing::warn!`.
pub(crate) fn collect_warnings(cfg: &PollingConfig) -> Vec<String> {
    let mut out = Vec::new();
    check(
        &mut out,
        "polling.overview.pg_status",
        cfg.overview.pg_status,
        Duration::from_secs(5),
        Duration::from_secs(5 * 60),
    );
    check(
        &mut out,
        "polling.overview.sysdiag",
        cfg.overview.sysdiag,
        Duration::from_secs(10),
        Duration::from_secs(10 * 60),
    );
    check(
        &mut out,
        "polling.browser.interval",
        cfg.browser.interval,
        Duration::from_secs(5),
        Duration::from_secs(5 * 60),
    );
    check(
        &mut out,
        "polling.bulletins.interval",
        cfg.bulletins.interval,
        Duration::from_secs(1),
        Duration::from_secs(5 * 60),
    );
    out
}

/// Log one `tracing::warn!` per out-of-band value. Called from the
/// config loader after successful deserialization.
pub(crate) fn warn_if_out_of_band(cfg: &PollingConfig) {
    for msg in collect_warnings(cfg) {
        tracing::warn!("{msg}");
    }
}

fn check(out: &mut Vec<String>, name: &str, value: Duration, min: Duration, max: Duration) {
    if value < min {
        out.push(format!(
            "{name} = {value:?} is below recommended minimum {min:?}; \
             the cluster may be hammered"
        ));
    } else if value > max {
        out.push(format!(
            "{name} = {value:?} is above recommended maximum {max:?}; \
             the UI may feel stale"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PollingConfig;
    use std::time::Duration;

    #[test]
    fn collect_warnings_quiet_for_defaults() {
        let cfg = PollingConfig::default();
        let warnings = collect_warnings(&cfg);
        assert!(
            warnings.is_empty(),
            "defaults must emit zero warnings, got: {warnings:?}",
        );
    }

    #[test]
    fn collect_warnings_fires_below_min_for_each_knob() {
        let mut cfg = PollingConfig::default();

        cfg.overview.pg_status = Duration::from_secs(1);
        let w = collect_warnings(&cfg);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("polling.overview.pg_status"));
        assert!(w[0].contains("below"));
        cfg.overview.pg_status = Duration::from_secs(10);

        cfg.overview.sysdiag = Duration::from_secs(5);
        let w = collect_warnings(&cfg);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("polling.overview.sysdiag"));
        cfg.overview.sysdiag = Duration::from_secs(30);

        cfg.browser.interval = Duration::from_secs(2);
        let w = collect_warnings(&cfg);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("polling.browser.interval"));
        cfg.browser.interval = Duration::from_secs(15);

        cfg.bulletins.interval = Duration::from_millis(500);
        let w = collect_warnings(&cfg);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("polling.bulletins.interval"));
    }

    #[test]
    fn collect_warnings_fires_above_max_for_each_knob() {
        let mut cfg = PollingConfig::default();

        cfg.overview.pg_status = Duration::from_secs(6 * 60);
        let w = collect_warnings(&cfg);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("polling.overview.pg_status"));
        assert!(w[0].contains("above"));
        cfg.overview.pg_status = Duration::from_secs(10);

        cfg.overview.sysdiag = Duration::from_secs(11 * 60);
        let w = collect_warnings(&cfg);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("polling.overview.sysdiag"));
        cfg.overview.sysdiag = Duration::from_secs(30);

        cfg.browser.interval = Duration::from_secs(6 * 60);
        let w = collect_warnings(&cfg);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("polling.browser.interval"));
        cfg.browser.interval = Duration::from_secs(15);

        cfg.bulletins.interval = Duration::from_secs(6 * 60);
        let w = collect_warnings(&cfg);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("polling.bulletins.interval"));
    }
}
