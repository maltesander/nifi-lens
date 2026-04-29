//! Configuration types and loader for nifi-lens.

pub mod init;
pub mod loader;
pub mod polling;

use std::path::PathBuf;

use serde::Deserialize;

/// Root of the user's config file.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Config {
    pub current_context: String,
    #[serde(default)]
    pub browser: BrowserConfig,
    #[serde(default)]
    pub bulletins: BulletinsConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub polling: PollingConfig,
    #[serde(default)]
    pub tracer: TracerConfig,
    #[serde(default)]
    pub contexts: Vec<Context>,
}

/// Browser-tab configuration set via `[browser]` in the TOML config.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct BrowserConfig {
    /// Initial-fetch ceiling for the queue listing two-phase async flow.
    /// If NiFi has not transitioned the request to `state == FINISHED`
    /// within this interval, the panel surfaces a timeout chip and the
    /// user retries via `r`. 30 s is comfortable for healthy clusters
    /// and tight enough to flag degraded ones.
    #[serde(default = "default_queue_listing_timeout", with = "humantime_serde")]
    pub queue_listing_timeout: std::time::Duration,

    /// Listing rows whose `queued_duration` exceeds this value render
    /// the entire row in `theme::warning()`. `0s` disables age-based
    /// highlighting (the `PEN` chip on penalized rows is unaffected).
    #[serde(
        default = "default_queue_listing_age_warning",
        with = "humantime_serde"
    )]
    pub queue_listing_age_warning: std::time::Duration,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            queue_listing_timeout: default_queue_listing_timeout(),
            queue_listing_age_warning: default_queue_listing_age_warning(),
        }
    }
}

fn default_queue_listing_timeout() -> std::time::Duration {
    std::time::Duration::from_secs(30)
}

fn default_queue_listing_age_warning() -> std::time::Duration {
    std::time::Duration::from_secs(5 * 60)
}

/// Bulletins-tab configuration set via `[bulletins]` in the TOML config.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct BulletinsConfig {
    /// Maximum number of bulletins kept in the rolling ring buffer that
    /// feeds the Bulletins tab and the Overview sparkline.
    /// Default 5000; valid range 100..=100_000. Larger values keep more
    /// history at the cost of memory (~1 MiB per 5000 rows).
    #[serde(default = "default_ring_size")]
    pub ring_size: usize,
}

impl Default for BulletinsConfig {
    fn default() -> Self {
        Self {
            ring_size: default_ring_size(),
        }
    }
}

fn default_ring_size() -> usize {
    5000
}

/// UI rendering preferences, set via `[ui]` in the TOML config.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct UiConfig {
    #[serde(default)]
    pub timestamp_format: crate::timestamp::TimestampFormat,
    #[serde(default)]
    pub timestamp_tz: crate::timestamp::TimestampTz,
}

/// One NiFi cluster context, set via `[[contexts]]` in the TOML config.
/// Multiple contexts may coexist; the active one is selected via
/// `current_context` at the top level or `--context <name>` on the CLI.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Context {
    /// Display name used in the UI and as the lookup key for `current_context` /
    /// `--context`. Must be unique across all contexts in the file.
    pub name: String,
    /// Base URL of the NiFi cluster, including scheme and port (e.g.
    /// `https://nifi.example.com:8443`). Trailing slashes are tolerated.
    pub url: String,
    /// Authentication mechanism for this context.
    /// See `AuthConfig` for the supported variants (`password`, `token`, `mtls`).
    pub auth: AuthConfig,
    /// Optional X-ProxiedEntitiesChain header value for proxied auth
    /// scenarios (e.g. running behind Knox or a custom auth proxy).
    #[serde(default)]
    pub proxied_entities_chain: Option<String>,
    /// How strictly to match the connected NiFi server's version against
    /// the API surface. See `VersionStrategy`.
    #[serde(default)]
    pub version_strategy: VersionStrategy,
    /// When `true`, accept self-signed and otherwise-invalid TLS
    /// certificates from the NiFi server. Use only for development; never
    /// in production. Default `false`.
    #[serde(default)]
    pub insecure_tls: bool,
    /// Optional path to a PEM-encoded CA certificate used to validate the
    /// NiFi server's TLS chain when the server presents a non-system-CA
    /// certificate.
    #[serde(default)]
    pub ca_cert_path: Option<PathBuf>,
    /// Route all traffic (HTTP and HTTPS) through this proxy URL.
    #[serde(default)]
    pub proxy_url: Option<String>,
    /// Route only HTTP traffic through this proxy URL.
    #[serde(default)]
    pub http_proxy_url: Option<String>,
    /// Route only HTTPS traffic through this proxy URL.
    #[serde(default)]
    pub https_proxy_url: Option<String>,
}

