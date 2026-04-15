//! Test-only helpers shared across the crate's unit tests.
//!
//! This module is compiled only under `#[cfg(test)]`. It provides
//! `fresh_state()` and `tiny_config()` so tests in any crate module can
//! construct a minimal `AppState` without duplicating the config
//! boilerplate.

use semver::Version;

use crate::app::state::AppState;
use crate::config::{
    AuthConfig, Config, Context, PasswordAuthConfig, PasswordCredentials, VersionStrategy,
};

/// Construct a minimal `Config` suitable for reducer and widget tests.
pub(crate) fn tiny_config() -> Config {
    Config {
        current_context: "dev".into(),
        bulletins: Default::default(),
        ui: Default::default(),
        contexts: vec![Context {
            name: "dev".into(),
            url: "https://dev:8443".into(),
            auth: AuthConfig::Password(PasswordAuthConfig {
                username: "admin".into(),
                credentials: PasswordCredentials::Plain {
                    password: "x".into(),
                },
            }),
            version_strategy: VersionStrategy::Strict,
            insecure_tls: false,
            ca_cert_path: None,
            proxied_entities_chain: None,
            proxy_url: None,
            http_proxy_url: None,
            https_proxy_url: None,
        }],
    }
}

/// Construct a fresh `AppState` for widget- and integration-level tests.
pub(crate) fn fresh_state() -> AppState {
    let c = tiny_config();
    AppState::new("dev".into(), Version::new(2, 9, 0), &c)
}
