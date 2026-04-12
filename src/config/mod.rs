//! Configuration types and loader for nifi-lens.

pub mod init;
pub mod loader;

use std::path::PathBuf;

use serde::Deserialize;

/// Root of the user's config file.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Config {
    pub current_context: String,
    #[serde(default)]
    pub bulletins: BulletinsConfig,
    #[serde(default)]
    pub contexts: Vec<Context>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct BulletinsConfig {
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

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Context {
    pub name: String,
    pub url: String,
    pub auth: AuthConfig,
    #[serde(default)]
    pub proxied_entities_chain: Option<String>,
    #[serde(default)]
    pub version_strategy: VersionStrategy,
    #[serde(default)]
    pub insecure_tls: bool,
    #[serde(default)]
    pub ca_cert_path: Option<PathBuf>,
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

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VersionStrategy {
    #[default]
    Strict,
    Closest,
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
}