/// Authentication configuration for a single NiFi context.
///
/// Uses a tagged enum so serde can dispatch on the `type` field.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AuthConfig {
    Password(PasswordAuthConfig),
    Token(TokenAuthConfig),
    Mtls(MtlsAuthConfig),
}

/// Username + password credentials for NiFi basic/login auth.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct PasswordAuthConfig {
    pub username: String,
    #[serde(flatten)]
    pub credentials: PasswordCredentials,
}

/// Password credentials source.
///
/// `#[serde(untagged)]` means serde tries variants in declaration order and
/// picks the first one whose fields all match. If a user writes BOTH
/// `password_env = "..."` and `password = "..."` in the same block,
/// `EnvVar` wins silently because it is declared first. This is intentional:
/// env vars are the preferred credential source, so the ambiguity resolves
/// toward the safer option.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum PasswordCredentials {
    EnvVar { password_env: String },
    Plain { password: String },
}

/// Bearer-token credentials for NiFi token auth.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct TokenAuthConfig {
    #[serde(flatten)]
    pub credentials: TokenCredentials,
}

/// Token credentials source.
///
/// `#[serde(untagged)]` tries variants in order; `EnvVar` wins when both
/// `token_env` and `token` are present.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum TokenCredentials {
    EnvVar { token_env: String },
    Plain { token: String },
}

/// Mutual-TLS client identity for certificate-based auth.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct MtlsAuthConfig {
    pub client_identity_path: PathBuf,
}

/// Strategy for matching the connected NiFi server's version against the
/// dynamic-client API surface. Set per-context via the `version_strategy`
/// key.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VersionStrategy {
    /// Require an exact match with one of `nifi-rust-client`'s pinned
    /// versions. Refuses to start otherwise. Safest; this is the default.
    #[default]
    Strict,
    /// Pick the closest pinned version less than or equal to the
    /// detected server version. Trades safety for compatibility with
    /// patch-level NiFi versions the client wasn't built against.
    Closest,
    /// Pick the latest pinned version regardless of the detected server
    /// version. Use only when you know exactly what you're doing.
    Latest,
}

/// Resolved authentication credentials ready for use by the NiFi client.
/// Produced by `loader::load` after consulting env vars / warning on plaintext.
#[derive(Debug, Clone)]
pub enum ResolvedAuth {
    Password { username: String, password: String },
    Token { token: String },
    Mtls { client_identity_pem: Vec<u8> },
}

/// A context with credentials fully resolved. Produced by `loader::load`.
#[derive(Debug, Clone)]
pub struct ResolvedContext {
    pub name: String,
    pub url: String,
    pub auth: ResolvedAuth,
    pub proxied_entities_chain: Option<String>,
    pub version_strategy: VersionStrategy,
    pub insecure_tls: bool,
    pub ca_cert_path: Option<PathBuf>,
    pub proxy_url: Option<String>,
    pub http_proxy_url: Option<String>,
    pub https_proxy_url: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct PollingConfig {
    #[serde(default)]
    pub cluster: crate::cluster::ClusterPollingConfig,
}

#[cfg(test)]
mod polling_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn polling_section_defaults_when_omitted() {
        let toml_src = r#"
current_context = "dev"

[[contexts]]
name = "dev"
url = "https://dev:8443"

[contexts.auth]
type = "password"
username = "admin"
password = "x"
"#;
        let cfg: Config = toml::from_str(toml_src).expect("parses");
        // Spot-check a couple of cluster cadences match their defaults.
        assert_eq!(cfg.polling.cluster.root_pg_status, Duration::from_secs(10));
        assert_eq!(
            cfg.polling.cluster.system_diagnostics,
            Duration::from_secs(30)
        );
        assert_eq!(cfg.polling.cluster.bulletins, Duration::from_secs(5));
    }

