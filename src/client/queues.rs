//! Helpers for `/flowfile-queues/{id}/listing-requests` (two-phase listing)
//! and `/flowfile-queues/{id}/flowfiles/{uuid}` (flowfile peek).

use nifi_rust_client::NifiError;
use nifi_rust_client::dynamic::types::{FlowFileDto, ListingRequestDto};

use crate::client::NifiClient;
use crate::error::NifiLensError;

/// Submit a new listing request for `queue_id`.
///
/// Calls `POST /nifi-api/flowfile-queues/{id}/listing-requests`.
/// Returns the initial `ListingRequestDto` (usually `finished: false`).
/// Maps `NifiError` → `NifiLensError::QueueListingFailed`.
pub async fn submit_listing_request(
    client: &NifiClient,
    queue_id: &str,
) -> Result<ListingRequestDto, NifiLensError> {
    client
        .flowfilequeues()
        .create_flow_file_listing(queue_id)
        .await
        .map_err(|err| NifiLensError::QueueListingFailed {
            queue_id: queue_id.to_string(),
            source: Box::new(err),
        })
}

/// Poll an in-flight listing request.
///
/// Calls `GET /nifi-api/flowfile-queues/{id}/listing-requests/{listing-request-id}`.
/// Maps `NifiError` → `NifiLensError::QueueListingPollFailed`.
pub async fn poll_listing_request(
    client: &NifiClient,
    queue_id: &str,
    request_id: &str,
) -> Result<ListingRequestDto, NifiLensError> {
    client
        .flowfilequeues()
        .get_listing_request(queue_id, request_id)
        .await
        .map_err(|err| NifiLensError::QueueListingPollFailed {
            queue_id: queue_id.to_string(),
            request_id: request_id.to_string(),
            source: Box::new(err),
        })
}

/// Cancel / remove a listing request (best-effort cleanup).
///
/// Calls `DELETE /nifi-api/flowfile-queues/{id}/listing-requests/{listing-request-id}`.
/// `NifiError::NotFound` → `Ok(())` (the request was already cleaned up by NiFi).
/// All other errors are logged at `warn!` and also return `Ok(())` — the caller
/// is performing teardown and cannot meaningfully recover.
pub async fn cancel_listing_request(
    client: &NifiClient,
    queue_id: &str,
    request_id: &str,
) -> Result<(), NifiLensError> {
    match client
        .flowfilequeues()
        .delete_listing_request(queue_id, request_id)
        .await
    {
        Ok(_) => Ok(()),
        Err(NifiError::NotFound { .. }) => {
            // Already gone — treat as success.
            Ok(())
        }
        Err(err) => {
            tracing::warn!(
                queue_id,
                request_id,
                error = %err,
                "failed to delete listing request (best-effort; ignoring)"
            );
            Ok(())
        }
    }
}

/// Fetch the full `FlowFileDto` for a single flowfile in a queue.
///
/// Calls `GET /nifi-api/flowfile-queues/{id}/flowfiles/{flowfile-uuid}`.
/// Pass `cluster_node_id` when operating against a clustered NiFi to pin the
/// fetch to the node that holds the content claim.
/// Maps `NifiError` → `NifiLensError::FlowfilePeekFailed`.
pub async fn get_flowfile(
    client: &NifiClient,
    queue_id: &str,
    flowfile_uuid: &str,
    cluster_node_id: Option<&str>,
) -> Result<FlowFileDto, NifiLensError> {
    client
        .flowfilequeues()
        .get_flow_file(queue_id, flowfile_uuid, cluster_node_id)
        .await
        .map_err(|err| NifiLensError::FlowfilePeekFailed {
            queue_id: queue_id.to_string(),
            flowfile_uuid: flowfile_uuid.to_string(),
            source: Box::new(err),
        })
}

