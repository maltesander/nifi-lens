//! Fixture-marker detection. The seeder creates a top-level process
//! group named [`FIXTURE_MARKER_NAME`] as the first step of seeding; its
//! presence means the cluster already holds the current fixture version
//! and `--skip-if-seeded` can short-circuit.

use nifi_rust_client::dynamic::{
    DynamicClient,
    traits::{ProcessGroupsApi as _, ProcessGroupsProcessGroupsApi as _},
};

use crate::error::{Result, SeederError};

/// Name of the marker PG. Bumping this (`v1` → `v2`) invalidates stale
/// fixtures: the next nuke-and-repave pass will delete the old marker
/// along with everything else.
pub const FIXTURE_MARKER_NAME: &str = "nifilens-fixture-v1";

/// Returns `Some(pg_id)` if the marker PG exists as a direct child of
/// root, `None` otherwise.
pub async fn find_marker(client: &DynamicClient) -> Result<Option<String>> {
    let entity = client
        .processgroups_api()
        .process_groups("root")
        .get_process_groups()
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
        if matches_name && let Some(id) = pg.id {
            return Ok(Some(id));
        }
    }
    Ok(None)
}
