//! DynamicClient construction with TLS, auth, and version-strategy options.

use nifi_rust_client::NifiClientBuilder;
use nifi_rust_client::config::auth::{PasswordAuth, StaticTokenAuth};
use nifi_rust_client::dynamic::{DynamicClient, VersionResolutionStrategy};
use snafu::ResultExt as _;

use crate::client::classify_or_fallback;
use crate::config::{ResolvedAuth, ResolvedContext, VersionStrategy};
use crate::error::{CaCertReadFailedSnafu, NifiLensError};

pub fn build_dynamic_client(ctx: &ResolvedContext) -> Result<DynamicClient, NifiLensError> {
    let mut builder = NifiClientBuilder::new(&ctx.url).map_err(|err| {
        classify_or_fallback(&ctx.name, Box::new(err), |source| {
            NifiLensError::ClientBuildFailed {
                context: ctx.name.clone(),
                source,
            }
        })
    })?;

    // TLS configuration.
    if ctx.insecure_tls {
        tracing::warn!(context = %ctx.name, "insecure TLS enabled; server certificate not verified");
        builder = builder.danger_accept_invalid_certs(true);
    } else if let Some(ca_path) = &ctx.ca_cert_path {
        let pem = std::fs::read(ca_path).context(CaCertReadFailedSnafu {
            path: ca_path.clone(),
        })?;
        builder = builder.add_root_certificate(&pem);
    }

    // mTLS client identity.
    if let ResolvedAuth::Mtls {
        ref client_identity_pem,
    } = ctx.auth
    {
        builder = builder
            .client_identity_pem(client_identity_pem)
            .map_err(|err| {
                classify_or_fallback(&ctx.name, Box::new(err), |source| {
                    NifiLensError::ClientBuildFailed {
                        context: ctx.name.clone(),
                        source,
                    }
                })
            })?;
    }

    // Proxied entities chain.
    if let Some(ref chain) = ctx.proxied_entities_chain {
        builder = builder.proxied_entities_chain(chain);
    }

    // Auth provider (password and token; mTLS authenticates via TLS handshake).
    match &ctx.auth {
        ResolvedAuth::Password { username, password } => {
            builder = builder.auth_provider(PasswordAuth::new(username, password));
        }
        ResolvedAuth::Token { token } => {
            builder = builder.auth_provider(StaticTokenAuth::new(token));
        }
        ResolvedAuth::Mtls { .. } => {
            // mTLS uses the client cert for auth — no auth provider needed.
        }
    }

    builder = builder.version_strategy(map_version_strategy(ctx.version_strategy));

    builder.build_dynamic().map_err(|err| {
        classify_or_fallback(&ctx.name, Box::new(err), |source| {
            NifiLensError::ClientBuildFailed {
                context: ctx.name.clone(),
                source,
            }
        })
    })
}

fn map_version_strategy(s: VersionStrategy) -> VersionResolutionStrategy {
    match s {
        VersionStrategy::Strict => VersionResolutionStrategy::Strict,
        VersionStrategy::Closest => VersionResolutionStrategy::Closest,
        VersionStrategy::Latest => VersionResolutionStrategy::Latest,
    }
}
