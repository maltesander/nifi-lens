//! sink-apac/ child PG. Stubbed; filled in Task 10.

use nifi_rust_client::dynamic::DynamicClient;

use crate::error::{Result, SeederError};
use crate::fixture::parameter_contexts::OrdersContextIds;

pub struct SinkApacIds {
    pub pg_id: String,
    pub in_port_id: String,
}

pub async fn seed(
    _client: &DynamicClient,
    _orders_pg_id: &str,
    _contexts: &OrdersContextIds,
    _incoming_apac_port_id: &str,
) -> Result<SinkApacIds> {
    Err(SeederError::Invariant {
        message: "sink_apac::seed not yet implemented".into(),
    })
}
