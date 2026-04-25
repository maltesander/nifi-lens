//! Polling cadences for cluster-wide endpoints. Deserialized from the
//! `[polling.cluster]` section of `config.toml`. Duration fields use the
//! humantime format (`"10s"`, `"30s"`, `"5m"`).
//!
//! Phase rollout note: in Task 1 the struct lives alongside the existing
//! per-view polling sections. Task 11 removes the per-view sections.

use std::time::Duration;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ClusterPollingConfig {
    #[serde(default = "default_root_pg_status", with = "humantime_serde")]
    pub root_pg_status: Duration,
    #[serde(default = "default_controller_services", with = "humantime_serde")]
    pub controller_services: Duration,
    #[serde(default = "default_controller_status", with = "humantime_serde")]
    pub controller_status: Duration,
    #[serde(default = "default_system_diagnostics", with = "humantime_serde")]
    pub system_diagnostics: Duration,
    #[serde(default = "default_bulletins", with = "humantime_serde")]
    pub bulletins: Duration,
    #[serde(default = "default_cluster_nodes", with = "humantime_serde")]
    pub cluster_nodes: Duration,
    #[serde(default = "default_tls_certs", with = "humantime_serde")]
    pub tls_certs: Duration,
    #[serde(default = "default_connections_by_pg", with = "humantime_serde")]
    pub connections_by_pg: Duration,
    #[serde(default = "default_version_control", with = "humantime_serde")]
    pub version_control: Duration,
    #[serde(default = "default_about", with = "humantime_serde")]
    pub about: Duration,
    #[serde(default = "default_max_interval", with = "humantime_serde")]
    pub max_interval: Duration,
    #[serde(default = "default_jitter_percent")]
    pub jitter_percent: u8,
}

impl Default for ClusterPollingConfig {
    fn default() -> Self {
        Self {
            root_pg_status: default_root_pg_status(),
            controller_services: default_controller_services(),
            controller_status: default_controller_status(),
            system_diagnostics: default_system_diagnostics(),
            bulletins: default_bulletins(),
            cluster_nodes: default_cluster_nodes(),
            tls_certs: default_tls_certs(),
            connections_by_pg: default_connections_by_pg(),
            version_control: default_version_control(),
            about: default_about(),
            max_interval: default_max_interval(),
            jitter_percent: default_jitter_percent(),
        }
    }
}

fn default_root_pg_status() -> Duration {
    Duration::from_secs(10)
}
fn default_controller_services() -> Duration {
    Duration::from_secs(10)
}
fn default_controller_status() -> Duration {
    Duration::from_secs(10)
}
fn default_system_diagnostics() -> Duration {
    Duration::from_secs(30)
}
fn default_bulletins() -> Duration {
    Duration::from_secs(5)
}
fn default_cluster_nodes() -> Duration {
    Duration::from_secs(5)
}
fn default_tls_certs() -> Duration {
    Duration::from_secs(3600)
}
fn default_connections_by_pg() -> Duration {
    Duration::from_secs(15)
}
fn default_version_control() -> Duration {
    Duration::from_secs(30)
}
fn default_about() -> Duration {
    Duration::from_secs(300)
}
fn default_max_interval() -> Duration {
    Duration::from_secs(60)
}
fn default_jitter_percent() -> u8 {
    20
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let c = ClusterPollingConfig::default();
        assert_eq!(c.root_pg_status, Duration::from_secs(10));
        assert_eq!(c.system_diagnostics, Duration::from_secs(30));
        assert_eq!(c.bulletins, Duration::from_secs(5));
        assert_eq!(c.cluster_nodes, Duration::from_secs(5));
        assert_eq!(c.tls_certs, Duration::from_secs(3600));
        assert_eq!(c.about, Duration::from_secs(300));
        assert_eq!(c.max_interval, Duration::from_secs(60));
        assert_eq!(c.jitter_percent, 20);
    }

    #[test]
    fn parses_tls_certs_override() {
        let toml = r#"tls_certs = "15m""#;
        let c: ClusterPollingConfig = toml::from_str(toml).unwrap();
        assert_eq!(c.tls_certs, Duration::from_secs(900));
    }

    #[test]
    fn parses_cluster_nodes_override() {
        let toml = r#"cluster_nodes = "3s""#;
        let c: ClusterPollingConfig = toml::from_str(toml).unwrap();
        assert_eq!(c.cluster_nodes, Duration::from_secs(3));
    }

    #[test]
    fn parses_toml_section() {
        let toml = r#"
            root_pg_status = "7s"
            controller_services = "12s"
            controller_status = "8s"
            system_diagnostics = "45s"
            bulletins = "3s"
            connections_by_pg = "20s"
            about = "10m"
            max_interval = "2m"
            jitter_percent = 15
        "#;
        let c: ClusterPollingConfig = toml::from_str(toml).unwrap();
        assert_eq!(c.root_pg_status, Duration::from_secs(7));
        assert_eq!(c.about, Duration::from_secs(600));
        assert_eq!(c.max_interval, Duration::from_secs(120));
        assert_eq!(c.jitter_percent, 15);
    }

    #[test]
    fn omitted_section_yields_defaults() {
        let c: ClusterPollingConfig = toml::from_str("").unwrap();
        assert_eq!(c, ClusterPollingConfig::default());
    }

    #[test]
    fn version_control_default_is_30s() {
        let c = ClusterPollingConfig::default();
        assert_eq!(c.version_control, Duration::from_secs(30));
    }

    #[test]
    fn parses_version_control_override() {
        let toml = r#"version_control = "2m""#;
        let c: ClusterPollingConfig = toml::from_str(toml).unwrap();
        assert_eq!(c.version_control, Duration::from_secs(120));
    }
}