    #[test]
    fn polling_section_parses_humantime_values() {
        let toml_src = r#"
current_context = "dev"

[polling.cluster]
root_pg_status      = "2s"
controller_services = "7s"
system_diagnostics  = "45s"
bulletins           = "250ms"
connections_by_pg   = "1m"

[[contexts]]
name = "dev"
url = "https://dev:8443"

[contexts.auth]
type = "password"
username = "admin"
password = "x"
"#;
        let cfg: Config = toml::from_str(toml_src).expect("parses");
        assert_eq!(cfg.polling.cluster.root_pg_status, Duration::from_secs(2));
        assert_eq!(
            cfg.polling.cluster.controller_services,
            Duration::from_secs(7)
        );
        assert_eq!(
            cfg.polling.cluster.system_diagnostics,
            Duration::from_secs(45)
        );
        assert_eq!(cfg.polling.cluster.bulletins, Duration::from_millis(250));
        assert_eq!(
            cfg.polling.cluster.connections_by_pg,
            Duration::from_secs(60)
        );
    }

    #[test]
    fn polling_partial_section_fills_in_defaults() {
        let toml_src = r#"
current_context = "dev"

[polling.cluster]
root_pg_status = "3s"

[[contexts]]
name = "dev"
url = "https://dev:8443"

[contexts.auth]
type = "password"
username = "admin"
password = "x"
"#;
        let cfg: Config = toml::from_str(toml_src).expect("parses");
        assert_eq!(cfg.polling.cluster.root_pg_status, Duration::from_secs(3));
        // Unspecified knobs keep their defaults.
        assert_eq!(
            cfg.polling.cluster.system_diagnostics,
            Duration::from_secs(30)
        );
        assert_eq!(cfg.polling.cluster.bulletins, Duration::from_secs(5));
    }

    #[test]
    fn polling_default_matches_serde_empty() {
        // Guard against drift between hand-written `Default` impls and
        // the `#[serde(default = "...")]` helpers.
        let from_default = PollingConfig::default();
        let from_serde: PollingConfig = toml::from_str("").expect("empty table parses as default");
        assert_eq!(from_default, from_serde);
    }
}

/// Tracer-tab configuration set via `[tracer]` in the TOML config.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct TracerConfig {
    /// Per-content-type streaming ceilings for the content viewer modal.
    /// See `TracerCeilingConfig`.
    #[serde(default)]
    pub ceiling: TracerCeilingConfig,

    /// **Deprecated:** legacy flat key from v0.1. If present, its value
    /// is mapped onto `ceiling.text` with a `tracing::warn!` (handled
    /// by `apply_legacy_tracer_keys` in the loader). Removed after v0.2.
    #[serde(default, deserialize_with = "deserialize_optional_byte_size")]
    pub modal_streaming_ceiling: Option<Option<usize>>,
}

/// Per-content-type streaming ceilings for the Tracer content viewer modal.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct TracerCeilingConfig {
    /// Per-side cap for `Text` / `Hex` / `Empty` bodies.
    #[serde(
        default = "default_ceiling_text",
        deserialize_with = "deserialize_byte_size_or_zero"
    )]
    pub text: Option<usize>,
    /// Per-side cap for fetched bytes that reach the Parquet/Avro decoder.
    #[serde(
        default = "default_ceiling_tabular",
        deserialize_with = "deserialize_byte_size_or_zero"
    )]
    pub tabular: Option<usize>,
    /// Per-side cap for bytes fed into `similar::TextDiff`.
    #[serde(
        default = "default_ceiling_diff",
        deserialize_with = "deserialize_byte_size_or_zero"
    )]
    pub diff: Option<usize>,
}

impl Default for TracerCeilingConfig {
    fn default() -> Self {
        Self {
            text: default_ceiling_text(),
            tabular: default_ceiling_tabular(),
            diff: default_ceiling_diff(),
        }
    }
}

fn default_ceiling_text() -> Option<usize> {
    Some(4 * crate::bytes::MIB as usize)
}

fn default_ceiling_tabular() -> Option<usize> {
    Some(64 * crate::bytes::MIB as usize)
}

fn default_ceiling_diff() -> Option<usize> {
    Some(16 * crate::bytes::MIB as usize)
}

