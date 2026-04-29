//! deadletter/ child PG. Stubbed; filled in Task 11.

use nifi_rust_client::dynamic::DynamicClient;

use crate::error::{Result, SeederError};

pub struct DeadletterIds {
    pub pg_id: String,
    pub in_port_id: String,
}

pub async fn seed(_client: &DynamicClient, _orders_pg_id: &str) -> Result<DeadletterIds> {
    Err(SeederError::Invariant {
        message: "deadletter::seed not yet implemented".into(),
    })
}
