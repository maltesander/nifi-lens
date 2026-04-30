//! `--break-after` sleep + idempotent three-step parameter mutation.
//!
//! After the orders-pipeline topology is fully started, sleep for the
//! configured `--break-after` duration, then run a three-step mutation on
//! `usd_rate` of `fixture-pc-orders`:
//!
//!   1. Clear value (`""`)         — "developer cleared it to start over"
//!   2. Set to `INTERMEDIATE_USD_RATE` (`"0.95"`) — "tried a different rate"
//!   3. Set to `BROKEN_USD_RATE` (`"oops"`)      — "left a debugging value in"
//!
//! Each step is its own `submit_parameter_context_update` call so NiFi's
//! action-history records three distinct audit entries. A 2-second pause
//! between steps separates the timestamps for screenshots without dragging
//! out CI.
//!
//! The sequence is gated by a value check on `usd_rate`: if the parameter is
//! already at `BROKEN_USD_RATE`, the entire dance is skipped (re-run safe).

use std::time::Duration;

use nifi_rust_client::dynamic::{DynamicClient, types};
use nifi_rust_client::wait;

use crate::error::{Result, SeederError};
use crate::fixture::parameter_contexts::{
    BROKEN_USD_RATE, HEALTHY_USD_RATE, INTERMEDIATE_USD_RATE,
};

/// Pause between consecutive mutation steps. Long enough that audit-history
/// timestamps render distinctly in screenshots; short enough not to slow CI.
const STEP_GAP: Duration = Duration::from_secs(2);

pub async fn apply_break(
    client: &DynamicClient,
    orders_context_id: &str,
    delay: Duration,
) -> Result<()> {
    if !delay.is_zero() {
        tracing::info!(?delay, "sleeping before applying parameter break");
        tokio::time::sleep(delay).await;
    }

    // Inspect current usd_rate value to decide whether to run the sequence.
    // The inheritance chain captured here is reused for every step so we
    // don't re-fetch it three times.
    let current = fetch_context(client, orders_context_id).await?;
    let inherited = current
        .component
        .as_ref()
        .and_then(|c| c.inherited_parameter_contexts.clone());
    let current_value = read_usd_rate_value(&current).ok_or_else(|| SeederError::Invariant {
        message: format!("parameter context {orders_context_id} has no usd_rate parameter"),
    })?;

    match current_value.as_deref() {
        Some(v) if v == BROKEN_USD_RATE => {
            tracing::info!("usd_rate already at BROKEN_USD_RATE — skipping three-step break");
            return Ok(());
        }
        Some(v) if v == HEALTHY_USD_RATE => {
            tracing::info!(%v, "applying three-step break: clear → intermediate → oops");
        }
        Some(v) => {
            tracing::warn!(%v, "usd_rate is in an unexpected state; running three-step break anyway");
        }
        None => {
            tracing::info!("usd_rate has no value; running three-step break from cleared state");
        }
    }

    // Step 1: clear the value. Demo narrative: "developer panicked and
    // wiped it to start clean".
    set_usd_rate(client, orders_context_id, "", inherited.as_ref()).await?;
    tracing::info!("step 1/3: usd_rate cleared");
    tokio::time::sleep(STEP_GAP).await;

    // Step 2: set to a numeric "tested rate". Demo narrative: "tried a
    // different value, briefly the pipeline runs clean".
    set_usd_rate(
        client,
        orders_context_id,
        INTERMEDIATE_USD_RATE,
        inherited.as_ref(),
    )
    .await?;
    tracing::info!("step 2/3: usd_rate -> {INTERMEDIATE_USD_RATE}");
    tokio::time::sleep(STEP_GAP).await;

    // Step 3: set to the broken value. Demo narrative: "left a debugging
    // placeholder in and walked away".
    set_usd_rate(
        client,
        orders_context_id,
        BROKEN_USD_RATE,
        inherited.as_ref(),
    )
    .await?;
    tracing::info!("step 3/3: usd_rate -> {BROKEN_USD_RATE}");

    tracing::info!("three-step break sequence complete");
    Ok(())
}

/// Fetch the parameter context entity and return it. Each step needs the
/// current revision (NiFi rejects stale revisions), so this is called once
/// per step.
async fn fetch_context(
    client: &DynamicClient,
    orders_context_id: &str,
) -> Result<types::ParameterContextEntity> {
    client
        .parametercontexts()
        .get_parameter_context(orders_context_id, None)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("fetch parameter context {orders_context_id}"),
            source: Box::new(e),
        })
}

/// Pull the current `usd_rate` value out of a fetched context entity.
/// Returns `Some(Some(value))` when the parameter is present with a value,
/// `Some(None)` when present but value-less, and `None` when the parameter
/// itself is missing.
fn read_usd_rate_value(entity: &types::ParameterContextEntity) -> Option<Option<String>> {
    let component = entity.component.as_ref()?;
    let params = component.parameters.as_ref()?;
    params.iter().find_map(|p| {
        let dto = p.parameter.as_ref()?;
        if dto.name.as_deref() == Some("usd_rate") {
            Some(dto.value.clone())
        } else {
            None
        }
    })
}

/// Submit one parameter-context update setting `usd_rate.value = value` and
/// wait for NiFi to apply it. The inheritance chain must be repeated on every
/// update — a missing `inherited_parameter_contexts` field is interpreted as
/// "clear inheritance", which would try to remove every parameter inherited
/// from `fixture-pc-platform` and fail with a 409 when any of those is
/// referenced by a running component.
async fn set_usd_rate(
    client: &DynamicClient,
    orders_context_id: &str,
    value: &str,
    inherited: Option<&Vec<types::ParameterContextReferenceEntity>>,
) -> Result<()> {
    // Re-fetch to pick up the revision NiFi advanced on the previous step.
    let current = fetch_context(client, orders_context_id).await?;

    let mut new_param_dto = types::ParameterDto::default();
    new_param_dto.name = Some("usd_rate".to_string());
    new_param_dto.value = Some(value.to_string());
    new_param_dto.sensitive = Some(false);

    let mut new_param = types::ParameterEntity::default();
    new_param.parameter = Some(new_param_dto);

    let mut update_dto = types::ParameterContextDto::default();
    update_dto.id = Some(orders_context_id.to_string());
    update_dto.parameters = Some(vec![new_param]);
    update_dto.inherited_parameter_contexts = inherited.cloned();

    let mut entity = types::ParameterContextEntity::default();
    entity.id = Some(orders_context_id.to_string());
    entity.revision = current.revision.clone();
    entity.component = Some(update_dto);

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

    // NiFi can stall a parameter update behind running components, so allow
    // a generous 60s ceiling per step.
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

    Ok(())
}
