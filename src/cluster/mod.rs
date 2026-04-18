//! Central cluster-state store. See
//! `docs/superpowers/specs/2026-04-18-central-cluster-store-design.md`.

pub mod config;
pub mod fetcher;
pub mod fetcher_tasks;
pub mod snapshot;
pub mod store;
pub mod subscriber;

pub use config::ClusterPollingConfig;
pub use snapshot::{ClusterSnapshot, EndpointState, FetchMeta};
pub use store::{ClusterStore, ClusterUpdate, SubscriberId};
pub use subscriber::SubscriberRegistry;

/// Identifies a cluster-wide endpoint managed by `ClusterStore`.
/// The discriminant is stable and used as an `AtomicUsize` index into
/// `SubscriberRegistry`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ClusterEndpoint {
    About = 0,
    ControllerStatus = 1,
    RootPgStatus = 2,
    ControllerServices = 3,
    SystemDiagnostics = 4,
    ConnectionsByPg = 5,
    Bulletins = 6,
}

impl ClusterEndpoint {
    /// Total count — used to size fixed arrays like
    /// `[AtomicUsize; ClusterEndpoint::COUNT]`.
    pub const COUNT: usize = 7;

    pub fn as_str(self) -> &'static str {
        match self {
            Self::About => "about",
            Self::ControllerStatus => "controller_status",
            Self::RootPgStatus => "root_pg_status",
            Self::ControllerServices => "controller_services",
            Self::SystemDiagnostics => "system_diagnostics",
            Self::ConnectionsByPg => "connections_by_pg",
            Self::Bulletins => "bulletins",
        }
    }
}

impl std::fmt::Display for ClusterEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_count_matches_variants() {
        // Guard: bumping this requires also updating the SubscriberRegistry
        // fixed-size arrays.
        assert_eq!(ClusterEndpoint::COUNT, 7);
    }

    #[test]
    fn endpoint_as_str_is_stable() {
        assert_eq!(ClusterEndpoint::RootPgStatus.as_str(), "root_pg_status");
        assert_eq!(
            format!("{}", ClusterEndpoint::ControllerServices),
            "controller_services"
        );
    }
}
