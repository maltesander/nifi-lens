//! Test-only helpers shared across the crate's unit tests.
//!
//! This module is compiled only under `#[cfg(test)]`. It provides
//! `fresh_state()` and `tiny_config()` so tests in any crate module can
//! construct a minimal `AppState` without duplicating the config
//! boilerplate.

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
    AppState::new("dev".into(), Version::new(2, 9, 0), &c)
}

/// Construct a minimal `RootPgStatusSnapshot` for Browser reducer tests
/// that don't care about the aggregate counts. Populates `nodes` with a
/// single root PG (id `"root"`, name `"root"`) and mirrors that id into
/// `process_group_ids` so the connections-by-PG watch channel stays in
/// sync (Task 5 contract).
pub(crate) fn tiny_root_pg_status() -> RootPgStatusSnapshot {
    RootPgStatusSnapshot {
        flow_files_queued: 0,
        bytes_queued: 0,
        connections: Vec::new(),
        process_group_count: 1,
        input_port_count: 0,
        output_port_count: 0,
        processors: ProcessorStateCounts::default(),
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
