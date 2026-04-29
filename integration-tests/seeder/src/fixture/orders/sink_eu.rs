//! sink-eu/ child PG. Stubbed; filled in Task 8.

use nifi_rust_client::dynamic::DynamicClient;

use crate::error::{Result, SeederError};
use crate::fixture::parameter_contexts::OrdersContextIds;

pub struct SinkEuIds {
    pub pg_id: String,
    pub in_port_id: String,
}

pub async fn seed(
    _client: &DynamicClient,
    _orders_pg_id: &str,
    _contexts: &OrdersContextIds,
    _incoming_eu_port_id: &str,
) -> Result<SinkEuIds> {
    Err(SeederError::Invariant {
        message: "sink_eu::seed not yet implemented".into(),
    })
}
