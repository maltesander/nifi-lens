//! `nifilens config init` — writes a commented template at the default path.

use std::path::PathBuf;

use snafu::ResultExt;

use crate::config::loader::resolve_path;
use crate::error::{ConfigAlreadyExistsSnafu, ConfigWriteFailedSnafu, NifiLensError};

const TEMPLATE: &str = r#"# nifi-lens configuration file.
#
# Kubeconfig-style: define one or more contexts and pick an active one via
# `current_context`. Override at runtime with `nifilens --context <name>`.

current_context = "dev"

# Bulletins tab ring buffer size. The Bulletins tab keeps a rolling
# in-memory window of recently-seen bulletins; this knob caps it.
# Valid range: 100..=100000. Default: 5000.
# [bulletins]
# ring_size = 5000

# Timestamp display options for the Bulletins and Tracer tabs.
# [ui]
# # Timestamp display format:
# #   "short"  — HH:MM:SS for today, "MMM DD HH:MM:SS" for older
# #   "iso"    — 2026-04-12T14:32:18Z
# #   "human"  — Apr 12 14:32:18
# timestamp_format = "short"
# # "utc" or "local" — "local" uses the host machine time zone.
# timestamp_tz = "utc"

# Example context. Duplicate this block for additional clusters.
[[contexts]]
# Human-readable name. Matches `current_context` above and `--context` on
# the CLI.
name = "dev"

# Base URL of the NiFi instance (https:// required).
url = "https://nifi-dev.internal:8443"

# Authentication. Three types are supported:
#
#   type = "password"  — username + password (or password_env)
#   type = "token"     — pre-obtained JWT (token or token_env)
#   type = "mtls"      — mutual TLS with client_identity_path
#
# Password is read from an environment variable. Export it before running
# nifilens:
#
#     export NIFILENS_DEV_PASSWORD=...
#
# Alternatively, replace `password_env = "..."` with `password = "..."` to
# use a literal (nifilens will emit a warning on every load).
[contexts.auth]
type = "password"
username = "admin"
password_env = "NIFILENS_DEV_PASSWORD"

# How to map the detected NiFi version to a supported API module:
#   "strict"   — exact major.minor match; fail otherwise (default)
#   "closest"  — nearest supported minor; ties go to the lower version
#   "latest"   — highest supported minor within the same major
version_strategy = "strict"

# If true: accept any TLS certificate, skip hostname verification.
# Use only for local dev against self-signed certs; prefer ca_cert_path.
insecure_tls = false

# Optional: path to a PEM-encoded CA certificate to trust in addition to
# the system trust store.
# ca_cert_path = "/etc/nifi-lens/certs/dev-ca.crt"

# Optional: HTTP proxy routing. Use proxy_url to route all traffic through a
# single proxy, or http_proxy_url / https_proxy_url to route by scheme.
# proxy_url = "http://proxy.internal:3128"
# http_proxy_url  = "http://proxy.internal:3128"
# https_proxy_url = "http://proxy.internal:3128"

# Poll cadences for the central cluster store. Values use the humantime
# format — examples: "5s", "750ms", "2m", "1h30m". Defaults shown.
# Cadences scale adaptively up to `max_interval` on slow clusters and
# are jittered by ±`jitter_percent/100` to avoid synchronized bursts.
# Out-of-band values (below the cluster-hammering floor or above the
# ui-feels-stale ceiling) produce a warning in the log file but are
# accepted as-is.
#
# [polling.cluster]
# root_pg_status      = "10s"   # recursive PG/processor/connection walk
# controller_services = "10s"   # root-scoped controller services
# controller_status   = "10s"   # /flow/status aggregate counters
# system_diagnostics  = "30s"   # /system-diagnostics (nodewise if available)
# bulletins           = "5s"    # /flow/bulletin-board cursor poll
# connections_by_pg   = "15s"   # per-PG connection endpoint backfill
# about               = "5m"    # /flow/about banner info
# max_interval        = "60s"   # adaptive cap on slow clusters
# jitter_percent      = 20      # ±20% jitter on each sleep
"#;

pub fn write_template(force: bool) -> Result<PathBuf, NifiLensError> {
    let path = resolve_path(None);

    if path.exists() && !force {
        return ConfigAlreadyExistsSnafu { path }.fail();
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| NifiLensError::Io { source })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(parent)
                .map_err(|source| NifiLensError::Io { source })?
                .permissions();
            perms.set_mode(0o700);
            std::fs::set_permissions(parent, perms)
                .map_err(|source| NifiLensError::Io { source })?;
        }
    }

    std::fs::write(&path, TEMPLATE).context(ConfigWriteFailedSnafu { path: path.clone() })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)
            .map_err(|source| NifiLensError::Io { source })?
            .permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&path, perms).map_err(|source| NifiLensError::Io { source })?;
    }

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_is_valid_toml_and_parses_as_config() {
        let parsed: crate::config::Config = toml::from_str(TEMPLATE).expect("template parses");
        assert_eq!(parsed.current_context, "dev");
        assert_eq!(parsed.contexts.len(), 1);
        assert_eq!(parsed.contexts[0].name, "dev");
    }

    #[test]
    fn template_bulletins_ring_size_defaults_when_block_commented() {
        let parsed: crate::config::Config = toml::from_str(TEMPLATE).expect("template parses");
        assert_eq!(parsed.bulletins.ring_size, 5000);
    }

    #[test]
    fn template_polling_defaults_when_block_commented() {
        use std::time::Duration;
        let parsed: crate::config::Config = toml::from_str(TEMPLATE).expect("template parses");
        // Spot-check the cluster cadences — the full set is exercised in
        // `config::polling` and `cluster::config`.
        assert_eq!(
            parsed.polling.cluster.root_pg_status,
            Duration::from_secs(10)
        );
        assert_eq!(
            parsed.polling.cluster.system_diagnostics,
            Duration::from_secs(30)
        );
        assert_eq!(
            parsed.polling.cluster.connections_by_pg,
            Duration::from_secs(15)
        );
        assert_eq!(parsed.polling.cluster.bulletins, Duration::from_secs(5));
    }
}
