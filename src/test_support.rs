//! Test-only helpers shared across the crate's unit tests.
//!
//! This module is compiled only under `#[cfg(test)]`. It provides
//! `fresh_state()` and `tiny_config()` so tests in any crate module can
//! construct a minimal `AppState` without duplicating the config
//! boilerplate.

use std::time::Duration;

use ratatui::backend::TestBackend;
use semver::Version;

use crate::app::state::AppState;
use crate::client::{
    ControllerServiceMember, ControllerServicesSnapshot, ProcessorStateCounts,
    RootPgStatusSnapshot,
    browser::{NodeKind, NodeStatusSummary, RawNode},
};
use crate::config::{
    AuthConfig, Config, Context, PasswordAuthConfig, PasswordCredentials, VersionStrategy,
};

/// Construct a minimal `Config` suitable for reducer and widget tests.
pub(crate) fn tiny_config() -> Config {
    Config {
        current_context: "dev".into(),
        browser: Default::default(),
        bulletins: Default::default(),
        ui: Default::default(),
        polling: Default::default(),
        tracer: Default::default(),
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
    AppState::new(
        "dev".into(),
        Version::new(2, 9, 0),
        &c,
        "https://nifi.test:8443".into(),
    )
}

/// Construct a minimal `RootPgStatusSnapshot` for Browser reducer tests
/// that don't care about the aggregate counts. Populates `nodes` with a
/// single root PG (id `"root"`, name `"root"`) and mirrors that id into
/// `process_group_ids` so the connections-by-PG watch channel stays in
/// sync.
pub(crate) fn tiny_root_pg_status() -> RootPgStatusSnapshot {
    RootPgStatusSnapshot {
        flow_files_queued: 0,
        bytes_queued: 0,
        connections: Vec::new(),
        process_group_count: 1,
        input_port_count: 0,
        output_port_count: 0,
        processors: ProcessorStateCounts::default(),
        remote_process_groups: crate::client::RemoteProcessGroupCounts::default(),
        process_group_ids: vec!["root".into()],
        nodes: vec![RawNode {
            parent_idx: None,
            kind: NodeKind::ProcessGroup,
            id: "root".into(),
            group_id: String::new(),
            name: "root".into(),
            status_summary: NodeStatusSummary::ProcessGroup {
                running: 0,
                stopped: 0,
                invalid: 0,
                disabled: 0,
            },
        }],
    }
}

/// Construct a minimal `ControllerServicesSnapshot` with the given
/// members. The counts tally follows the same invariant rules as the
/// production builder.
pub(crate) fn tiny_controller_services(
    members: Vec<ControllerServiceMember>,
) -> ControllerServicesSnapshot {
    ControllerServicesSnapshot {
        counts: Default::default(),
        members,
    }
}

/// Filler value for the `fetch_duration` field on synthetic
/// `ClusterUpdate`s built in tests. Tests never read this value
/// back — it's there only because the struct requires it.
pub(crate) fn default_fetch_duration() -> Duration {
    Duration::from_millis(5)
}

/// Common `TestBackend` dimensions used across view snapshot tests.
/// Named for readability; use them when spinning up a backend for
/// Bulletins / Browser / Events / Tracer screens.
pub(crate) const TEST_BACKEND_WIDTH: u16 = 100;
pub(crate) const TEST_BACKEND_SHORT: u16 = 20;
pub(crate) const TEST_BACKEND_MEDIUM: u16 = 24;
pub(crate) const TEST_BACKEND_TALL: u16 = 28;

/// Shorthand for `TestBackend::new` at the standard width.
pub(crate) fn test_backend(height: u16) -> TestBackend {
    TestBackend::new(TEST_BACKEND_WIDTH, height)
}
