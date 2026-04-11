//! DynamicClient construction with TLS and version-strategy options.

use nifi_rust_client::NifiClientBuilder;
use nifi_rust_client::dynamic::{DynamicClient, VersionResolutionStrategy};
use snafu::ResultExt as _;

use crate::client::classify_or_fallback;
use crate::config::{ResolvedContext, VersionStrategy};
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
        // The library only exposes danger_accept_invalid_certs — there is no
        // separate danger_accept_invalid_hostnames method in 0.5.0.
        builder = builder.danger_accept_invalid_certs(true);
    } else if let Some(ca_path) = &ctx.ca_cert_path {
        let pem = std::fs::read(ca_path).context(CaCertReadFailedSnafu {
            path: ca_path.clone(),
        })?;
        // add_root_certificate is infallible in 0.5.0 — it returns Self, no Result.
        builder = builder.add_root_certificate(&pem);
    }
    // Otherwise: system trust store is the default.

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
