//! transform/ child PG. Stubbed; filled in Task 7.

use nifi_rust_client::dynamic::DynamicClient;

use crate::error::{Result, SeederError};
use crate::fixture::parameter_contexts::OrdersContextIds;
use crate::fixture::services::ServiceIds;

pub struct TransformIds {
    pub pg_id: String,
    pub incoming_port_id: String,
    pub out_eu_port_id: String,
    pub out_us_port_id: String,
    pub out_apac_port_id: String,
    pub out_failed_port_id: String,
}

pub async fn seed(
    _client: &DynamicClient,
    _orders_pg_id: &str,
    _contexts: &OrdersContextIds,
    _service_ids: &ServiceIds,
    _version: &semver::Version,
) -> Result<TransformIds> {
    Err(SeederError::Invariant {
        message: "transform::seed not yet implemented".into(),
    })
}
