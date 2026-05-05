//! Seeds the access-control side of the fixture: creates the
//! `ops-team` group, then attaches realistic component-level policies
//! to fixture PGs. Runs after `fixture::seed` because it depends on
//! the orders-pipeline + versioned-clean PG IDs being present.
//!
//! Identities admin/alice/bob/carol are created automatically by NiFi
//! at startup from `authorizers.xml`'s `Initial User Identity` properties.
//! This module never creates them.

use nifi_rust_client::dynamic::DynamicClient;
use nifi_rust_client::dynamic::types;

use crate::error::{Result, SeederError};
use crate::marker::FIXTURE_MARKER_NAME;

const GROUP_NAME: &str = "ops-team";
const ORDERS_PG_NAME: &str = "orders-pipeline";
const VERSIONED_CLEAN_PG_NAME: &str = "versioned-clean";

/// Grants admin the per-root-PG policies that NiFi's Initial Admin
/// bootstrap doesn't create in clustered mode (the root PG UUID isn't
/// known when the FileAccessPolicyProvider initializes — only `/flow`,
/// `/controller`, `/tenants`, `/policies`, `/restricted-components`,
/// and `/proxy` exist). Without these, admin's nuke-and-repave fails
/// with 403 on the very first `GET /process-groups/root/...` call.
///
/// Runs BEFORE `cleanup::nuke_and_repave` and `fixture::seed`.
pub async fn bootstrap_admin_policies(client: &DynamicClient) -> Result<()> {
    tracing::info!("bootstrapping admin per-PG policies");

    let admin_id = lookup_user_id(client, "admin").await?;
    // Node identity user — needs /site-to-site read so RPGs across the
    // 2.6.0 ↔ 2.9.0 fixtures can complete the S2S handshake (the orders
    // RPGs target nifi-2-6-0 from both clusters, and inter-node API
    // federation also goes through this path).
    let node_id = lookup_user_id(client, "CN=localhost").await?;
    let root_id = lookup_root_pg_id(client).await?;

    // CN=localhost is on the /process-groups/{root} read policy so the
    // S2S handshake can enumerate public input ports nested inside the
    // remote-targets PG; the RPG reads via the node identity, not
    // admin's bearer token.
    let read_pg_users: &[&str] = &[&admin_id, &node_id];
    let write_pg_users: &[&str] = &[&admin_id];
    ensure_policy(
        client,
        "read",
        &format!("/process-groups/{root_id}"),
        read_pg_users,
    )
    .await?;
    ensure_policy(
        client,
        "write",
        &format!("/process-groups/{root_id}"),
        write_pg_users,
    )
    .await?;
    ensure_policy(
        client,
        "read",
        &format!("/data/process-groups/{root_id}"),
        &[&admin_id],
    )
    .await?;
    ensure_policy(
        client,
        "write",
        &format!("/data/process-groups/{root_id}"),
        &[&admin_id],
    )
    .await?;
    ensure_policy(
        client,
        "write",
        &format!("/operate/process-groups/{root_id}"),
        &[&admin_id],
    )
    .await?;
    ensure_policy(client, "read", "/parameter-contexts", &[&admin_id]).await?;
    ensure_policy(client, "write", "/parameter-contexts", &[&admin_id]).await?;
    ensure_policy(client, "read", "/provenance", &[&admin_id]).await?;
    ensure_policy(client, "read", "/provenance-data", &[&admin_id]).await?;
    ensure_policy(
        client,
        "read",
        &format!("/provenance-data/process-groups/{root_id}"),
        &[&admin_id],
    )
    .await?;
    ensure_policy(client, "read", "/counters", &[&admin_id]).await?;
    ensure_policy(client, "write", "/counters", &[&admin_id]).await?;
    ensure_policy(client, "read", "/system", &[&admin_id]).await?;
    // Required for the RPG → site-to-site handshake the orders-pipeline
    // fixture relies on for input port discovery. The node identity is
    // listed alongside admin because the cross-cluster RPG presents the
    // shared keystore cert (CN=localhost), not admin's bearer token.
    ensure_policy(client, "read", "/site-to-site", &[&admin_id, &node_id]).await?;

    tracing::info!("admin bootstrap policies complete");
    Ok(())
}

