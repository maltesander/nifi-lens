//! High-level NiFi client wrapper used by nifi-lens.
//!
//! The wrapper owns a `nifi_rust_client::dynamic::DynamicClient`, the
//! originating context's name, and the version that the library detected at
//! login time. The wrapped client is exposed via `Deref` so callers can write
//! `client.flow_api().get_about_info()` without an explicit accessor.

pub mod build;

use std::ops::{Deref, DerefMut};

use nifi_rust_client::NifiError;
use nifi_rust_client::dynamic::{DynamicClient, traits::FlowApi as _};
use semver::Version;

use crate::config::ResolvedContext;
use crate::error::NifiLensError;

/// Try to classify a boxed library error into a specific `NifiLensError`
/// variant with a targeted hint, falling back to a caller-provided
/// generic constructor when no specific match is found.
///
/// Downcasts the boxed source to `nifi_rust_client::NifiError` and matches
/// on the variant. Unclassified variants (network errors, 5xx responses,
/// etc.) pass through to `fallback`.
pub(crate) fn classify_or_fallback(
    context: &str,
    source: Box<dyn std::error::Error + Send + Sync>,
    fallback: impl FnOnce(Box<dyn std::error::Error + Send + Sync>) -> NifiLensError,
) -> NifiLensError {
    if let Some(nifi_err) = source.downcast_ref::<NifiError>() {
        match nifi_err {
            NifiError::UnsupportedVersion { detected } => {
                return NifiLensError::UnsupportedNifiVersion {
                    context: context.to_string(),
                    detected: detected.clone(),
                };
            }
            NifiError::InvalidCertificate { .. } => {
                return NifiLensError::TlsCertInvalid {
                    context: context.to_string(),
                    source,
                };
            }
            NifiError::Unauthorized { .. } | NifiError::Auth { .. } => {
                return NifiLensError::NifiUnauthorized {
                    context: context.to_string(),
                };
            }
            _ => {}
        }
    }
    fallback(source)
}

pub struct NifiClient {
    inner: DynamicClient,
    context_name: String,
    detected_version: Version,
}

impl std::fmt::Debug for NifiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // DynamicClient does not implement Debug; we emit just the fields we own.
        f.debug_struct("NifiClient")
            .field("context_name", &self.context_name)
            .field("detected_version", &self.detected_version)
            .finish_non_exhaustive()
    }
}

impl Deref for NifiClient {
    type Target = DynamicClient;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for NifiClient {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl NifiClient {
    /// Build, login, detect version, and return a connected client.
    pub async fn connect(ctx: &ResolvedContext) -> Result<Self, NifiLensError> {
        tracing::debug!(context = %ctx.name, url = %ctx.url, "connecting");

        let inner = build::build_dynamic_client(ctx)?;

        // DynamicClient::login is &self (not &mut self) and also triggers
        // version detection automatically.
        inner
            .login(&ctx.username, &ctx.password)
            .await
            .map_err(|err| {
                classify_or_fallback(&ctx.name, Box::new(err), |source| {
                    NifiLensError::LoginFailed {
                        context: ctx.name.clone(),
                        source,
                    }
                })
            })?;

        // detected_version() returns DetectedVersion (an enum), not a String.
        // DetectedVersion implements Display with semver format ("2.8.0" etc).
        let version_str = inner.detected_version().to_string();
        let detected_version =
            Version::parse(&version_str).map_err(|err| NifiLensError::LoginFailed {
                context: ctx.name.clone(),
                source: Box::new(err),
            })?;

        Ok(Self {
            inner,
            context_name: ctx.name.clone(),
            detected_version,
        })
    }

    pub fn context_name(&self) -> &str {
        &self.context_name
    }

    pub fn detected_version(&self) -> &Version {
        &self.detected_version
    }

    /// Convenience wrapper around `flow_api().get_about_info()` that maps
    /// the error into `NifiLensError`.
    pub async fn about(&self) -> Result<AboutSnapshot, NifiLensError> {
        tracing::debug!(context = %self.context_name, "fetching /flow/about");
        let about = self
            .inner
            .flow_api()
            .get_about_info()
            .await
            .map_err(|err| {
                classify_or_fallback(&self.context_name, Box::new(err), |source| {
                    NifiLensError::AboutFailed {
                        context: self.context_name.clone(),
                        source,
                    }
                })
            })?;

        Ok(AboutSnapshot {
            version: about.version.clone().unwrap_or_default(),
            title: about.title.clone().unwrap_or_default(),
        })
    }
}

/// Snapshot of the `/flow/about` endpoint used by the identity strip.
#[derive(Debug, Clone, Default)]
pub struct AboutSnapshot {
    pub version: String,
    pub title: String,
}
