//! Parameter context fixture for the orders-pipeline narrative.
//!
//! Five contexts in a 3-tier inheritance hierarchy:
//!
//! - `fixture-pc-platform`             (root parent)
//! - `fixture-pc-orders`               (inherits platform)
//! - `fixture-pc-region-eu/-us/-apac`  (each inherits orders)
//!
//! Bound by `orders/mod.rs` to specific PGs:
//!   ingest      → fixture-pc-platform     (chain depth 1)
//!   transform   → fixture-pc-orders       (chain depth 2)
//!   sink-eu     → fixture-pc-region-eu    (chain depth 3)
//!   sink-us     → fixture-pc-region-us    (chain depth 3)
//!   sink-apac   → fixture-pc-region-apac  (chain depth 3)
//!
//! `transform`'s `usd_rate` parameter is the one mutated to "oops" in
//! `orders::break::apply_break` — this is the headline failure narrative.

use nifi_rust_client::dynamic::{DynamicClient, types};

use crate::error::{Result, SeederError};

/// IDs of every parameter context this module creates. Used by `orders/`
/// modules to bind PGs and by `orders::break` to PUT the mutation.
pub struct OrdersContextIds {
    pub platform_id: String,
    pub orders_id: String,
    pub region_eu_id: String,
    pub region_us_id: String,
    pub region_apac_id: String,
}

/// Initial healthy value of `usd_rate`. `break_::apply_break` mutates this
/// through a three-step sequence (clear → `INTERMEDIATE_USD_RATE` →
/// `BROKEN_USD_RATE`) so the action-history modal shows a "developer
/// struggled and left it broken" narrative rather than a single mutation.
pub const HEALTHY_USD_RATE: &str = "1.0827";

/// Mid-sequence "tested rate" set on step 2 of the three-step break. Numeric
/// on purpose — during the brief window between step 2 and step 3 the
/// pipeline runs cleanly with a real (if wrong) rate. This makes the audit
/// history visibly show a "looks fine" interlude before the final break.
pub const INTERMEDIATE_USD_RATE: &str = "0.95";

/// Final broken value set on step 3 of the three-step break. Non-numeric on
/// purpose — `UpdateRecord-fx-rate`'s RecordPath uses `:toNumber()`, which
/// throws on this and routes the flowfile to `failure`. The `failure`
/// relationship is wired to two downstream connections (deadletter +
/// tag-retries), so NiFi clones the failed flowfile: one copy fires WARN
/// bulletins via the deadletter LogAttribute, the other continues down the
/// main flow to the regional sinks.
pub const BROKEN_USD_RATE: &str = "oops";

pub async fn seed(client: &DynamicClient) -> Result<OrdersContextIds> {
    tracing::info!("seeding fixture-pc-platform");
    let platform_id = create_context(
        client,
        "fixture-pc-platform",
        "Platform-wide infrastructure parameters (cross-cutting)",
        vec![
            param(
                "kafka_bootstrap",
                "kafka.platform.svc.cluster.local:9092",
                false,
            ),
            param(
                "audit_log_endpoint",
                "https://audit.platform.svc/events",
                false,
            ),
            param_sensitive("db_password", "********"),
        ],
        vec![],
    )
    .await?;

    tracing::info!(%platform_id, "seeding fixture-pc-orders");
    let orders_id = create_context(
        client,
        "fixture-pc-orders",
        "Orders domain parameters (inherits platform)",
        vec![
            param("usd_rate", HEALTHY_USD_RATE, false),
            param("region_filter", "EU,US,APAC", false),
            param("currency_default", "USD", false),
            param("retry_max", "5", false),
        ],
        vec![context_ref(&platform_id)],
    )
    .await?;

    tracing::info!(%orders_id, "seeding fixture-pc-region-eu");
    let region_eu_id = create_context(
        client,
        "fixture-pc-region-eu",
        "EU regional overlay (inherits orders)",
        vec![
            param("region_filter", "EU", false),
            param("compliance_tag", "GDPR-2024", false),
        ],
        vec![context_ref(&orders_id)],
    )
    .await?;

    tracing::info!(%orders_id, "seeding fixture-pc-region-us");
    let region_us_id = create_context(
        client,
        "fixture-pc-region-us",
        "US regional overlay (inherits orders)",
        vec![
            param("region_filter", "US", false),
            param("compliance_tag", "SOC2", false),
        ],
        vec![context_ref(&orders_id)],
    )
    .await?;

    tracing::info!(%orders_id, "seeding fixture-pc-region-apac");
    let region_apac_id = create_context(
        client,
        "fixture-pc-region-apac",
        "APAC regional overlay (inherits orders)",
        vec![
            param("region_filter", "APAC", false),
            param("compliance_tag", "PDPA-2023", false),
        ],
        vec![context_ref(&orders_id)],
    )
    .await?;

    Ok(OrdersContextIds {
        platform_id,
        orders_id,
        region_eu_id,
        region_us_id,
        region_apac_id,
    })
}