pub async fn seed(client: &DynamicClient) -> Result<()> {
    tracing::info!("seeding access-control fixture (ops-team + component policies)");

    let admin_id = lookup_user_id(client, "admin").await?;
    let alice_id = lookup_user_id(client, "alice").await?;
    let bob_id = lookup_user_id(client, "bob").await?;
    let carol_id = lookup_user_id(client, "carol").await?;

    let ops_team_id = lookup_or_create_group(client, GROUP_NAME, &[&alice_id, &carol_id]).await?;

    let marker_id = lookup_child_pg_id_by_name(client, "root", FIXTURE_MARKER_NAME).await?;
    let orders_pg_id = lookup_child_pg_id_by_name(client, &marker_id, ORDERS_PG_NAME).await?;
    let versioned_clean_pg_id =
        lookup_child_pg_id_by_name(client, &marker_id, VERSIONED_CLEAN_PG_NAME).await?;

    // admin is included in every fixture-side policy so the next
    // nuke-and-repave pass can empty queues + delete child PGs without
    // hitting 403 from ops-team / bob-only policies. Forensic semantics
    // are unchanged — admin already has cluster-wide access via the
    // bootstrap policies.

    create_policy(
        client,
        "read",
        &format!("/process-groups/{orders_pg_id}"),
        &[&ops_team_id],
        &[&admin_id],
    )
    .await?;
    create_policy(
        client,
        "write",
        &format!("/process-groups/{orders_pg_id}"),
        &[&ops_team_id],
        &[&admin_id],
    )
    .await?;
    create_policy(
        client,
        "read",
        &format!("/data/process-groups/{orders_pg_id}"),
        &[&ops_team_id],
        &[&admin_id],
    )
    .await?;
    create_policy(
        client,
        "write",
        &format!("/data/process-groups/{orders_pg_id}"),
        &[],
        &[&admin_id],
    )
    .await?;
    create_policy(
        client,
        "write",
        &format!("/operate/process-groups/{orders_pg_id}"),
        &[&ops_team_id],
        &[&admin_id],
    )
    .await?;

    create_policy(
        client,
        "read",
        &format!("/process-groups/{versioned_clean_pg_id}"),
        &[],
        &[&bob_id, &admin_id],
    )
    .await?;
    create_policy(
        client,
        "write",
        &format!("/process-groups/{versioned_clean_pg_id}"),
        &[],
        &[&admin_id],
    )
    .await?;
    create_policy(
        client,
        "read",
        &format!("/data/process-groups/{versioned_clean_pg_id}"),
        &[],
        &[&bob_id, &admin_id],
    )
    .await?;
    create_policy(
        client,
        "write",
        &format!("/data/process-groups/{versioned_clean_pg_id}"),
        &[],
        &[&admin_id],
    )
    .await?;

    tracing::info!("access-control fixture seed complete");
    Ok(())
}

async fn lookup_user_id(client: &DynamicClient, identity: &str) -> Result<String> {
    let entity = client
        .tenants()
        .get_users()
        .await
        .map_err(|e| SeederError::Api {
            message: "GET /tenants/users".into(),
            source: Box::new(e),
        })?;
    let users = entity.users.unwrap_or_default();
    let found = users
        .into_iter()
        .find(|u| u.component.as_ref().and_then(|c| c.identity.as_deref()) == Some(identity));
    let user = found.ok_or_else(|| SeederError::Invariant {
        message: format!(
            "user {identity} missing from /tenants/users \
             (auto-bootstrap should have created it)"
        ),
    })?;
    user.component
        .and_then(|c| c.id)
        .or(user.id)
        .ok_or_else(|| SeederError::Invariant {
            message: format!("user {identity} has no id"),
        })
}

/// Idempotent: returns the existing group ID if `name` already exists,
/// otherwise creates the group with `member_user_ids` and returns the
/// new ID. A pre-existing group is left untouched (members not synced).
async fn lookup_or_create_group(
    client: &DynamicClient,
    name: &str,
    member_user_ids: &[&str],
) -> Result<String> {
    let entity = client
        .tenants()
        .get_user_groups()
        .await
        .map_err(|e| SeederError::Api {
            message: "GET /tenants/user-groups".into(),
            source: Box::new(e),
        })?;
    let existing = entity
        .user_groups
        .unwrap_or_default()
        .into_iter()
        .find(|g| g.component.as_ref().and_then(|c| c.identity.as_deref()) == Some(name));
    if let Some(g) = existing {
        let id = g
            .component
            .as_ref()
            .and_then(|c| c.id.clone())
            .or(g.id)
            .ok_or_else(|| SeederError::Invariant {
                message: format!("existing group {name} has no id"),
            })?;
        tracing::info!(group = name, %id, "group already exists; reusing");
        return Ok(id);
    }

    let users: Vec<types::TenantEntity> = member_user_ids
        .iter()
        .map(|id| {
            let mut t = types::TenantEntity::default();
            t.id = Some((*id).to_string());
            t
        })
        .collect();

    let mut component = types::UserGroupDto::default();
    component.identity = Some(name.to_string());
    component.users = Some(users);

    let mut revision = types::RevisionDto::default();
    revision.version = Some(0);

    let mut body = types::UserGroupEntity::default();
    body.component = Some(component);
    body.revision = Some(revision);

    let created = client
        .tenants()
        .create_user_group(&body)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("POST /tenants/user-groups (name={name})"),
            source: Box::new(e),
        })?;
    let id = created
        .component
        .as_ref()
        .and_then(|c| c.id.clone())
        .or(created.id)
        .ok_or_else(|| SeederError::Invariant {
            message: format!("created user-group {name} has no id"),
        })?;
    tracing::info!(group = name, %id, "group created");
    Ok(id)
}

