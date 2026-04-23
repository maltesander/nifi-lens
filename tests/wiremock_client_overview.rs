//! Wiremock tests: Phase 1 overview client wrappers.

use nifi_lens::client::NifiClient;
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn stub_login_and_about(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/nifi-api/access/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string("stub-jwt-token"))
        .mount(server)
        .await;
    let about = serde_json::json!({
        "about": {
            "version": "2.8.0",
            "title": "NiFi",
            "uri": server.uri(),
        }
    });
    Mock::given(method("GET"))
        .and(path("/nifi-api/flow/about"))
        .respond_with(ResponseTemplate::new(200).set_body_json(about))
        .mount(server)
        .await;
}

fn ctx(url: String) -> ResolvedContext {
    ResolvedContext {
        name: "wiremock".into(),
        url,
        auth: ResolvedAuth::Password {
            username: "admin".into(),
            password: "anything".into(),
        },
        version_strategy: VersionStrategy::Closest,
        insecure_tls: true,
        ca_cert_path: None,
        proxied_entities_chain: None,
        proxy_url: None,
        http_proxy_url: None,
        https_proxy_url: None,
    }
}

#[tokio::test]
async fn controller_status_returns_counts() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    let body = serde_json::json!({
        "controllerStatus": {
            "activeThreadCount": 3,
            "runningCount": 12,
            "stoppedCount": 4,
            "invalidCount": 1,
            "disabledCount": 2,
            "flowFilesQueued": 55,
            "bytesQueued": 4096,
        }
    });
    Mock::given(method("GET"))
        .and(path("/nifi-api/flow/status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let status = client.controller_status().await.expect("ok");
    assert_eq!(status.running, 12);
    assert_eq!(status.stopped, 4);
    assert_eq!(status.invalid, 1);
    assert_eq!(status.disabled, 2);
    assert_eq!(status.active_threads, 3);
    assert_eq!(status.flow_files_queued, 55);
    assert_eq!(status.bytes_queued, 4096);
}

#[tokio::test]
async fn root_pg_status_flattens_connections() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    // A minimal recursive PG tree: root has one child processor and one
    // connection, and one child PG that itself has another connection.
    let body = serde_json::json!({
        "processGroupStatus": {
            "id": "root",
            "name": "NiFi Flow",
            "aggregateSnapshot": {
                "id": "root",
                "name": "NiFi Flow",
                "flowFilesQueued": 500,
                "bytesQueued": 1024,
                "connectionStatusSnapshots": [
                    {
                        "id": "conn-a",
                        "connectionStatusSnapshot": {
                            "id": "conn-a",
                            "name": "noisy queue",
                            "groupId": "root",
                            "sourceName": "Generate",
                            "destinationName": "Consume",
                            "percentUseCount": 95,
                            "percentUseBytes": 42,
                            "flowFilesQueued": 9500,
                            "bytesQueued": 1048576,
                            "queued": "9,500 / 1 MB"
                        }
                    }
                ],
                "processGroupStatusSnapshots": [
                    {
                        "id": "child-pg",
                        "processGroupStatusSnapshot": {
                            "id": "child-pg",
                            "name": "child",
                            "connectionStatusSnapshots": [
                                {
                                    "id": "conn-b",
                                    "connectionStatusSnapshot": {
                                        "id": "conn-b",
                                        "name": "small queue",
                                        "groupId": "child-pg",
                                        "sourceName": "Tag",
                                        "destinationName": "Route",
                                        "percentUseCount": 5,
                                        "percentUseBytes": 1,
                                        "flowFilesQueued": 12,
                                        "bytesQueued": 1234,
                                        "queued": "12 / 1.2 KB"
                                    }
                                }
                            ]
                        }
                    }
                ]
            }
        }
    });
    Mock::given(method("GET"))
        .and(path("/nifi-api/flow/process-groups/root/status"))
        .and(query_param("recursive", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let snapshot = client.root_pg_status().await.expect("ok");
    assert_eq!(snapshot.connections.len(), 2);
    // Sorted descending by max(percent_use_count, percent_use_bytes).
    assert_eq!(snapshot.connections[0].id, "conn-a");
    assert_eq!(snapshot.connections[0].fill_percent, 95);
    assert_eq!(snapshot.connections[0].flow_files_queued, 9500);
    assert_eq!(snapshot.connections[1].id, "conn-b");
    assert_eq!(snapshot.connections[1].fill_percent, 5);
    assert_eq!(snapshot.flow_files_queued, 500);
    assert_eq!(snapshot.bytes_queued, 1024);
}

#[tokio::test]
async fn bulletin_board_returns_bulletins() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    let body = serde_json::json!({
        "bulletinBoard": {
            "bulletins": [
                {
                    "id": 101,
                    "groupId": "root",
                    "sourceId": "proc-1",
                    "bulletin": {
                        "id": 101,
                        "category": "Log Message",
                        "level": "ERROR",
                        "message": "boom",
                        "sourceId": "proc-1",
                        "sourceName": "FailingProcessor",
                        "sourceType": "PROCESSOR",
                        "groupId": "root",
                        "timestamp": "10:14:22 UTC",
                        "timestampIso": "2026-04-11T10:14:22.123Z"
                    }
                },
                {
                    "id": 102,
                    "bulletin": {
                        "id": 102,
                        "level": "WARN",
                        "message": "hiccup",
                        "sourceId": "proc-2",
                        "sourceName": "NoisyProcessor",
                        "sourceType": "PROCESSOR",
                        "timestampIso": "2026-04-11T10:14:23.000Z"
                    }
                }
            ],
            "generated": "10:14:23 UTC"
        }
    });
    Mock::given(method("GET"))
        .and(path("/nifi-api/flow/bulletin-board"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let board = client.bulletin_board(None, Some(50)).await.expect("ok");
    assert_eq!(board.bulletins.len(), 2);
    assert_eq!(board.bulletins[0].id, 101);
    assert_eq!(board.bulletins[0].level, "ERROR");
    assert_eq!(board.bulletins[0].source_name, "FailingProcessor");
    assert_eq!(board.bulletins[0].timestamp_iso, "2026-04-11T10:14:22.123Z");
}

/// NiFi < 2.7.2 omits `timestampIso` on `BulletinDTO` and ships
/// `timestamp` as time-only ("HH:MM:SS UTC"). The client must synthesize
/// an ISO string by combining the time with today's UTC date so
/// downstream consumers (Bulletins tab time column, Overview histogram,
/// detail modal) render uniformly across versions.
#[tokio::test]
async fn bulletin_board_synthesizes_iso_from_legacy_time_only() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    let body = serde_json::json!({
        "bulletinBoard": {
            "bulletins": [
                {
                    "id": 7,
                    "groupId": "root",
                    "sourceId": "proc-legacy",
                    "bulletin": {
                        "id": 7,
                        "level": "WARN",
                        "message": "legacy-format",
                        "sourceId": "proc-legacy",
                        "sourceName": "LegacyProcessor",
                        "sourceType": "PROCESSOR",
                        "groupId": "root",
                        "timestamp": "12:13:58 UTC"
                    }
                }
            ],
            "generated": "12:13:58 UTC"
        }
    });
    Mock::given(method("GET"))
        .and(path("/nifi-api/flow/bulletin-board"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let board = client.bulletin_board(None, Some(50)).await.expect("ok");
    assert_eq!(board.bulletins.len(), 1);
    let iso = &board.bulletins[0].timestamp_iso;
    assert!(
        iso.ends_with("T12:13:58.000Z"),
        "expected synthesized ISO ending in T12:13:58.000Z; got {iso:?}"
    );
    // Shape: YYYY-MM-DDTHH:MM:SS.mmmZ (24 chars).
    assert_eq!(iso.len(), 24, "expected 24-char ISO-8601; got {iso:?}");
    // `timestamp_human` is preserved verbatim — downstream code may rely
    // on the raw server string for display fallbacks.
    assert_eq!(board.bulletins[0].timestamp_human, "12:13:58 UTC");
}
