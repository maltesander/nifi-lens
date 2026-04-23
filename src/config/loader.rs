//! Path resolution, permission checks, parsing, and credential resolution
//! for the nifi-lens config file.

use std::path::{Path, PathBuf};

use snafu::ResultExt;

use crate::cli::Args;
use crate::config::{
    AuthConfig, Config, PasswordCredentials, ResolvedAuth, ResolvedContext, TokenCredentials,
};
use crate::error::{
    CaCertNotFoundSnafu, ClientIdentityNotFoundSnafu, ClientIdentityReadFailedSnafu,
    ConfigMissingSnafu, ConfigParseSnafu, ConfigWorldReadableSnafu, MissingAuthEnvVarSnafu,
    NifiLensError, UnknownContextSnafu,
};

/// Load the config file pointed at by `args`, resolve the active context,
/// resolve its credentials, and return both.
pub fn load(args: &Args) -> Result<(Config, ResolvedContext), NifiLensError> {
    let path = resolve_path(args.config.clone());
    if !path.exists() {
        return ConfigMissingSnafu { path }.fail();
    }

    check_permissions(&path)?;

    let contents = std::fs::read_to_string(&path).map_err(|source| NifiLensError::Io { source })?;
    let mut config: Config =
        toml::from_str(&contents).context(ConfigParseSnafu { path: path.clone() })?;

    config.tracer = crate::config::apply_legacy_tracer_keys(config.tracer);

    validate_bulletins(&config.bulletins)?;
    crate::config::polling::warn_if_out_of_band(&config.polling);

    let active_name = args
        .context
        .as_deref()
        .unwrap_or(&config.current_context)
        .to_string();

    let context = config
        .contexts
        .iter()
        .find(|c| c.name == active_name)
        .ok_or_else(|| {
            UnknownContextSnafu {
                name: active_name.clone(),
                available: config
                    .contexts
                    .iter()
                    .map(|c| c.name.clone())
                    .collect::<Vec<_>>(),
            }
            .build()
        })?;

    let auth = resolve_auth(&context.name, &context.auth)?;

    validate_tls(context)?;

    validate_proxy_urls(context)?;

    let resolved = ResolvedContext {
        name: context.name.clone(),
        url: context.url.clone(),
        auth,
        version_strategy: context.version_strategy,
        insecure_tls: context.insecure_tls,
        ca_cert_path: context.ca_cert_path.clone(),
        proxied_entities_chain: context.proxied_entities_chain.clone(),
        proxy_url: context.proxy_url.clone(),
        http_proxy_url: context.http_proxy_url.clone(),
        https_proxy_url: context.https_proxy_url.clone(),
    };

    Ok((config, resolved))
}

