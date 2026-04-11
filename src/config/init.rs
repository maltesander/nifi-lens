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

# Example context. Duplicate this block for additional clusters.
[[contexts]]
# Human-readable name. Matches `current_context` above and `--context` on
# the CLI.
name = "dev"

# Base URL of the NiFi instance (https:// required).
url = "https://nifi-dev.internal:8443"

# Login username.
username = "admin"

# Password is read from an environment variable. Export it before running
# nifilens:
#
#     export NIFILENS_DEV_PASSWORD=...
#
# Alternatively, replace `password_env = "..."` with `password = "..."` to
# use a literal (nifilens will emit a warning on every load).
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
}
