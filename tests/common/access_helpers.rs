//! Helpers for the access-modal live integration test.
//!
//! Looks up fixture-specific PG and user IDs against a live NiFi via
//! the `nifi-rust-client` dynamic API. Used by
//! `tests/integration_browser_access.rs`.

// Each integration test binary recompiles `tests/common/mod.rs`; the
// other binaries don't use these helpers, so without this allow they
// emit dead-code warnings.
#![allow(dead_code)]

use nifi_lens::client::NifiClient;

/// Look up a process group's UUID by name. Searches root + one level of
/// nested PGs (sufficient for the orders-pipeline / versioned-clean
/// fixture layout, which lives under the `nifilens-fixture-v8` marker).
pub async fn lookup_pg_id_by_name(client: &NifiClient, name: &str) -> String {
    let root = client
        .processgroups()
        .get_process_groups("root")
        .await
        .expect("list root child PGs");
    let groups = root.process_groups.unwrap_or_default();

    if let Some(id) = pg_id_by_name(&groups, name) {
        return id;
    }
    for parent in &groups {
        let Some(parent_id) = parent
            .component
            .as_ref()
            .and_then(|c| c.id.clone())
            .or_else(|| parent.id.clone())
        else {
            continue;
        };
        let listing = client
            .processgroups()
            .get_process_groups(&parent_id)
            .await
            .expect("list child PGs");
        let inner = listing.process_groups.unwrap_or_default();
        if let Some(id) = pg_id_by_name(&inner, name) {
            return id;
        }
    }
    panic!("PG {name} not found in fixture");
}

fn pg_id_by_name(
    groups: &[nifi_rust_client::dynamic::types::ProcessGroupEntity],
    name: &str,
) -> Option<String> {
    let pg = groups
        .iter()
        .find(|pg| pg.component.as_ref().and_then(|c| c.name.as_deref()) == Some(name))?;
    pg.component
        .as_ref()
        .and_then(|c| c.id.clone())
        .or_else(|| pg.id.clone())
}

/// Look up a user's UUID by identity string.
pub async fn lookup_user_id_by_identity(client: &NifiClient, identity: &str) -> String {
    let entity = client
        .tenants()
        .get_users()
        .await
        .expect("GET /tenants/users");
    entity
        .users
        .unwrap_or_default()
        .into_iter()
        .find(|u| u.component.as_ref().and_then(|c| c.identity.as_deref()) == Some(identity))
        .and_then(|u| u.component.and_then(|c| c.id).or(u.id))
        .unwrap_or_else(|| panic!("user {identity} not found in /tenants/users"))
}