/// Migrate the deprecated flat `modal_streaming_ceiling` key onto
/// `ceiling.text` if the user has not also set `[ceiling] text`
/// explicitly. Emits a `tracing::warn!` whenever the legacy key is
/// seen so users notice the deprecation.
///
/// **Edge case:** explicit-set detection compares `ceiling.text`
/// against the default value. A user who writes `text = "4MiB"`
/// (matching the default) AND sets the legacy key will have the
/// legacy value silently win. This is acceptable for a one-release
/// deprecation window — the warn still fires either way.
fn apply_legacy_tracer_keys(mut cfg: TracerConfig) -> TracerConfig {
    if let Some(legacy) = cfg.modal_streaming_ceiling.take() {
        let explicit_text_set = cfg.ceiling.text != default_ceiling_text();
        if !explicit_text_set {
            cfg.ceiling.text = legacy;
        }
        tracing::warn!(
            "[tracer] modal_streaming_ceiling is deprecated; use [tracer.ceiling] text instead"
        );
    }
    cfg
}

/// Same byte-size parsing as `deserialize_byte_size_or_zero`, but
/// wrapped in an outer `Option` so a missing key parses as `None`
/// (i.e. "user didn't set the legacy key at all").
fn deserialize_optional_byte_size<'de, D>(
    deserializer: D,
) -> Result<Option<Option<usize>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let raw = Option::<String>::deserialize(deserializer)?;
    Ok(raw.map(|s| parse_byte_size_or_zero(&s)))
}

/// Deserialize a human-readable byte-size string into `Option<usize>`.
///
/// Accepts `"N"`, `"N B"`, `"N KiB"`, `"N MiB"`, `"N GiB"`.
/// `"0"` (or any integer ≤ 0) maps to `None` (unbounded).
/// Unparseable values warn-log and fall back to the default (4 MiB).
fn deserialize_byte_size_or_zero<'de, D>(deserializer: D) -> Result<Option<usize>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let raw = String::deserialize(deserializer)?;
    Ok(parse_byte_size_or_zero(&raw))
}

