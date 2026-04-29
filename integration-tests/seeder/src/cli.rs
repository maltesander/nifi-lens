//! Clap-derive CLI for the fixture seeder.

use std::path::PathBuf;

#[derive(clap::Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Path to nifilens config.toml
    #[arg(long, value_name = "PATH")]
    pub config: PathBuf,

    /// Which context from the config to seed
    #[arg(long, value_name = "NAME")]
    pub context: String,

    /// Exit 0 immediately if the fixture marker PG is already present.
    /// Intended for live-dev iteration: re-running the seeder against
    /// an already-seeded cluster becomes a no-op.
    #[arg(long)]
    pub skip_if_seeded: bool,

    /// Log level (off, error, warn, info, debug, trace). Default: info.
    #[arg(long, value_name = "LEVEL", default_value = "info")]
    pub log_level: String,

    /// Sleep this long after seeding the topology before mutating the
    /// `usd_rate` parameter context value. Default `0s` for CI; longer
    /// (e.g. `5m`) for live demo capture so the "before" state is
    /// observable.
    #[arg(long, value_name = "DURATION", default_value = "0s",
          value_parser = humantime::parse_duration)]
    pub break_after: std::time::Duration,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_required_args() {
        let args = Args::try_parse_from([
            "nifilens-fixture-seeder",
            "--config",
            "/tmp/x.toml",
            "--context",
            "dev-nifi-2-6-0",
        ])
        .unwrap();
        assert_eq!(args.config, PathBuf::from("/tmp/x.toml"));
        assert_eq!(args.context, "dev-nifi-2-6-0");
        assert!(!args.skip_if_seeded);
        assert_eq!(args.log_level, "info");
    }

    #[test]
    fn parses_skip_if_seeded_flag() {
        let args = Args::try_parse_from([
            "nifilens-fixture-seeder",
            "--config",
            "/x.toml",
            "--context",
            "dev-nifi-2-6-0",
            "--skip-if-seeded",
        ])
        .unwrap();
        assert!(args.skip_if_seeded);
    }

    #[test]
    fn missing_context_errors() {
        let err =
            Args::try_parse_from(["nifilens-fixture-seeder", "--config", "/x.toml"]).unwrap_err();
        let rendered = err.render().to_string();
        assert!(
            rendered.contains("--context"),
            "error should mention --context, got: {rendered}"
        );
    }

    #[test]
    fn parses_break_after_flag() {
        let args = Args::try_parse_from([
            "nifilens-fixture-seeder",
            "--config",
            "/x.toml",
            "--context",
            "dev-nifi-2-6-0",
            "--break-after",
            "30s",
        ])
        .unwrap();
        assert_eq!(args.break_after, std::time::Duration::from_secs(30));
    }

    #[test]
    fn break_after_defaults_to_zero() {
        let args = Args::try_parse_from([
            "nifilens-fixture-seeder",
            "--config",
            "/x.toml",
            "--context",
            "dev-nifi-2-6-0",
        ])
        .unwrap();
        assert_eq!(args.break_after, std::time::Duration::ZERO);
    }
}
