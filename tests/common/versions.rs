//! Integration test helpers for looping over the supported NiFi versions.
//!
//! `FIXTURE_VERSIONS` is generated at compile time by `build.rs` from
//! `integration-tests/versions.toml`. Adding a new version requires editing
//! that file AND adding a match arm to `port_for` below (and optionally
//! updating `docker-compose.yml` if the compose file is how the container
//! is booted).

include!(concat!(env!("OUT_DIR"), "/fixture_versions.rs"));

/// Local host port that the given NiFi version is exposed on by
/// `integration-tests/docker-compose.yml`. Panics on an unknown version so
/// that adding a new version to `versions.toml` without updating this match
/// arm fails loudly at test time.
pub fn port_for(version: &str) -> u16 {
    match version {
        "2.6.0" => 8443,
        "2.9.0" => 8444,
        other => panic!(
            "unknown fixture version {other}: add a port_for match arm in \
             tests/common/versions.rs"
        ),
    }
}

/// Context name used in `integration-tests/nifilens-config.toml` for the
/// given version. Converts `2.6.0` → `dev-nifi-2-6-0`.
pub fn context_for(version: &str) -> String {
    format!("dev-nifi-{}", version.replace('.', "-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_versions_is_non_empty() {
        assert!(
            !FIXTURE_VERSIONS.is_empty(),
            "FIXTURE_VERSIONS must have at least one version"
        );
    }

    #[test]
    fn fixture_versions_contains_pinned_floor() {
        assert!(
            FIXTURE_VERSIONS.contains(&"2.6.0"),
            "2.6.0 is the pinned floor and must always be present"
        );
    }

    #[test]
    fn context_for_converts_dots_to_dashes() {
        assert_eq!(context_for("2.6.0"), "dev-nifi-2-6-0");
        assert_eq!(context_for("2.9.0"), "dev-nifi-2-9-0");
    }

    #[test]
    fn port_for_known_versions() {
        assert_eq!(port_for("2.6.0"), 8443);
        assert_eq!(port_for("2.9.0"), 8444);
    }

    #[test]
    #[should_panic(expected = "unknown fixture version")]
    fn port_for_unknown_panics() {
        let _ = port_for("99.99.99");
    }
}