fn parse_byte_size_or_zero(raw: &str) -> Option<usize> {
    let trimmed = raw.trim();
    if let Ok(n) = trimmed.parse::<i64>() {
        if n <= 0 {
            return None;
        }
        return Some(n as usize);
    }
    let suffixes: &[(&str, usize)] = &[
        ("GiB", 1024 * 1024 * 1024),
        ("MiB", 1024 * 1024),
        ("KiB", 1024),
        ("B", 1),
    ];
    for (sfx, mult) in suffixes {
        if let Some(num) = trimmed.strip_suffix(sfx)
            && let Ok(n) = num.trim().parse::<i64>()
        {
            return if n <= 0 {
                None
            } else {
                Some((n as usize).saturating_mul(*mult))
            };
        }
    }
    tracing::warn!(
        value = %raw,
        "tracer byte-size config: unparseable value, falling back to default"
    );
    default_ceiling_text()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn password_env_toml(username: &str, env_var: &str) -> String {
        format!(
            r#"
            name = "test"
            url = "https://nifi.example.com"
            [auth]
            type = "password"
            username = "{username}"
            password_env = "{env_var}"
            "#
        )
    }

    fn password_plain_toml(username: &str, password: &str) -> String {
        format!(
            r#"
            name = "test"
            url = "https://nifi.example.com"
            [auth]
            type = "password"
            username = "{username}"
            password = "{password}"
            "#
        )
    }

    fn token_env_toml(env_var: &str) -> String {
        format!(
            r#"
            name = "test"
            url = "https://nifi.example.com"
            [auth]
            type = "token"
            token_env = "{env_var}"
            "#
        )
    }

    fn token_plain_toml(token: &str) -> String {
        format!(
            r#"
            name = "test"
            url = "https://nifi.example.com"
            [auth]
            type = "token"
            token = "{token}"
            "#
        )
    }

    fn mtls_toml(path: &str) -> String {
        format!(
            r#"
            name = "test"
            url = "https://nifi.example.com"
            [auth]
            type = "mtls"
            client_identity_path = "{path}"
            "#
        )
    }

    #[test]
    fn password_auth_config_deserializes_with_env_var() {
        let ctx: Context = toml::from_str(&password_env_toml("alice", "MY_NIFI_PASS")).unwrap();
        assert_eq!(
            ctx.auth,
            AuthConfig::Password(PasswordAuthConfig {
                username: "alice".into(),
                credentials: PasswordCredentials::EnvVar {
                    password_env: "MY_NIFI_PASS".into()
                },
            })
        );
    }

    #[test]
    fn password_auth_config_deserializes_with_plaintext() {
        let ctx: Context = toml::from_str(&password_plain_toml("bob", "s3cret")).unwrap();
        assert_eq!(
            ctx.auth,
            AuthConfig::Password(PasswordAuthConfig {
                username: "bob".into(),
                credentials: PasswordCredentials::Plain {
                    password: "s3cret".into()
                },
            })
        );
    }

    #[test]
    fn token_auth_config_deserializes_with_env_var() {
        let ctx: Context = toml::from_str(&token_env_toml("NIFI_TOKEN")).unwrap();
        assert_eq!(
            ctx.auth,
            AuthConfig::Token(TokenAuthConfig {
                credentials: TokenCredentials::EnvVar {
                    token_env: "NIFI_TOKEN".into()
                },
            })
        );
    }

    #[test]
    fn token_auth_config_deserializes_with_plaintext() {
        let ctx: Context = toml::from_str(&token_plain_toml("mytoken123")).unwrap();
        assert_eq!(
            ctx.auth,
            AuthConfig::Token(TokenAuthConfig {
                credentials: TokenCredentials::Plain {
                    token: "mytoken123".into()
                },
            })
        );
    }

    #[test]
    fn mtls_auth_config_deserializes() {
        let ctx: Context = toml::from_str(&mtls_toml("/etc/certs/client.pem")).unwrap();
        assert_eq!(
            ctx.auth,
            AuthConfig::Mtls(MtlsAuthConfig {
                client_identity_path: PathBuf::from("/etc/certs/client.pem"),
            })
        );
    }

    #[test]
    fn proxied_entities_chain_deserializes() {
        let toml = r#"
            name = "test"
            url = "https://nifi.example.com"
            proxied_entities_chain = "<CN=proxy,OU=NiFi>"
            [auth]
            type = "token"
            token = "tok"
        "#;
        let ctx: Context = toml::from_str(toml).unwrap();
        assert_eq!(
            ctx.proxied_entities_chain.as_deref(),
            Some("<CN=proxy,OU=NiFi>")
        );
    }

    #[test]
    fn proxy_url_fields_deserialize() {
        let toml = r#"
            name = "test"
            url = "https://nifi.example.com"
            proxy_url       = "http://proxy.internal:3128"
            http_proxy_url  = "http://proxy.internal:3129"
            https_proxy_url = "http://proxy.internal:3130"
            [auth]
            type = "token"
            token = "tok"
        "#;
        let ctx: Context = toml::from_str(toml).unwrap();
        assert_eq!(ctx.proxy_url.as_deref(), Some("http://proxy.internal:3128"));
        assert_eq!(
            ctx.http_proxy_url.as_deref(),
            Some("http://proxy.internal:3129")
        );
        assert_eq!(
            ctx.https_proxy_url.as_deref(),
            Some("http://proxy.internal:3130")
        );
    }

    #[test]
    fn proxy_url_fields_default_to_none() {
        let toml = r#"
            name = "test"
            url = "https://nifi.example.com"
            [auth]
            type = "token"
            token = "tok"
        "#;
        let ctx: Context = toml::from_str(toml).unwrap();
        assert!(ctx.proxy_url.is_none());
        assert!(ctx.http_proxy_url.is_none());
        assert!(ctx.https_proxy_url.is_none());
    }

    #[test]
    fn unknown_auth_type_errors() {
        let toml = r#"
            name = "test"
            url = "https://nifi.example.com"
            [auth]
            type = "kerberos"
            principal = "user@REALM"
        "#;
        assert!(toml::from_str::<Context>(toml).is_err());
    }

    #[test]
    fn missing_auth_errors() {
        let toml = r#"
            name = "test"
            url = "https://nifi.example.com"
        "#;
        assert!(toml::from_str::<Context>(toml).is_err());
    }

    #[test]
    fn ui_config_defaults_when_missing() {
        let raw = r#"
            current_context = "dev"
            [[contexts]]
            name = "dev"
            url = "https://nifi.example.com"
            [contexts.auth]
            type = "token"
            token = "tok"
        "#;
        let cfg: Config = toml::from_str(raw).unwrap();
        assert_eq!(
            cfg.ui.timestamp_format,
            crate::timestamp::TimestampFormat::Short
        );
        assert_eq!(cfg.ui.timestamp_tz, crate::timestamp::TimestampTz::Utc);
    }

    #[test]
    fn ui_config_parses_iso_and_local() {
        let raw = r#"
            current_context = "dev"
            [ui]
            timestamp_format = "iso"
            timestamp_tz = "local"
            [[contexts]]
            name = "dev"
            url = "https://nifi.example.com"
            [contexts.auth]
            type = "token"
            token = "tok"
        "#;
        let cfg: Config = toml::from_str(raw).unwrap();
        assert_eq!(
            cfg.ui.timestamp_format,
            crate::timestamp::TimestampFormat::Iso
        );
        assert_eq!(cfg.ui.timestamp_tz, crate::timestamp::TimestampTz::Local);
    }

    #[test]
    fn ui_config_rejects_unknown_format() {
        let raw = r#"
            current_context = "dev"
            [ui]
            timestamp_format = "nope"
            [[contexts]]
            name = "dev"
            url = "https://nifi.example.com"
            [contexts.auth]
            type = "token"
            token = "tok"
        "#;
        let result: Result<Config, _> = toml::from_str(raw);
        assert!(
            result.is_err(),
            "expected parse error for invalid timestamp_format"
        );
    }

    #[test]
    fn tracer_section_legacy_key_default_when_absent() {
        // No [tracer] section → defaults.
        let cfg: TracerConfig = toml::from_str("").unwrap();
        let cfg = apply_legacy_tracer_keys(cfg);
        assert_eq!(cfg.ceiling.text, Some(4 * crate::bytes::MIB as usize));
    }

    #[test]
    fn tracer_section_legacy_key_parses_mib_suffix() {
        let cfg: TracerConfig = toml::from_str(
            r#"
            modal_streaming_ceiling = "16MiB"
            "#,
        )
        .unwrap();
        let cfg = apply_legacy_tracer_keys(cfg);
        assert_eq!(cfg.ceiling.text, Some(16 * crate::bytes::MIB as usize));
    }

    #[test]
    fn tracer_section_legacy_key_parses_kib_and_bare_bytes() {
        let cfg: TracerConfig = toml::from_str(
            r#"
            modal_streaming_ceiling = "512KiB"
            "#,
        )
        .unwrap();
        let cfg = apply_legacy_tracer_keys(cfg);
        assert_eq!(cfg.ceiling.text, Some(512 * 1024));

        let cfg2: TracerConfig = toml::from_str(
            r#"
            modal_streaming_ceiling = "65536"
            "#,
        )
        .unwrap();
        let cfg2 = apply_legacy_tracer_keys(cfg2);
        assert_eq!(cfg2.ceiling.text, Some(65536));
    }

    #[test]
    fn tracer_section_legacy_key_zero_means_unbounded() {
        let cfg: TracerConfig = toml::from_str(
            r#"
            modal_streaming_ceiling = "0"
            "#,
        )
        .unwrap();
        let cfg = apply_legacy_tracer_keys(cfg);
        assert_eq!(cfg.ceiling.text, None);
    }

    #[test]
    fn tracer_section_legacy_key_bad_value_falls_back_to_default() {
        let cfg: TracerConfig = toml::from_str(
            r#"
            modal_streaming_ceiling = "not-a-size"
            "#,
        )
        .unwrap();
        let cfg = apply_legacy_tracer_keys(cfg);
        // Bad values warn-log but fall back to the 4 MiB default.
        assert_eq!(cfg.ceiling.text, Some(4 * crate::bytes::MIB as usize));
    }

    #[test]
    fn tracer_legacy_modal_streaming_ceiling_maps_to_text() {
        let cfg: TracerConfig = toml::from_str(
            r#"
            modal_streaming_ceiling = "16MiB"
            "#,
        )
        .unwrap();
        let cfg = apply_legacy_tracer_keys(cfg);
        // Legacy key wins for `text` ceiling; others stay default.
        assert_eq!(cfg.ceiling.text, Some(16 * crate::bytes::MIB as usize));
        assert_eq!(cfg.ceiling.tabular, Some(64 * crate::bytes::MIB as usize));
        assert_eq!(cfg.ceiling.diff, Some(16 * crate::bytes::MIB as usize));
    }

    #[test]
    fn tracer_explicit_text_overrides_legacy_key() {
        let cfg: TracerConfig = toml::from_str(
            r#"
            modal_streaming_ceiling = "16MiB"
            [ceiling]
            text = "8MiB"
            "#,
        )
        .unwrap();
        let cfg = apply_legacy_tracer_keys(cfg);
        // Explicit `[ceiling] text` wins over the legacy key.
        assert_eq!(cfg.ceiling.text, Some(8 * crate::bytes::MIB as usize));
    }

    #[test]
    fn tracer_no_legacy_key_means_no_warn_and_no_change() {
        let cfg: TracerConfig = toml::from_str("").unwrap();
        let cfg = apply_legacy_tracer_keys(cfg);
        assert_eq!(cfg.ceiling.text, Some(4 * crate::bytes::MIB as usize));
        assert_eq!(cfg.modal_streaming_ceiling, None);
    }

    #[test]
    fn tracer_ceiling_section_defaults() {
        let cfg: TracerConfig = toml::from_str("").unwrap();
        assert_eq!(cfg.ceiling.text, Some(4 * crate::bytes::MIB as usize));
        assert_eq!(cfg.ceiling.tabular, Some(64 * crate::bytes::MIB as usize));
        assert_eq!(cfg.ceiling.diff, Some(16 * crate::bytes::MIB as usize));
    }

    #[test]
    fn tracer_ceiling_section_parses_all_three_keys() {
        let cfg: TracerConfig = toml::from_str(
            r#"
            [ceiling]
            text    = "8MiB"
            tabular = "256MiB"
            diff    = "32MiB"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.ceiling.text, Some(8 * crate::bytes::MIB as usize));
        assert_eq!(cfg.ceiling.tabular, Some(256 * crate::bytes::MIB as usize));
        assert_eq!(cfg.ceiling.diff, Some(32 * crate::bytes::MIB as usize));
    }

    #[test]
    fn tracer_ceiling_zero_is_unbounded() {
        let cfg: TracerConfig = toml::from_str(
            r#"
            [ceiling]
            tabular = "0"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.ceiling.tabular, None);
        // Other keys keep defaults.
        assert_eq!(cfg.ceiling.text, Some(4 * crate::bytes::MIB as usize));
    }

    #[test]
    fn browser_section_defaults_match_spec() {
        let cfg: BrowserConfig = toml::from_str("").expect("empty config parses with defaults");
        assert_eq!(
            cfg.queue_listing_timeout,
            std::time::Duration::from_secs(30)
        );
        assert_eq!(
            cfg.queue_listing_age_warning,
            std::time::Duration::from_secs(5 * 60)
        );
    }

    #[test]
    fn browser_section_overrides_defaults() {
        let toml = r#"
            queue_listing_timeout = "10s"
            queue_listing_age_warning = "30s"
        "#;
        let cfg: BrowserConfig = toml::from_str(toml).expect("parses");
        assert_eq!(
            cfg.queue_listing_timeout,
            std::time::Duration::from_secs(10)
        );
        assert_eq!(
            cfg.queue_listing_age_warning,
            std::time::Duration::from_secs(30)
        );
    }

    #[test]
    fn browser_age_warning_zero_disables() {
        let toml = r#"
            queue_listing_age_warning = "0s"
        "#;
        let cfg: BrowserConfig = toml::from_str(toml).expect("parses");
        assert_eq!(cfg.queue_listing_age_warning, std::time::Duration::ZERO);
    }

    #[test]
    fn browser_default_matches_serde_empty() {
        // Guard against drift between hand-written `Default` impl and the
        // `#[serde(default = "...")]` helpers.
        let from_default = BrowserConfig::default();
        let from_serde: BrowserConfig = toml::from_str("").expect("empty table parses as default");
        assert_eq!(from_default, from_serde);
    }
}
