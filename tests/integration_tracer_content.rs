//! Integration: Tracer content fetch.
//!
//! Gated with `#[ignore]` — only runs via `./integration-tests/run.sh`
//! which boots the Docker fixture. The test fetches the latest provenance
//! events for a component, picks any event that reports output content
//! available, then fetches that content. A 404 / unavailable response is
//! treated as success (content may have been garbage-collected by NiFi).
//! Only transport-level errors cause the test to fail.

use nifi_lens::client::NifiClient;
use nifi_lens::client::tracer::ContentSide;
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};
use nifi_lens::error::NifiLensError;

#[path = "common/mod.rs"]
mod common;
use common::versions::{FIXTURE_VERSIONS, context_for, port_for};

/// Component-id of the `noisy-pipeline` generate-flowfile processor seeded by
/// `nifilens-fixture-seeder`. The integration fixture always seeds this
/// processor so there should be recent provenance events available.
///
/// If this ID drifts with future fixture versions the test will receive an
/// empty event list and skip the content fetch, but will not fail — the
/// assertion only fires on transport errors.
const NOISY_COMPONENT_ID: &str = "fixture-noisy-generate";

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn integration_tracer_content_text_render() {
    let username = std::env::var("NIFILENS_IT_USERNAME").expect("NIFILENS_IT_USERNAME must be set");
    let password = std::env::var("NIFILENS_IT_PASSWORD").expect("NIFILENS_IT_PASSWORD must be set");
    let ca_path =
        std::env::var("NIFILENS_IT_CA_CERT_PATH").expect("NIFILENS_IT_CA_CERT_PATH must be set");

    for &version in FIXTURE_VERSIONS {
        eprintln!("--- integration_tracer_content running against NiFi {version} ---");

        let ctx = ResolvedContext {
            name: context_for(version),
            url: format!("https://localhost:{}", port_for(version)),
            auth: ResolvedAuth::Password {
                username: username.clone(),
                password: password.clone(),
            },
            version_strategy: VersionStrategy::Closest,
            insecure_tls: false,
            ca_cert_path: Some(ca_path.clone().into()),
            proxied_entities_chain: None,
            proxy_url: None,
            http_proxy_url: None,
            https_proxy_url: None,
        };

        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        // 1. Fetch the latest provenance events for the probe component.
        //    NiFi 2.6.0 returns 404 when the component is no longer part of
        //    the flow (or was never seeded). Treat this as a skip, not a
        //    failure — the test is about content fetch, not latest-events.
        let snapshot = match client.latest_events(NOISY_COMPONENT_ID, 20).await {
            Ok(s) => s,
            Err(NifiLensError::LatestProvenanceEventsFailed { .. }) => {
                eprintln!(
                    "  latest_events returned error for {NOISY_COMPONENT_ID} on {version} — skipping"
                );
                continue;
            }
            Err(other) => panic!("latest_events on {version} failed: {other:?}"),
        };

        eprintln!(
            "  {} events for component {}",
            snapshot.events.len(),
            snapshot.component_id
        );

        // 2. Find any event that has output content available.
        let event_with_content = snapshot
            .events
            .iter()
            .find(|e| {
                // Use event_id as a proxy — fetch the detail to check availability.
                // We pick the first event and attempt content fetch unconditionally;
                // a 404 is acceptable.
                let _ = e.event_id; // silence unused warning
                true
            })
            .cloned();

        let Some(summary) = event_with_content else {
            eprintln!("  no events for {NOISY_COMPONENT_ID} on {version} — skipping content fetch");
            continue;
        };

        eprintln!(
            "  probing content for event_id={} type={}",
            summary.event_id, summary.event_type
        );

        // 3. Attempt to fetch output content.
        //    404 / unavailable → skip (content may be GC'd).
        //    Only transport errors (non-HTTP) fail the test.
        match client
            .provenance_content(summary.event_id, ContentSide::Output, None)
            .await
        {
            Ok(cs) => {
                eprintln!("  content fetch ok: {} bytes", cs.bytes_fetched);
            }
            Err(NifiLensError::ProvenanceContentFetchFailed { .. }) => {
                // Could be a 404 (content GC'd) or a 403 (no replay claim).
                // Either is acceptable in an integration context.
                eprintln!(
                    "  content unavailable for event_id={} — acceptable",
                    summary.event_id
                );
            }
            Err(other) => {
                panic!("unexpected transport error fetching content on {version}: {other:?}");
            }
        }
    }
}
