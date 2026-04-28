//! Helpers for `/flow/history` (action audit) and `/flow/{type}/{id}/status/history`
//! (sparkline series). PR 1 of the browser-forensics work covers action history;
//! status-history support is added in PR 2.

use std::pin::Pin;

use nifi_rust_client::NifiError;
use nifi_rust_client::pagination::{
    HistoryFilter, HistoryPage, HistoryPaginator, flow_history_dynamic,
};

/// Local alias for the future type used by upstream's
/// `flow_history_dynamic` closure. Upstream's `BoxedFetchFuture` alias is
/// crate-private, so the wrapper signature spells the equivalent shape
/// out by hand.
type BoxedHistoryFuture<'a> =
    Pin<Box<dyn core::future::Future<Output = Result<HistoryPage, NifiError>> + Send + 'a>>;

/// Component kinds that participate in the Browser detail panes. Used by
/// the action-history modal to label the source on rows and (in PR 2) to
/// dispatch status-history fetches per kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComponentKind {
    Processor,
    ProcessGroup,
    Connection,
    ControllerService,
    Port,
}

impl ComponentKind {
    /// Short display label used in the action-history modal's `type` column.
    pub fn label(self) -> &'static str {
        match self {
            Self::Processor => "Processor",
            Self::ProcessGroup => "ProcessGroup",
            Self::Connection => "Connection",
            Self::ControllerService => "ControllerService",
            Self::Port => "Port",
        }
    }
}

/// Build a `HistoryPaginator` filtered by `source_id`. Returns the
/// paginator unchanged so the caller drives paging themselves.
///
/// Callers must hold the `DynamicClient` reference for the paginator's
/// lifetime — `next_page` borrows the inner client.
pub fn flow_actions_paginator<'a>(
    client: &'a nifi_rust_client::dynamic::DynamicClient,
    source_id: &str,
    page_size: u32,
) -> HistoryPaginator<impl FnMut(u32, u32) -> BoxedHistoryFuture<'a> + 'a> {
    let filter = HistoryFilter {
        source_id: Some(source_id.to_string()),
        ..HistoryFilter::default()
    };
    flow_history_dynamic(client, filter, page_size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::client::NifiClient;

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

    fn action(id: i32, source_id: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "sourceId": source_id,
            "timestamp": "2026-04-27T10:00:00Z",
            "action": {
                "id": id,
                "operation": "Configure",
                "sourceId": source_id,
                "sourceName": "test",
                "sourceType": "Processor",
                "userIdentity": "alice",
                "timestamp": "2026-04-27T10:00:00Z"
            }
        })
    }

    #[tokio::test(flavor = "current_thread")]
    async fn paginator_filters_by_source_id_and_stitches_pages() {
        let server = MockServer::start().await;
        // Page 1: 2 actions out of 3 total.
        Mock::given(method("GET"))
            .and(path("/nifi-api/flow/history"))
            .and(query_param("offset", "0"))
            .and(query_param("count", "2"))
            .and(query_param("sourceId", "proc-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "history": {
                    "total": 3,
                    "actions": [action(10, "proc-1"), action(11, "proc-1")]
                }
            })))
            .mount(&server)
            .await;
        // Page 2: 1 action remaining.
        Mock::given(method("GET"))
            .and(path("/nifi-api/flow/history"))
            .and(query_param("offset", "2"))
            .and(query_param("count", "2"))
            .and(query_param("sourceId", "proc-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "history": {
                    "total": 3,
                    "actions": [action(12, "proc-1")]
                }
            })))
            .mount(&server)
            .await;

        let client_arc = test_client(&server).await;
        let guard = client_arc.read().await;
        // NifiClient: Deref<Target = DynamicClient>, so &guard auto-derefs.
        let mut p = flow_actions_paginator(&guard, "proc-1", 2);
        let page1 = p.next_page().await.expect("page1").expect("some");
        assert_eq!(page1.len(), 2);
        assert_eq!(page1[0].id, Some(10));
        let page2 = p.next_page().await.expect("page2").expect("some");
        assert_eq!(page2.len(), 1);
        assert_eq!(page2[0].id, Some(12));
        let exhausted = p.next_page().await.expect("exhausted");
        assert!(exhausted.is_none(), "third page must be None");
    }

    #[test]
    fn component_kind_labels() {
        assert_eq!(ComponentKind::Processor.label(), "Processor");
        assert_eq!(ComponentKind::Port.label(), "Port");
    }
}
