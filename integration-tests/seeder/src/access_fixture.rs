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
    // /data-transfer/input-ports/{id} write is added per-port from
    // fixture::orders::remote_targets via grant_data_transfer_policy as
    // each public input port is created — NiFi rejects a wildcard
    // /data-transfer/input-ports policy ("An unexpected type of resource").

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

    // NiFi's /data policy inheritance walks one level up (component →
    // parent PG); it does NOT walk further up the PG hierarchy. So a
    // single /data/process-groups/{root} policy doesn't cover deeply
    // nested processors / connections. Explicitly grant admin /data on
    // every PG under the marker so the component-→-PG inheritance has
    // somewhere to land. Without this, queue-listing requests return
    // 403 ("Unable to view the data for Processor X") even though the
    // /policies effective lookup says admin has access.
    grant_admin_data_recursively(client, &admin_id, &marker_id).await?;
    // The /data/process-groups inheritance is one-level only. In a
    // 2-node cluster the queue-listing two-stage commit re-checks
    // /data/processors/{id} on each node and our parent-PG policy
    // doesn't cascade — grant admin /data on every processor + every
    // connection under the marker explicitly. Bounded ~50 components.
    grant_admin_component_data_recursively(client, &admin_id, &marker_id).await?;

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

/// Grants admin + the cluster-node identity (CN=localhost) write
/// access to `/data-transfer/input-ports/{port_id}`. Called from
/// `fixture::orders::remote_targets` immediately after each public
/// input port is created.
///
/// Required because the `/site-to-site` controller listing filters
/// public input ports by `Write - /data-transfer/input-ports/{id}` —
/// without this policy CN=localhost sees zero remote input ports during
/// the RPG handshake even though it has read on the port itself via
/// /process-groups/{root} inheritance. NiFi does not support a
/// wildcard policy on /data-transfer/input-ports, so the policy must
/// be created per-port.
pub async fn grant_data_transfer_policy(client: &DynamicClient, port_id: &str) -> Result<()> {
    let admin_id = lookup_user_id(client, "admin").await?;
    let node_id = lookup_user_id(client, "CN=localhost").await?;
    ensure_policy(
        client,
        "write",
        &format!("/data-transfer/input-ports/{port_id}"),
        &[&admin_id, &node_id],
    )
    .await
}

/// Same shape as `create_policy` but no-ops if the (action, resource)
/// already exists *as an explicit policy on `resource`*. NiFi's
/// `GET /policies/{action}/{resource}` returns the *effective* policy,
/// which may be an inherited parent's policy — distinguishing requires
/// comparing `component.resource` to the requested resource.
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
    if let Ok(entity) = existing {
        let effective_resource = entity
            .component
            .as_ref()
            .and_then(|c| c.resource.as_deref())
            .unwrap_or("");
        if effective_resource == resource {
            tracing::info!(action, resource, "policy already present; skipping");
            return Ok(());
        }
        tracing::info!(
            action,
            resource,
            inherited_from = effective_resource,
            "creating explicit policy (effective is inherited)",
        );
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

/// Walk every PG under `pg_id` (depth-first) and ensure admin has
/// /data/process-groups/{id} R+W on each. The orders-pipeline and
/// versioned-clean PGs are skipped — `seed` adds those itself with
/// the appropriate fixture identities (admin + ops-team / admin + bob).
async fn grant_admin_data_recursively(
    client: &DynamicClient,
    admin_id: &str,
    pg_id: &str,
) -> Result<()> {
    let listing = client
        .processgroups()
        .get_process_groups(pg_id)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("GET /process-groups/{pg_id}/process-groups for admin /data walk"),
            source: Box::new(e),
        })?;
    let groups = listing.process_groups.unwrap_or_default();
    for child in groups {
        let child_id = child
            .component
            .as_ref()
            .and_then(|c| c.id.clone())
            .or_else(|| child.id.clone());
        let child_name = child
            .component
            .as_ref()
            .and_then(|c| c.name.clone())
            .unwrap_or_default();
        let Some(child_id) = child_id else { continue };
        // orders-pipeline + versioned-clean get fixture-specific
        // policies further down in `seed`; the recursive walk would
        // otherwise plant admin-only policies here that those calls
        // would 409-collide with.
        if child_name != ORDERS_PG_NAME && child_name != VERSIONED_CLEAN_PG_NAME {
            ensure_policy(
                client,
                "read",
                &format!("/data/process-groups/{child_id}"),
                &[admin_id],
            )
            .await?;
            ensure_policy(
                client,
                "write",
                &format!("/data/process-groups/{child_id}"),
                &[admin_id],
            )
            .await?;
        }
        Box::pin(grant_admin_data_recursively(client, admin_id, &child_id)).await?;
    }
    Ok(())
}

/// Walk every processor + connection under `pg_id` (depth-first) and
/// grant admin /data/processors/{id} R+W and /data/connections/{id} R+W.
/// NiFi's /data inheritance only walks one level (component → parent PG)
/// so even with /data/process-groups/{X} R+W, processors INSIDE a
/// child PG of X aren't covered.
async fn grant_admin_component_data_recursively(
    client: &DynamicClient,
    admin_id: &str,
    pg_id: &str,
) -> Result<()> {
    let processors = client
        .processgroups()
        .get_processors(pg_id, None)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("GET /process-groups/{pg_id}/processors"),
            source: Box::new(e),
        })?;
    for proc in processors.processors.unwrap_or_default() {
        let proc_id = proc
            .component
            .as_ref()
            .and_then(|c| c.id.clone())
            .or_else(|| proc.id.clone());
        if let Some(id) = proc_id {
            ensure_policy(
                client,
                "read",
                &format!("/data/processors/{id}"),
                &[admin_id],
            )
            .await?;
            ensure_policy(
                client,
                "write",
                &format!("/data/processors/{id}"),
                &[admin_id],
            )
            .await?;
        }
    }

    let connections = client
        .processgroups()
        .get_connections(pg_id)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("GET /process-groups/{pg_id}/connections"),
            source: Box::new(e),
        })?;
    for conn in connections.connections.unwrap_or_default() {
        let conn_id = conn
            .component
            .as_ref()
            .and_then(|c| c.id.clone())
            .or_else(|| conn.id.clone());
        if let Some(id) = conn_id {
            ensure_policy(
                client,
                "read",
                &format!("/data/connections/{id}"),
                &[admin_id],
            )
            .await?;
            ensure_policy(
                client,
                "write",
                &format!("/data/connections/{id}"),
                &[admin_id],
            )
            .await?;
        }
    }

    let listing = client
        .processgroups()
        .get_process_groups(pg_id)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("GET /process-groups/{pg_id}/process-groups for /data walk"),
            source: Box::new(e),
        })?;
    for child in listing.process_groups.unwrap_or_default() {
        let child_id = child
            .component
            .as_ref()
            .and_then(|c| c.id.clone())
            .or_else(|| child.id.clone());
        let Some(child_id) = child_id else { continue };
        Box::pin(grant_admin_component_data_recursively(
            client, admin_id, &child_id,
        ))
        .await?;
    }
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
