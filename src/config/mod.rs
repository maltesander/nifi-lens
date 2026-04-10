//! Configuration types and loader for nifi-lens.

pub mod loader;

use std::path::PathBuf;

use serde::Deserialize;

/// Root of the user's config file.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Config {
    pub current_context: String,
    #[serde(default)]
    pub contexts: Vec<Context>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Context {
    pub name: String,
    pub url: String,
    pub username: String,
    #[serde(flatten)]
    pub credentials: Credentials,
    #[serde(default)]
    pub version_strategy: VersionStrategy,
    #[serde(default)]
    pub insecure_tls: bool,
    #[serde(default)]
    pub ca_cert_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Credentials {
    EnvVar { password_env: String },
    Plain { password: String },
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VersionStrategy {
    #[default]
    Strict,
    Closest,
    Latest,
}

/// A context with credentials resolved to a plaintext password. Produced by
/// `loader::load` after consulting env vars / warning on plaintext.
#[derive(Debug, Clone)]
pub struct ResolvedContext {
    pub name: String,
    pub url: String,
    pub username: String,
    pub password: String,
    pub version_strategy: VersionStrategy,
    pub insecure_tls: bool,
    pub ca_cert_path: Option<PathBuf>,
}
