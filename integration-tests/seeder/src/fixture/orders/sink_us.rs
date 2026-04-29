//! sink-us/ child PG. Stubbed; filled in Task 9.

use nifi_rust_client::dynamic::DynamicClient;

use crate::error::{Result, SeederError};
use crate::fixture::parameter_contexts::OrdersContextIds;

pub struct SinkUsIds {
    pub pg_id: String,
    pub in_port_id: String,
}

pub async fn seed(
    _client: &DynamicClient,
    _orders_pg_id: &str,
    _contexts: &OrdersContextIds,
) -> Result<SinkUsIds> {
    Err(SeederError::Invariant {
        message: "sink_us::seed not yet implemented".into(),
    })
}