/// Same shape as `create_policy` but no-ops if the (action, resource)
/// already exists. Used by `bootstrap_admin_policies`, which may run
/// against a partially-bootstrapped cluster.
async fn ensure_policy(
    client: &DynamicClient,
    action: &str,
    resource: &str,
    user_ids: &[&str],
) -> Result<()> {
    // The policy *resource* path conventionally includes a leading slash
    // (`/process-groups/{id}`), but NiFi's `GET /policies/{action}/{resource}`
    // URL template inlines the resource directly — so a leading slash
    // produces `//` and Jetty rejects with "Ambiguous URI empty segment".
    // Strip the leading slash for the URL only; the create body keeps it.
    let resource_for_lookup = resource.trim_start_matches('/');
    let existing = client
        .policies()
        .get_access_policy_for_resource(action, resource_for_lookup)
        .await;
    if existing.is_ok() {
        tracing::info!(action, resource, "policy already present; skipping");
        return Ok(());
    }
    create_policy(client, action, resource, &[], user_ids).await
}

async fn lookup_root_pg_id(client: &DynamicClient) -> Result<String> {
    let entity = client
        .flow()
        .get_flow("root", None)
        .await
        .map_err(|e| SeederError::Api {
            message: "GET /flow/process-groups/root".into(),
            source: Box::new(e),
        })?;
    entity
        .process_group_flow
        .and_then(|pgf| pgf.id)
        .ok_or_else(|| SeederError::Invariant {
            message: "root PG id missing from /flow/process-groups/root response".into(),
        })
}

async fn create_policy(
    client: &DynamicClient,
    action: &str,
    resource: &str,
    group_ids: &[&str],
    user_ids: &[&str],
) -> Result<()> {
    let groups: Vec<types::TenantEntity> = group_ids
        .iter()
        .map(|id| {
            let mut t = types::TenantEntity::default();
            t.id = Some((*id).to_string());
            t
        })
        .collect();
    let users: Vec<types::TenantEntity> = user_ids
        .iter()
        .map(|id| {
            let mut t = types::TenantEntity::default();
            t.id = Some((*id).to_string());
            t
        })
        .collect();

    let mut component = types::AccessPolicyDto::default();
    component.action = Some(action.to_string());
    component.resource = Some(resource.to_string());
    component.user_groups = Some(groups);
    component.users = Some(users);

    let mut revision = types::RevisionDto::default();
    revision.version = Some(0);

    let mut body = types::AccessPolicyEntity::default();
    body.component = Some(component);
    body.revision = Some(revision);

    client
        .policies()
        .create_access_policy(&body)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("POST /policies {action} {resource}"),
            source: Box::new(e),
        })?;
    tracing::info!(action, resource, "access policy created");
    Ok(())
}

async fn lookup_child_pg_id_by_name(
    client: &DynamicClient,
    parent_pg_id: &str,
    name: &str,
) -> Result<String> {
    let listing = client
        .processgroups()
        .get_process_groups(parent_pg_id)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("GET /process-groups/{parent_pg_id}/process-groups"),
            source: Box::new(e),
        })?;
    let groups = listing.process_groups.unwrap_or_default();
    let pg = groups
        .into_iter()
        .find(|pg| pg.component.as_ref().and_then(|c| c.name.as_deref()) == Some(name))
        .ok_or_else(|| SeederError::Invariant {
            message: format!("PG {name} not found under {parent_pg_id}"),
        })?;
    pg.component
        .as_ref()
        .and_then(|c| c.id.clone())
        .or(pg.id)
        .ok_or_else(|| SeederError::Invariant {
            message: format!("PG {name} has no id"),
        })
}
