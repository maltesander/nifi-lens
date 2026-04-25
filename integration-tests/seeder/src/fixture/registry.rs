//! Registry-client + bucket setup. Idempotent — safe to re-run on a
//! pre-seeded fixture (`--skip-if-seeded` short-circuits before this
//! anyway, but the underlying operations are idempotent regardless).

use std::collections::HashMap;

use nifi_rust_client::dynamic::DynamicClient;

use crate::error::{Result, SeederError};

/// In-Docker URL for NiFi → Registry. Service-to-service via Docker DNS.
const REGISTRY_URL_FOR_NIFI: &str = "http://nifi-registry:18080";

/// Host-side URL for the seeder → Registry. Port-mapped from the Docker container.
const REGISTRY_URL_FOR_SEEDER: &str = "http://localhost:18080";

const REGISTRY_CLIENT_NAME: &str = "local-registry";
const BUCKET_NAME: &str = "nifilens-fixture";

/// Result of registry setup: caller uses these ids when committing
/// flows to the registry.
pub struct RegistryIds {
    pub client_id: String,
    pub bucket_id: String,
}

/// Idempotent registry setup: ensure a NiFi-side registry-client points at
/// `http://nifi-registry:18080`, and a `nifilens-fixture` bucket exists in
/// the registry. Returns the registry-client id and bucket id so callers
/// can commit flows.
pub async fn seed(client: &DynamicClient) -> Result<RegistryIds> {
    tracing::info!("ensuring NiFi-side registry-client");
    let client_id = ensure_registry_client(client).await?;
    tracing::info!("ensuring NiFi-Registry-side bucket");
    let bucket_id = ensure_bucket().await?;
    Ok(RegistryIds {
        client_id,
        bucket_id,
    })
}

async fn ensure_registry_client(client: &DynamicClient) -> Result<String> {
    let existing = client
        .controller()
        .get_flow_registry_clients()
        .await
        .map_err(|e| SeederError::Api {
            message: "list flow registry clients".into(),
            source: Box::new(e),
        })?;
    if let Some(found) = existing.registries.as_ref().and_then(|list| {
        list.iter().find(|entity| {
            entity
                .component
                .as_ref()
                .and_then(|c| c.name.as_ref())
                .map(|n| n == REGISTRY_CLIENT_NAME)
                .unwrap_or(false)
        })
    }) {
        let id = found.id.clone().ok_or_else(|| SeederError::Invariant {
            message: "registry-client found by name but has no id".into(),
        })?;
        tracing::info!(%id, "registry-client already present");
        return Ok(id);
    }

    // Discover the NifiRegistryFlowRegistryClient bundle so we don't
    // hardcode a NAR version.
    let types = client
        .controller()
        .get_registry_client_types()
        .await
        .map_err(|e| SeederError::Api {
            message: "list registry client types".into(),
            source: Box::new(e),
        })?;
    let entry = types
        .flow_registry_client_types
        .as_ref()
        .and_then(|list| {
            list.iter().find(|t| {
                t.r#type
                    .as_ref()
                    .map(|s| s.contains("NifiRegistryFlowRegistryClient"))
                    .unwrap_or(false)
            })
        })
        .ok_or_else(|| SeederError::Invariant {
            message: "no NifiRegistryFlowRegistryClient type advertised".into(),
        })?;

    let mut props = HashMap::new();
    props.insert("url".to_string(), Some(REGISTRY_URL_FOR_NIFI.to_string()));

    let mut dto = nifi_rust_client::dynamic::types::FlowRegistryClientDto::default();
    dto.name = Some(REGISTRY_CLIENT_NAME.to_string());
    dto.r#type = entry.r#type.clone();
    dto.bundle = entry.bundle.clone();
    dto.properties = Some(props);

    let mut revision = nifi_rust_client::dynamic::types::RevisionDto::default();
    revision.client_id = Some("nifilens-fixture-seeder".to_string());
    revision.version = Some(0);

    let mut body = nifi_rust_client::dynamic::types::FlowRegistryClientEntity::default();
    body.component = Some(dto);
    body.revision = Some(revision);

    let created = client
        .controller()
        .create_flow_registry_client(&body)
        .await
        .map_err(|e| SeederError::Api {
            message: "create flow registry client".into(),
            source: Box::new(e),
        })?;
    let id = created.id.clone().ok_or_else(|| SeederError::Invariant {
        message: "created registry-client has no id".into(),
    })?;
    tracing::info!(%id, name = %REGISTRY_CLIENT_NAME, "created registry-client");
    Ok(id)
}

async fn ensure_bucket() -> Result<String> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| SeederError::Api {
            message: "build registry HTTP client".into(),
            source: Box::new(e),
        })?;

    let list_url = format!("{REGISTRY_URL_FOR_SEEDER}/nifi-registry-api/buckets");
    let response = http
        .get(&list_url)
        .send()
        .await
        .map_err(|e| SeederError::Api {
            message: format!("GET {list_url}"),
            source: Box::new(e),
        })?;
    response
        .error_for_status_ref()
        .map_err(|e| SeederError::Api {
            message: format!("GET {list_url} failed"),
            source: Box::new(e),
        })?;
    let buckets: Vec<serde_json::Value> = response.json().await.map_err(|e| SeederError::Api {
        message: format!("parse JSON from {list_url}"),
        source: Box::new(e),
    })?;
    if let Some(existing) = buckets
        .iter()
        .find(|b| b.get("name").and_then(|n| n.as_str()) == Some(BUCKET_NAME))
    {
        let id = existing
            .get("identifier")
            .and_then(|s| s.as_str())
            .ok_or_else(|| SeederError::Invariant {
                message: "bucket found but has no identifier".into(),
            })?
            .to_string();
        tracing::info!(%id, name = %BUCKET_NAME, "bucket already present");
        return Ok(id);
    }

    let create_url = format!("{REGISTRY_URL_FOR_SEEDER}/nifi-registry-api/buckets");
    let body = serde_json::json!({"name": BUCKET_NAME});
    let response = http
        .post(&create_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| SeederError::Api {
            message: format!("POST {create_url}"),
            source: Box::new(e),
        })?;
    response
        .error_for_status_ref()
        .map_err(|e| SeederError::Api {
            message: format!("POST {create_url} failed"),
            source: Box::new(e),
        })?;
    let created: serde_json::Value = response.json().await.map_err(|e| SeederError::Api {
        message: format!("parse JSON from {create_url}"),
        source: Box::new(e),
    })?;
    let id = created
        .get("identifier")
        .and_then(|s| s.as_str())
        .ok_or_else(|| SeederError::Invariant {
            message: "created bucket has no identifier".into(),
        })?
        .to_string();
    tracing::info!(%id, name = %BUCKET_NAME, "created bucket");
    Ok(id)
}
