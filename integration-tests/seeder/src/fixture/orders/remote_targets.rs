//! Sibling subtree hosting RPG-target input ports. Stubbed; filled in Task 6.

use nifi_rust_client::dynamic::DynamicClient;

use crate::error::{Result, SeederError};

pub async fn seed(_client: &DynamicClient, _pg_id: &str) -> Result<(String, String)> {
    Err(SeederError::Invariant {
        message: "remote_targets::seed not yet implemented".into(),
    })
}