/// Convert an optional millisecond count from NiFi into a `Duration`.
///
/// `Some(v)` where `v > 0` → `Duration::from_millis(v as u64)`.
/// `Some(0)`, `Some(negative)`, and `None` → `Duration::ZERO`.
pub(crate) fn ms_to_duration(ms: Option<i64>) -> std::time::Duration {
    match ms {
        Some(v) if v > 0 => std::time::Duration::from_millis(v as u64),
        _ => std::time::Duration::ZERO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn test_client(server: &MockServer) -> Arc<RwLock<NifiClient>> {
        Mock::given(method("GET"))
            .and(path("/nifi-api/flow/about"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!(
                {"about": {"version": "2.6.0", "title": "NiFi"}}
            )))
            .mount(server)
            .await;
        let inner = nifi_rust_client::NifiClientBuilder::new(&server.uri())
            .expect("builder")
            .build_dynamic()
            .expect("dynamic");
        inner.detect_version().await.expect("detect_version");
        let version = semver::Version::parse("2.6.0").expect("parse");
        Arc::new(RwLock::new(NifiClient::from_parts(inner, "test", version)))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn submit_listing_request_returns_dto() {
        let server = MockServer::start().await;
        let client = test_client(&server).await;

        Mock::given(method("POST"))
            .and(path("/nifi-api/flowfile-queues/q1/listing-requests"))
            .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
                "listingRequest": {
                    "id": "req-1",
                    "state": "WAITING",
                    "percentCompleted": 0,
                    "finished": false
                }
            })))
            .mount(&server)
            .await;

        let guard = client.read().await;
        let dto = submit_listing_request(&guard, "q1").await.expect("ok");
        assert_eq!(dto.id, Some("req-1".to_string()));
        assert_eq!(dto.state, Some("WAITING".to_string()));
        assert_eq!(dto.finished, Some(false));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn poll_listing_request_returns_finished_state() {
        let server = MockServer::start().await;
        let client = test_client(&server).await;

        Mock::given(method("GET"))
            .and(path("/nifi-api/flowfile-queues/q1/listing-requests/req-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "listingRequest": {
                    "id": "req-1",
                    "state": "FINISHED",
                    "percentCompleted": 100,
                    "finished": true,
                    "flowFileSummaries": [
                        {
                            "uuid": "ff-aaaa",
                            "filename": "a.parquet",
                            "size": 1024,
                            "position": 1,
                            "penalized": false,
                            "queuedDuration": 5000,
                            "lineageDuration": 60000,
                            "clusterNodeId": null
                        }
                    ]
                }
            })))
            .mount(&server)
            .await;

        let guard = client.read().await;
        let dto = poll_listing_request(&guard, "q1", "req-1")
            .await
            .expect("ok");
        assert_eq!(dto.finished, Some(true));
        let summaries = dto.flow_file_summaries.expect("summaries present");
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].uuid, Some("ff-aaaa".to_string()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancel_listing_request_204_is_ok() {
        let server = MockServer::start().await;
        let client = test_client(&server).await;

        Mock::given(method("DELETE"))
            .and(path("/nifi-api/flowfile-queues/q1/listing-requests/req-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "listingRequest": {
                    "id": "req-1",
                    "state": "WAITING",
                    "percentCompleted": 0,
                    "finished": false
                }
            })))
            .mount(&server)
            .await;

        let guard = client.read().await;
        cancel_listing_request(&guard, "q1", "req-1")
            .await
            .expect("ok");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancel_listing_request_404_is_swallowed() {
        let server = MockServer::start().await;
        let client = test_client(&server).await;

        Mock::given(method("DELETE"))
            .and(path("/nifi-api/flowfile-queues/q1/listing-requests/req-1"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "message": "not found"
            })))
            .mount(&server)
            .await;

        let guard = client.read().await;
        // Must return Ok(()) even on 404 — best-effort cleanup.
        cancel_listing_request(&guard, "q1", "req-1")
            .await
            .expect("404 is swallowed");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn get_flowfile_threads_cluster_node_id() {
        let server = MockServer::start().await;
        let client = test_client(&server).await;

        Mock::given(method("GET"))
            .and(path("/nifi-api/flowfile-queues/q1/flowfiles/ff-aaaa"))
            .and(query_param("clusterNodeId", "node-7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "flowFile": {
                    "uuid": "ff-aaaa",
                    "filename": "a.parquet",
                    "size": 1024,
                    "position": 1,
                    "penalized": false,
                    "attributes": {
                        "record.count": "1000"
                    }
                }
            })))
            .mount(&server)
            .await;

        let guard = client.read().await;
        let dto = get_flowfile(&guard, "q1", "ff-aaaa", Some("node-7"))
            .await
            .expect("ok");
        let attrs = dto.attributes.expect("attributes present");
        assert_eq!(
            attrs.get("record.count").and_then(|v| v.as_deref()),
            Some("1000")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn get_flowfile_omits_node_param_when_none() {
        let server = MockServer::start().await;
        let client = test_client(&server).await;

        Mock::given(method("GET"))
            .and(path("/nifi-api/flowfile-queues/q1/flowfiles/ff-bbbb"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "flowFile": {
                    "uuid": "ff-bbbb",
                    "filename": "b.json",
                    "size": 512,
                    "position": 2,
                    "penalized": false,
                    "attributes": {}
                }
            })))
            .mount(&server)
            .await;

        let guard = client.read().await;
        get_flowfile(&guard, "q1", "ff-bbbb", None)
            .await
            .expect("ok");

        // Verify no clusterNodeId query param was sent.
        let received = server.received_requests().await.expect("requests");
        // Filter out the /flow/about request used by test_client.
        let flowfile_reqs: Vec<_> = received
            .iter()
            .filter(|r| r.url.path().contains("/flowfiles/"))
            .collect();
        assert_eq!(flowfile_reqs.len(), 1, "exactly one flowfile request");
        let has_node_param = flowfile_reqs[0]
            .url
            .query_pairs()
            .any(|(k, _)| k == "clusterNodeId");
        assert!(
            !has_node_param,
            "clusterNodeId must not be present when cluster_node_id is None"
        );
    }

    #[test]
    fn ms_to_duration_positive() {
        assert_eq!(
            ms_to_duration(Some(5000)),
            std::time::Duration::from_millis(5000)
        );
    }

    #[test]
    fn ms_to_duration_zero_and_negative_and_none() {
        assert_eq!(ms_to_duration(Some(0)), std::time::Duration::ZERO);
        assert_eq!(ms_to_duration(Some(-1)), std::time::Duration::ZERO);
        assert_eq!(ms_to_duration(None), std::time::Duration::ZERO);
    }
}
