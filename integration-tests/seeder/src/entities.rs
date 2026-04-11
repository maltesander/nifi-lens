//! Entity-builder helpers. Collapse the verbosity of constructing
//! `nifi_rust_client::dynamic::types::*Entity` DTOs into small functions
//! that take topology-level parameters.
//!
//! Every DTO in the generated bindings is `#[non_exhaustive]` and uses
//! `Option<T>` for virtually every field, so construction goes through
//! `Default::default()` followed by field assignments. These helpers
//! hide that boilerplate behind topology-level signatures.

use std::collections::HashMap;

use nifi_rust_client::dynamic::types;

/// Build a `ProcessGroupEntity` suitable for `create_process_group`.
///
/// The returned entity has `component.name = Some(name)` and a revision
/// of `0` (required for POST creations).
pub fn make_pg(name: &str) -> types::ProcessGroupEntity {
    let mut component = types::ProcessGroupDto::default();
    component.name = Some(name.to_string());

    let mut revision = types::RevisionDto::default();
    revision.version = Some(0);

    let mut entity = types::ProcessGroupEntity::default();
    entity.component = Some(component);
    entity.revision = Some(revision);
    entity
}

/// Build a `ProcessorEntity`.
///
/// * `processor_type` is the fully-qualified Java class name (e.g.,
///   `org.apache.nifi.processors.standard.GenerateFlowFile`).
/// * `properties` is a flat map of NiFi property name to value.
/// * `scheduling_period` is the scheduling period string (e.g. `"1 sec"`,
///   `"100 ms"`, `"2 sec"`, `"1 min"`). `None` leaves the NiFi default.
/// * `auto_terminate` lists relationships that should be auto-terminated.
pub fn make_processor(
    name: &str,
    processor_type: &str,
    properties: HashMap<String, String>,
    scheduling_period: Option<&str>,
    auto_terminate: Vec<&str>,
) -> types::ProcessorEntity {
    let mut config = types::ProcessorConfigDto::default();
    config.properties = Some(properties.into_iter().map(|(k, v)| (k, Some(v))).collect());
    if let Some(schedule) = scheduling_period {
        config.scheduling_period = Some(schedule.to_string());
    }
    if !auto_terminate.is_empty() {
        config.auto_terminated_relationships =
            Some(auto_terminate.into_iter().map(String::from).collect());
    }

    let mut component = types::ProcessorDto::default();
    component.name = Some(name.to_string());
    component.r#type = Some(processor_type.to_string());
    component.config = Some(config);

    let mut revision = types::RevisionDto::default();
    revision.version = Some(0);

    let mut entity = types::ProcessorEntity::default();
    entity.component = Some(component);
    entity.revision = Some(revision);
    entity
}

/// Build a `ConnectionEntity` between two endpoints in the same PG.
///
/// * `group_id` is the UUID of the parent PG.
/// * `source_id` / `destination_id` are UUIDs from previously-created
///   processors or ports.
/// * `source_type` / `destination_type` are one of `"PROCESSOR"`,
///   `"INPUT_PORT"`, `"OUTPUT_PORT"`, etc.
/// * `relationships` are the selected relationship names on the source
///   side of the connection.
pub fn make_connection(
    group_id: &str,
    source_id: &str,
    source_type: &str,
    destination_id: &str,
    destination_type: &str,
    relationships: Vec<&str>,
) -> types::ConnectionEntity {
    // `ConnectableDto` has non-Option `group_id`, `id`, and `r#type`
    // fields. Since the DTO is `#[non_exhaustive]` we can't use a struct
    // literal — start from `default()` and overwrite.
    let mut source = types::ConnectableDto::default();
    source.group_id = group_id.to_string();
    source.id = source_id.to_string();
    source.r#type = source_type.to_string();

    let mut destination = types::ConnectableDto::default();
    destination.group_id = group_id.to_string();
    destination.id = destination_id.to_string();
    destination.r#type = destination_type.to_string();

    let mut component = types::ConnectionDto::default();
    component.source = Some(source);
    component.destination = Some(destination);
    if !relationships.is_empty() {
        component.selected_relationships =
            Some(relationships.into_iter().map(String::from).collect());
    }

    let mut revision = types::RevisionDto::default();
    revision.version = Some(0);

    // `ConnectionEntity.source_type` and `destination_type` are
    // non-Option `String`s — NiFi requires them at the entity level as
    // well as inside the component DTO.
    let mut entity = types::ConnectionEntity::default();
    entity.source_type = source_type.to_string();
    entity.destination_type = destination_type.to_string();
    entity.component = Some(component);
    entity.revision = Some(revision);
    entity
}

/// Build a `PortEntity` for input/output ports.
pub fn make_port(name: &str) -> types::PortEntity {
    let mut component = types::PortDto::default();
    component.name = Some(name.to_string());

    let mut revision = types::RevisionDto::default();
    revision.version = Some(0);

    let mut entity = types::PortEntity::default();
    entity.component = Some(component);
    entity.revision = Some(revision);
    entity
}

/// Build a `ControllerServiceEntity`.
///
/// Same pattern as [`make_processor`] — `cs_type` is the fully-qualified
/// Java class name of the controller service, `properties` is a flat
/// NiFi property-name -> value map.
pub fn make_controller_service(
    name: &str,
    cs_type: &str,
    properties: HashMap<String, String>,
) -> types::ControllerServiceEntity {
    let mut component = types::ControllerServiceDto::default();
    component.name = Some(name.to_string());
    component.r#type = Some(cs_type.to_string());
    component.properties = Some(properties.into_iter().map(|(k, v)| (k, Some(v))).collect());

    let mut revision = types::RevisionDto::default();
    revision.version = Some(0);

    let mut entity = types::ControllerServiceEntity::default();
    entity.component = Some(component);
    entity.revision = Some(revision);
    entity
}

/// Convenience: build a `HashMap<String, String>` from an array of
/// `(&str, &str)` pairs. Used to keep property literals in fixtures
/// short and readable.
pub fn props(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn props_helper_builds_map() {
        let m = props(&[("a", "1"), ("b", "2")]);
        assert_eq!(m.get("a"), Some(&"1".to_string()));
        assert_eq!(m.get("b"), Some(&"2".to_string()));
    }

    #[test]
    fn make_pg_sets_name() {
        let pg = make_pg("healthy-pipeline");
        assert_eq!(
            pg.component.and_then(|c| c.name),
            Some("healthy-pipeline".into())
        );
    }
}
