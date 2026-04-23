//! Fixture-marker detection. The seeder creates a top-level process
//! group named [`FIXTURE_MARKER_NAME`] as the first step of seeding; its
//! presence means the cluster already holds the current fixture version
//! and `--skip-if-seeded` can short-circuit.

use nifi_rust_client::dynamic::DynamicClient;

use crate::error::{Result, SeederError};

/// Name of the marker PG. Bumping this (`v2` → `v3`) invalidates stale
/// fixtures: the next nuke-and-repave pass will delete the old marker
/// along with everything else.
pub const FIXTURE_MARKER_NAME: &str = "nifilens-fixture-v4";

/// Returns `Some(pg_id)` if the marker PG exists as a direct child of
/// root, `None` otherwise.
pub async fn find_marker(client: &DynamicClient) -> Result<Option<String>> {
    let entity = client
        .processgroups()
        .get_process_groups("root")
        .await
        .map_err(|e| SeederError::Api {
            message: "list root process groups".into(),
            source: Box::new(e),
        })?;

    let Some(children) = entity.process_groups else {
        return Ok(None);
    };

    for pg in children {
        let matches_name = pg
            .component
            .as_ref()
            .and_then(|c| c.name.as_ref())
            .is_some_and(|n| n == FIXTURE_MARKER_NAME);
        if matches_name {
            let id = pg.component.and_then(|c| c.id).or(pg.id);
            if let Some(id) = id {
                return Ok(Some(id));
            }
        }
    }
    Ok(None)
}