/// Resolve the config file path. Precedence: explicit --config, then
/// $XDG_CONFIG_HOME/nifilens/config.toml, then $HOME/.config/nifilens/config.toml.
pub fn resolve_path(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(p) = explicit {
        return p;
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("nifilens/config.toml");
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config/nifilens/config.toml")
}

#[cfg(unix)]
fn check_permissions(path: &Path) -> Result<(), NifiLensError> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path).map_err(|source| NifiLensError::Io { source })?;
    let mode = meta.permissions().mode();
    if mode & 0o077 != 0 {
        return ConfigWorldReadableSnafu {
            path: path.to_path_buf(),
        }
        .fail();
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_permissions(_path: &Path) -> Result<(), NifiLensError> {
    Ok(())
}

fn validate_bulletins(bulletins: &crate::config::BulletinsConfig) -> Result<(), NifiLensError> {
    if !(100..=100_000).contains(&bulletins.ring_size) {
        return Err(NifiLensError::ConfigInvalid {
            detail: format!(
                "bulletins.ring_size must be between 100 and 100000 (got {})",
                bulletins.ring_size
            ),
        });
    }
    Ok(())
}

pub(crate) fn resolve_env_var(context_name: &str, var: &str) -> Result<String, NifiLensError> {
    std::env::var(var).map_err(|_| {
        MissingAuthEnvVarSnafu {
            context: context_name.to_string(),
            var: var.to_string(),
        }
        .build()
    })
}

pub(crate) fn resolve_auth(
    context_name: &str,
    auth: &AuthConfig,
) -> Result<ResolvedAuth, NifiLensError> {
    match auth {
        AuthConfig::Password(pw) => {
            let password = match &pw.credentials {
                PasswordCredentials::EnvVar { password_env } => {
                    resolve_env_var(context_name, password_env)?
                }
                PasswordCredentials::Plain { password } => {
                    tracing::warn!(context = %context_name, "plaintext password in config");
                    password.clone()
                }
            };
            Ok(ResolvedAuth::Password {
                username: pw.username.clone(),
                password,
            })
        }
        AuthConfig::Token(tok) => {
            let token = match &tok.credentials {
                TokenCredentials::EnvVar { token_env } => resolve_env_var(context_name, token_env)?,
                TokenCredentials::Plain { token } => {
                    tracing::warn!(context = %context_name, "plaintext token in config");
                    token.clone()
                }
            };
            Ok(ResolvedAuth::Token { token })
        }
        AuthConfig::Mtls(mtls) => {
            if !mtls.client_identity_path.exists() {
                return ClientIdentityNotFoundSnafu {
                    path: mtls.client_identity_path.clone(),
                }
                .fail();
            }
            let pem = std::fs::read(&mtls.client_identity_path).context(
                ClientIdentityReadFailedSnafu {
                    path: mtls.client_identity_path.clone(),
                },
            )?;
            Ok(ResolvedAuth::Mtls {
                client_identity_pem: pem,
            })
        }
    }
}

fn validate_tls(context: &crate::config::Context) -> Result<(), NifiLensError> {
    if context.insecure_tls && context.ca_cert_path.is_some() {
        tracing::warn!(context = %context.name, "insecure_tls=true; ca_cert_path is ignored");
    }
    if let Some(ca) = &context.ca_cert_path
        && !ca.exists()
    {
        return CaCertNotFoundSnafu { path: ca.clone() }.fail();
    }
    Ok(())
}

fn validate_proxy_urls(context: &crate::config::Context) -> Result<(), NifiLensError> {
    let fields = [
        ("proxy_url", &context.proxy_url),
        ("http_proxy_url", &context.http_proxy_url),
        ("https_proxy_url", &context.https_proxy_url),
    ];
    for (field, value) in fields {
        if let Some(raw) = value {
            url::Url::parse(raw).map_err(|_| NifiLensError::ConfigInvalid {
                detail: format!(
                    "context {:?}: {field} is not a valid URL: {raw:?}",
                    context.name
                ),
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::io::Write;

    fn temp_config_with(contents: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(file.path()).unwrap().permissions();
            perms.set_mode(0o600);
            fs::set_permissions(file.path(), perms).unwrap();
        }
        file
    }

    fn args_for(path: &std::path::Path, context: Option<&str>) -> Args {
        Args {
            config: Some(path.to_path_buf()),
            context: context.map(|s| s.to_string()),
            debug: false,
            log_level: None,
            no_color: false,
            allow_writes: false,
            command: None,
        }
    }

    #[test]
    fn load_missing_file_errors() {
        let args = args_for(std::path::Path::new("/nonexistent/nifilens.toml"), None);
        let err = load(&args).unwrap_err();
        assert!(matches!(err, NifiLensError::ConfigMissing { .. }));
    }

    #[test]
    #[cfg(unix)]
    fn load_world_readable_errors() {
        let file = temp_config_with(
            r#"
current_context = "dev"

[[contexts]]
name = "dev"
url = "https://localhost:8443"

[contexts.auth]
type = "password"
username = "admin"
password_env = "TEST_PW"
"#,
        );
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(file.path()).unwrap().permissions();
        perms.set_mode(0o644);
        fs::set_permissions(file.path(), perms).unwrap();

        let args = args_for(file.path(), None);
        let err = load(&args).unwrap_err();
        assert!(matches!(err, NifiLensError::ConfigWorldReadable { .. }));
    }

    #[test]
    fn load_parse_error_surfaces_path() {
        let file = temp_config_with("this is not = valid = toml");
        let args = args_for(file.path(), None);
        let err = load(&args).unwrap_err();
        assert!(matches!(err, NifiLensError::ConfigParse { .. }));
    }

    #[test]
    fn env_var_credentials_resolve() {
        let file = temp_config_with(
            r#"
current_context = "dev"

[[contexts]]
name = "dev"
url = "https://localhost:8443"

[contexts.auth]
type = "password"
username = "admin"
password_env = "NIFILENS_TEST_PW"
"#,
        );
        // SAFETY: set_var is unsafe in Rust 2024 edition. This test mutates
        // process env, which is racy with parallel tests touching the same
        // var — we use a uniquely-named var to avoid collisions.
        unsafe { std::env::set_var("NIFILENS_TEST_PW", "secret-123") };
        let args = args_for(file.path(), None);
        let (_config, resolved) = load(&args).unwrap();
        match &resolved.auth {
            ResolvedAuth::Password { password, .. } => assert_eq!(password, "secret-123"),
            other => panic!("expected ResolvedAuth::Password, got {other:?}"),
        }
        unsafe { std::env::remove_var("NIFILENS_TEST_PW") };
    }

    #[test]
    fn env_var_missing_errors() {
        let file = temp_config_with(
            r#"
current_context = "dev"

[[contexts]]
name = "dev"
url = "https://localhost:8443"

[contexts.auth]
type = "password"
username = "admin"
password_env = "NIFILENS_DEFINITELY_NOT_SET_XYZ"
"#,
        );
        // SAFETY: ensure the env var is absent; unsafe because Rust 2024.
        unsafe { std::env::remove_var("NIFILENS_DEFINITELY_NOT_SET_XYZ") };
        let args = args_for(file.path(), None);
        let err = load(&args).unwrap_err();
        assert!(matches!(err, NifiLensError::MissingAuthEnvVar { .. }));
    }

    #[test]
    fn plaintext_password_resolves() {
        let file = temp_config_with(
            r#"
current_context = "dev"

[[contexts]]
name = "dev"
url = "https://localhost:8443"

[contexts.auth]
type = "password"
username = "admin"
password = "literal"
"#,
        );
        let args = args_for(file.path(), None);
        let (_, resolved) = load(&args).unwrap();
        match &resolved.auth {
            ResolvedAuth::Password { password, .. } => assert_eq!(password, "literal"),
            other => panic!("expected ResolvedAuth::Password, got {other:?}"),
        }
    }

    #[test]
    fn context_override_from_args() {
        let file = temp_config_with(
            r#"
current_context = "dev"

[[contexts]]
name = "dev"
url = "https://dev:8443"

[contexts.auth]
type = "password"
username = "admin"
password = "x"

[[contexts]]
name = "prod"
url = "https://prod:8443"

[contexts.auth]
type = "password"
username = "admin"
password = "y"
"#,
        );
        let args = args_for(file.path(), Some("prod"));
        let (_, resolved) = load(&args).unwrap();
        assert_eq!(resolved.name, "prod");
        assert_eq!(resolved.url, "https://prod:8443");
    }

    #[test]
    fn unknown_context_errors() {
        let file = temp_config_with(
            r#"
current_context = "dev"

[[contexts]]
name = "dev"
url = "https://dev:8443"

[contexts.auth]
type = "password"
username = "admin"
password = "x"
"#,
        );
        let args = args_for(file.path(), Some("staging"));
        let err = load(&args).unwrap_err();
        match err {
            NifiLensError::UnknownContext { name, available } => {
                assert_eq!(name, "staging");
                assert_eq!(available, vec!["dev".to_string()]);
            }
            other => panic!("expected UnknownContext, got {other:?}"),
        }
    }

    #[test]
    fn bulletins_defaults_when_missing() {
        let file = temp_config_with(
            r#"
current_context = "dev"

[[contexts]]
name = "dev"
url = "https://dev:8443"

[contexts.auth]
type = "password"
username = "admin"
password = "x"
"#,
        );
        let args = args_for(file.path(), None);
        let (config, _) = load(&args).unwrap();
        assert_eq!(config.bulletins.ring_size, 5000);
    }

    #[test]
    fn bulletins_explicit_ring_size_parses() {
        let file = temp_config_with(
            r#"
current_context = "dev"

[bulletins]
ring_size = 2500

[[contexts]]
name = "dev"
url = "https://dev:8443"

[contexts.auth]
type = "password"
username = "admin"
password = "x"
"#,
        );
        let args = args_for(file.path(), None);
        let (config, _) = load(&args).unwrap();
        assert_eq!(config.bulletins.ring_size, 2500);
    }

    #[test]
    fn bulletins_ring_size_below_floor_errors() {
        let file = temp_config_with(
            r#"
current_context = "dev"

[bulletins]
ring_size = 50

[[contexts]]
name = "dev"
url = "https://dev:8443"

[contexts.auth]
type = "password"
username = "admin"
password = "x"
"#,
        );
        let args = args_for(file.path(), None);
        let err = load(&args).unwrap_err();
        match err {
            NifiLensError::ConfigInvalid { detail } => {
                assert!(detail.contains("ring_size"));
                assert!(detail.contains("50"));
            }
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
    }

    #[test]
    fn bulletins_ring_size_above_ceiling_errors() {
        let file = temp_config_with(
            r#"
current_context = "dev"

[bulletins]
ring_size = 500000

[[contexts]]
name = "dev"
url = "https://dev:8443"

[contexts.auth]
type = "password"
username = "admin"
password = "x"
"#,
        );
        let args = args_for(file.path(), None);
        let err = load(&args).unwrap_err();
        assert!(matches!(err, NifiLensError::ConfigInvalid { .. }));
    }

    #[test]
    fn ca_cert_missing_errors() {
        let file = temp_config_with(
            r#"
current_context = "dev"

[[contexts]]
name = "dev"
url = "https://dev:8443"
ca_cert_path = "/definitely/not/here.crt"

[contexts.auth]
type = "password"
username = "admin"
password = "x"
"#,
        );
        let args = args_for(file.path(), None);
        let err = load(&args).unwrap_err();
        assert!(matches!(err, NifiLensError::CaCertNotFound { .. }));
    }

    #[test]
    fn token_env_var_resolves() {
        let file = temp_config_with(
            r#"
current_context = "dev"

[[contexts]]
name = "dev"
url = "https://localhost:8443"

[contexts.auth]
type = "token"
token_env = "NIFILENS_TEST_TOKEN"
"#,
        );
        // SAFETY: unsafe because Rust 2024 edition.
        unsafe { std::env::set_var("NIFILENS_TEST_TOKEN", "bearer-abc") };
        let args = args_for(file.path(), None);
        let (_config, resolved) = load(&args).unwrap();
        match &resolved.auth {
            ResolvedAuth::Token { token } => assert_eq!(token, "bearer-abc"),
            other => panic!("expected ResolvedAuth::Token, got {other:?}"),
        }
        unsafe { std::env::remove_var("NIFILENS_TEST_TOKEN") };
    }

    #[test]
    fn token_env_var_missing_errors() {
        let file = temp_config_with(
            r#"
current_context = "dev"

[[contexts]]
name = "dev"
url = "https://localhost:8443"

[contexts.auth]
type = "token"
token_env = "NIFILENS_TOKEN_DEFINITELY_NOT_SET_XYZ"
"#,
        );
        // SAFETY: ensure the env var is absent; unsafe because Rust 2024.
        unsafe { std::env::remove_var("NIFILENS_TOKEN_DEFINITELY_NOT_SET_XYZ") };
        let args = args_for(file.path(), None);
        let err = load(&args).unwrap_err();
        assert!(matches!(err, NifiLensError::MissingAuthEnvVar { .. }));
    }

    #[test]
    fn mtls_identity_not_found_errors() {
        let file = temp_config_with(
            r#"
current_context = "dev"

[[contexts]]
name = "dev"
url = "https://localhost:8443"

[contexts.auth]
type = "mtls"
client_identity_path = "/definitely/not/here.pem"
"#,
        );
        let args = args_for(file.path(), None);
        let err = load(&args).unwrap_err();
        assert!(matches!(
            err,
            NifiLensError::ClientIdentityNotFound { .. }
                | NifiLensError::ClientIdentityReadFailed { .. }
        ));
    }

    #[test]
    fn proxied_entities_chain_resolves() {
        let file = temp_config_with(
            r#"
current_context = "dev"

[[contexts]]
name = "dev"
url = "https://localhost:8443"
proxied_entities_chain = "CN=proxy.example.com"

[contexts.auth]
type = "password"
username = "admin"
password = "x"
"#,
        );
        let args = args_for(file.path(), None);
        let (_config, resolved) = load(&args).unwrap();
        assert_eq!(
            resolved.proxied_entities_chain.as_deref(),
            Some("CN=proxy.example.com")
        );
    }

    #[test]
    fn proxy_url_resolves() {
        let file = temp_config_with(
            r#"
current_context = "dev"

[[contexts]]
name = "dev"
url = "https://localhost:8443"
proxy_url = "http://proxy.internal:3128"

[contexts.auth]
type = "password"
username = "admin"
password = "x"
"#,
        );
        let args = args_for(file.path(), None);
        let (_config, resolved) = load(&args).unwrap();
        assert_eq!(
            resolved.proxy_url.as_deref(),
            Some("http://proxy.internal:3128")
        );
        assert!(resolved.http_proxy_url.is_none());
        assert!(resolved.https_proxy_url.is_none());
    }

    #[test]
    fn http_and_https_proxy_urls_resolve_independently() {
        let file = temp_config_with(
            r#"
current_context = "dev"

[[contexts]]
name = "dev"
url = "https://localhost:8443"
http_proxy_url  = "http://proxy.internal:3128"
https_proxy_url = "http://secure-proxy.internal:3129"

[contexts.auth]
type = "password"
username = "admin"
password = "x"
"#,
        );
        let args = args_for(file.path(), None);
        let (_config, resolved) = load(&args).unwrap();
        assert!(resolved.proxy_url.is_none());
        assert_eq!(
            resolved.http_proxy_url.as_deref(),
            Some("http://proxy.internal:3128")
        );
        assert_eq!(
            resolved.https_proxy_url.as_deref(),
            Some("http://secure-proxy.internal:3129")
        );
    }

    #[test]
    fn invalid_proxy_url_errors() {
        let file = temp_config_with(
            r#"
current_context = "dev"

[[contexts]]
name = "dev"
url = "https://localhost:8443"
proxy_url = "not a url"

[contexts.auth]
type = "password"
username = "admin"
password = "x"
"#,
        );
        let args = args_for(file.path(), None);
        let err = load(&args).unwrap_err();
        match err {
            NifiLensError::ConfigInvalid { detail } => {
                assert!(detail.contains("proxy_url"));
                assert!(detail.contains("not a url"));
            }
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
    }

    #[test]
    fn invalid_https_proxy_url_names_field() {
        let file = temp_config_with(
            r#"
current_context = "dev"

[[contexts]]
name = "dev"
url = "https://localhost:8443"
https_proxy_url = "://bad"

[contexts.auth]
type = "password"
username = "admin"
password = "x"
"#,
        );
        let args = args_for(file.path(), None);
        let err = load(&args).unwrap_err();
        match err {
            NifiLensError::ConfigInvalid { detail } => {
                assert!(detail.contains("https_proxy_url"));
            }
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
    }
}
