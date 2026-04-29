//! Phase 8 of seeding: --break-after sleep + idempotent parameter mutation.
//!
//! After the orders-pipeline topology is fully started, sleep for the
//! configured duration, then PUT `usd_rate = "oops"` on fixture-pc-orders.
//! The PUT is gated by a value check: if `usd_rate` is already `"oops"`,
//! this is a no-op (re-run safe).

use std::time::Duration;

use nifi_rust_client::dynamic::{DynamicClient, types};
use nifi_rust_client::wait;

use crate::error::{Result, SeederError};
use crate::fixture::parameter_contexts::{BROKEN_USD_RATE, HEALTHY_USD_RATE};

pub async fn apply_break(
    client: &DynamicClient,
    orders_context_id: &str,
    delay: Duration,
) -> Result<()> {
    if !delay.is_zero() {
        tracing::info!(?delay, "sleeping before applying parameter break");
        tokio::time::sleep(delay).await;
    }

    // Fetch the current context so we can inspect usd_rate's value and
    // carry the current revision into the update.
    let current = client
        .parametercontexts()
        .get_parameter_context(orders_context_id, None)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("fetch parameter context {orders_context_id}"),
            source: Box::new(e),
        })?;

    let component = current
        .component
        .as_ref()
        .ok_or_else(|| SeederError::Invariant {
            message: format!("parameter context {orders_context_id} missing component"),
        })?;
    let params = component
        .parameters
        .as_ref()
        .ok_or_else(|| SeederError::Invariant {
            message: format!("parameter context {orders_context_id} missing parameters"),
        })?;
    let current_value: Option<&str> = params.iter().find_map(|p| {
        let dto = p.parameter.as_ref()?;
        if dto.name.as_deref() == Some("usd_rate") {
            dto.value.as_deref()
        } else {
            None
        }
    });

    match current_value {
        Some(v) if v == BROKEN_USD_RATE => {
            tracing::info!("usd_rate already broken — skipping mutation");
            return Ok(());
        }
        Some(v) if v == HEALTHY_USD_RATE => {
            tracing::info!(%v, "applying break: usd_rate -> oops");
        }
        Some(v) => {
            tracing::warn!(%v, "usd_rate is in an unexpected state; applying break anyway");
        }
        None => {
            return Err(SeederError::Invariant {
                message: format!("parameter context {orders_context_id} has no usd_rate parameter"),
            });
        }
    }

    // Build a parameter-context update entity carrying just the one mutated
    // parameter. NiFi diffs against the current state and only acts on the
    // delta.
    let mut new_param_dto = types::ParameterDto::default();
    new_param_dto.name = Some("usd_rate".to_string());
    new_param_dto.value = Some(BROKEN_USD_RATE.to_string());
    new_param_dto.sensitive = Some(false);

    let mut new_param = types::ParameterEntity::default();
    new_param.parameter = Some(new_param_dto);

    let mut update_dto = types::ParameterContextDto::default();
    update_dto.id = Some(orders_context_id.to_string());
    update_dto.parameters = Some(vec![new_param]);

    let mut entity = types::ParameterContextEntity::default();
    entity.id = Some(orders_context_id.to_string());
    entity.revision = current.revision.clone();
    entity.component = Some(update_dto);

    // Parameter context updates are async — submitted via update-request,
    // then polled until complete.
    let update_request = client
        .parametercontexts()
        .submit_parameter_context_update(orders_context_id, &entity)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("submit parameter update on {orders_context_id}"),
            source: Box::new(e),
        })?;

    let request_id = update_request
        .request
        .as_ref()
        .and_then(|r| r.request_id.clone())
        .ok_or_else(|| SeederError::Invariant {
            message: "parameter update request has no id".into(),
        })?;

    // Poll until complete or timeout. The library helper handles three
    // edge cases the bespoke loop missed: it inspects `failure_reason`
    // (so a NiFi-rejected update surfaces as an error rather than a
    // silent success), it traces best-effort cleanup failures, and it
    // maps timeouts to NifiError::Timeout. NiFi can stall a parameter
    // update behind running components, so allow a generous 60s ceiling.
    let config = wait::WaitConfig {
        timeout: Duration::from_secs(60),
        ..Default::default()
    };
    wait::parameter_context_update_dynamic(client, orders_context_id, &request_id, config)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("await parameter update {orders_context_id}/{request_id}"),
            source: Box::new(e),
        })?;

    tracing::info!("usd_rate mutated to {BROKEN_USD_RATE}");
    Ok(())
}