/// Bind `pg_id` to `context_id`. GET the current entity for its revision,
/// then PUT a minimal patch.
pub async fn bind(client: &DynamicClient, pg_id: &str, context_id: &str) -> Result<()> {
    let current = client
        .processgroups()
        .get_process_group(pg_id)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("get process group {pg_id} for parameter context binding"),
            source: Box::new(e),
        })?;

    let revision = current
        .revision
        .clone()
        .ok_or_else(|| SeederError::Invariant {
            message: format!("process group {pg_id} has no revision"),
        })?;

    let mut component = types::ProcessGroupDto::default();
    component.id = current.component.as_ref().and_then(|c| c.id.clone());
    component.name = current.component.as_ref().and_then(|c| c.name.clone());
    component.parameter_context = Some(context_ref(context_id));

    let mut entity = types::ProcessGroupEntity::default();
    entity.id = current.id.clone();
    entity.revision = Some(revision);
    entity.component = Some(component);

    client
        .processgroups()
        .update_process_group(pg_id, &entity)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("bind parameter context {context_id} to process group {pg_id}"),
            source: Box::new(e),
        })?;

    tracing::debug!(%pg_id, %context_id, "parameter context bound");
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal builders
// ---------------------------------------------------------------------------

async fn create_context(
    client: &DynamicClient,
    name: &str,
    description: &str,
    parameters: Vec<types::ParameterEntity>,
    inherits: Vec<types::ParameterContextReferenceEntity>,
) -> Result<String> {
    let mut dto = types::ParameterContextDto::default();
    dto.name = Some(name.to_string());
    dto.description = Some(description.to_string());
    dto.parameters = Some(parameters);
    if !inherits.is_empty() {
        dto.inherited_parameter_contexts = Some(inherits);
    }

    let mut revision = types::RevisionDto::default();
    revision.version = Some(0);

    let mut entity = types::ParameterContextEntity::default();
    entity.component = Some(dto);
    entity.revision = Some(revision);

    let created = client
        .parametercontexts()
        .create_parameter_context(&entity)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("create parameter context {name}"),
            source: Box::new(e),
        })?;

    created
        .component
        .and_then(|c| c.id)
        .or(created.id)
        .ok_or_else(|| SeederError::Invariant {
            message: format!("parameter context {name} has no id after create"),
        })
}

fn param(name: &str, value: &str, sensitive: bool) -> types::ParameterEntity {
    let mut dto = types::ParameterDto::default();
    dto.name = Some(name.to_string());
    dto.value = Some(value.to_string());
    dto.sensitive = Some(sensitive);

    let mut entity = types::ParameterEntity::default();
    entity.parameter = Some(dto);
    entity
}

fn param_sensitive(name: &str, value: &str) -> types::ParameterEntity {
    param(name, value, true)
}

fn context_ref(context_id: &str) -> types::ParameterContextReferenceEntity {
    let mut dto = types::ParameterContextReferenceDto::default();
    dto.id = Some(context_id.to_string());

    let mut entity = types::ParameterContextReferenceEntity::default();
    entity.id = Some(context_id.to_string());
    entity.component = Some(dto);
    entity
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthy_usd_rate_is_numeric() {
        // Trivial unit test asserting the constant parses as a float;
        // guards against accidentally swapping HEALTHY_USD_RATE and
        // BROKEN_USD_RATE during edits.
        assert!(HEALTHY_USD_RATE.parse::<f64>().is_ok());
    }

    #[test]
    fn intermediate_usd_rate_is_numeric() {
        assert!(INTERMEDIATE_USD_RATE.parse::<f64>().is_ok());
    }

    #[test]
    fn broken_usd_rate_is_not_numeric() {
        assert!(BROKEN_USD_RATE.parse::<f64>().is_err());
    }

    #[test]
    fn param_sensitive_flag_set() {
        let e = param_sensitive("db_password", "x");
        assert_eq!(e.parameter.unwrap().sensitive, Some(true));
    }
}
