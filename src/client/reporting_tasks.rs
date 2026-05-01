//! Typed snapshot of NiFi reporting tasks (`/flow/reporting-tasks`).
//!
//! Reporting tasks are cluster-scoped components that run on a fixed
//! schedule (TIMER_DRIVEN or CRON_DRIVEN) and emit observability data
//! (Prometheus exporter, S2S bulletin reporter, MonitorMemory, …).

use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct ReportingTasksSnapshot {
    pub tasks: Vec<ReportingTaskRow>,
    pub fetched_at: Instant,
}

#[derive(Debug, Clone)]
pub struct ReportingTaskRow {
    pub id: String,
    pub name: String,
    /// Fully-qualified class name, e.g.
    /// `org.apache.nifi.reporting.prometheus.PrometheusReportingTask`.
    pub task_type: String,
    pub state: ReportingTaskState,
    /// Wire string: `"TIMER_DRIVEN"` or `"CRON_DRIVEN"`. Kept as a
    /// string because there are exactly two values and downstream
    /// rendering shows them verbatim.
    pub scheduling_strategy: String,
    /// Period literal: `"30s"` for TIMER_DRIVEN or a cron expression
    /// for CRON_DRIVEN. Rendered verbatim.
    pub scheduling_period: String,
    pub active_thread_count: u32,
    pub validation_status: ValidationStatus,
    pub validation_errors: Vec<String>,
    pub comments: Option<String>,
    /// `None` value means the property is sensitive — NiFi masks it
    /// server-side and the renderer shows `[sensitive]`.
    pub properties: BTreeMap<String, Option<String>>,
    pub descriptors: BTreeMap<String, ReportingTaskPropertyDescriptor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportingTaskState {
    Running,
    Stopped,
    Disabled,
}

impl ReportingTaskState {
    /// Map the NiFi wire string. Unknown values warn and fall back to
    /// `Stopped` — mirrors `ProcessorStatus::from_wire` discipline.
    pub fn from_wire(raw: &str) -> Self {
        if raw.eq_ignore_ascii_case("RUNNING") {
            Self::Running
        } else if raw.eq_ignore_ascii_case("STOPPED") {
            Self::Stopped
        } else if raw.eq_ignore_ascii_case("DISABLED") {
            Self::Disabled
        } else {
            tracing::warn!(value = %raw, "unknown reporting-task state");
            Self::Stopped
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationStatus {
    Valid,
    Invalid,
    Validating,
}

impl ValidationStatus {
    pub fn from_wire(raw: &str) -> Self {
        if raw.eq_ignore_ascii_case("VALID") {
            Self::Valid
        } else if raw.eq_ignore_ascii_case("INVALID") {
            Self::Invalid
        } else if raw.eq_ignore_ascii_case("VALIDATING") {
            Self::Validating
        } else {
            tracing::warn!(value = %raw, "unknown reporting-task validation status");
            Self::Validating
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReportingTaskPropertyDescriptor {
    pub display_name: String,
    pub sensitive: bool,
    pub required: bool,
    pub default_value: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReportingTaskCounts {
    pub total: usize,
    pub running: usize,
    pub stopped: usize,
    pub invalid: usize,
}

impl ReportingTasksSnapshot {
    /// Map the wire-level `ReportingTasksEntity` into our typed snapshot.
    /// Sensitive properties (per descriptor) have their value masked to
    /// `None` so renderers show `[sensitive]`.
    pub fn from_entity(entity: nifi_rust_client::dynamic::types::ReportingTasksEntity) -> Self {
        let tasks = entity
            .reporting_tasks
            .unwrap_or_default()
            .into_iter()
            .filter_map(ReportingTaskRow::from_entity)
            .collect();
        Self {
            tasks,
            fetched_at: Instant::now(),
        }
    }

    pub fn counts(&self) -> ReportingTaskCounts {
        let mut c = ReportingTaskCounts {
            total: self.tasks.len(),
            ..ReportingTaskCounts::default()
        };
        for t in &self.tasks {
            if t.validation_status == ValidationStatus::Invalid {
                c.invalid += 1;
            }
            match t.state {
                ReportingTaskState::Running if t.validation_status == ValidationStatus::Valid => {
                    c.running += 1;
                }
                ReportingTaskState::Stopped | ReportingTaskState::Disabled => {
                    c.stopped += 1;
                }
                ReportingTaskState::Running => {
                    // running-but-not-valid → does not count toward
                    // `running`. The `invalid` bucket already reflects it.
                }
            }
        }
        c
    }
}

impl ReportingTaskRow {
    pub fn from_entity(
        entity: nifi_rust_client::dynamic::types::ReportingTaskEntity,
    ) -> Option<Self> {
        let component = entity.component?;
        let id = component.id.clone().or(entity.id)?;

        let descriptors: BTreeMap<String, ReportingTaskPropertyDescriptor> = component
            .descriptors
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(k, opt_dto)| {
                opt_dto.map(|dto| {
                    (
                        k.clone(),
                        ReportingTaskPropertyDescriptor {
                            display_name: dto.display_name.unwrap_or_else(|| k.clone()),
                            sensitive: dto.sensitive.unwrap_or(false),
                            required: dto.required.unwrap_or(false),
                            default_value: dto.default_value,
                        },
                    )
                })
            })
            .collect();

        // Mask sensitive values: descriptor.sensitive == true → value None.
        let properties: BTreeMap<String, Option<String>> = component
            .properties
            .unwrap_or_default()
            .into_iter()
            .map(|(k, v)| {
                let masked = descriptors.get(&k).map(|d| d.sensitive).unwrap_or(false);
                let value = if masked { None } else { v };
                (k, value)
            })
            .collect();

        Some(Self {
            id,
            name: component.name.unwrap_or_default(),
            task_type: component.r#type.unwrap_or_default(),
            state: ReportingTaskState::from_wire(component.state.as_deref().unwrap_or("")),
            scheduling_strategy: component.scheduling_strategy.unwrap_or_default(),
            scheduling_period: component.scheduling_period.unwrap_or_default(),
            active_thread_count: component.active_thread_count.unwrap_or(0).max(0) as u32,
            validation_status: ValidationStatus::from_wire(
                component.validation_status.as_deref().unwrap_or(""),
            ),
            validation_errors: component.validation_errors.unwrap_or_default(),
            comments: component.comments.filter(|s| !s.is_empty()),
            properties,
            descriptors,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(state: ReportingTaskState, valid: ValidationStatus) -> ReportingTaskRow {
        ReportingTaskRow {
            id: "x".into(),
            name: "x".into(),
            task_type: "x".into(),
            state,
            scheduling_strategy: "TIMER_DRIVEN".into(),
            scheduling_period: "30s".into(),
            active_thread_count: 0,
            validation_status: valid,
            validation_errors: vec![],
            comments: None,
            properties: BTreeMap::new(),
            descriptors: BTreeMap::new(),
        }
    }

    #[test]
    fn state_from_wire_known() {
        assert_eq!(
            ReportingTaskState::from_wire("RUNNING"),
            ReportingTaskState::Running
        );
        assert_eq!(
            ReportingTaskState::from_wire("STOPPED"),
            ReportingTaskState::Stopped
        );
        assert_eq!(
            ReportingTaskState::from_wire("DISABLED"),
            ReportingTaskState::Disabled
        );
    }

    #[test]
    fn state_from_wire_unknown_falls_back_to_stopped() {
        assert_eq!(
            ReportingTaskState::from_wire("WHATEVER"),
            ReportingTaskState::Stopped
        );
    }

    #[test]
    fn validation_from_wire() {
        assert_eq!(
            ValidationStatus::from_wire("VALID"),
            ValidationStatus::Valid
        );
        assert_eq!(
            ValidationStatus::from_wire("INVALID"),
            ValidationStatus::Invalid
        );
        assert_eq!(
            ValidationStatus::from_wire("VALIDATING"),
            ValidationStatus::Validating
        );
        assert_eq!(
            ValidationStatus::from_wire("???"),
            ValidationStatus::Validating
        );
    }

    #[test]
    fn from_wire_is_case_insensitive() {
        assert_eq!(
            ReportingTaskState::from_wire("running"),
            ReportingTaskState::Running
        );
        assert_eq!(
            ReportingTaskState::from_wire("Stopped"),
            ReportingTaskState::Stopped
        );
        assert_eq!(
            ValidationStatus::from_wire("invalid"),
            ValidationStatus::Invalid
        );
    }

    #[test]
    fn snapshot_counts_running_stopped_invalid() {
        let snapshot = ReportingTasksSnapshot {
            tasks: vec![
                row(ReportingTaskState::Running, ValidationStatus::Valid),
                row(ReportingTaskState::Running, ValidationStatus::Valid),
                row(ReportingTaskState::Stopped, ValidationStatus::Valid),
                row(ReportingTaskState::Disabled, ValidationStatus::Valid),
                row(ReportingTaskState::Running, ValidationStatus::Invalid),
            ],
            fetched_at: Instant::now(),
        };
        let counts = snapshot.counts();
        assert_eq!(counts.running, 2, "running AND valid only");
        assert_eq!(counts.stopped, 2, "stopped + disabled");
        assert_eq!(
            counts.invalid, 1,
            "validation_status Invalid is orthogonal to state"
        );
        assert_eq!(counts.total, 5);
    }

    #[test]
    fn from_entity_masks_sensitive_properties() {
        use nifi_rust_client::dynamic::types::{
            PropertyDescriptorDto, ReportingTaskDto, ReportingTaskEntity, ReportingTasksEntity,
        };
        use std::collections::HashMap;

        let mut props = HashMap::new();
        props.insert("password".to_string(), Some("hunter2".to_string()));
        props.insert("public".to_string(), Some("v".to_string()));

        let mut desc_password = PropertyDescriptorDto::default();
        desc_password.display_name = Some("Password".to_string());
        desc_password.sensitive = Some(true);
        desc_password.required = Some(true);

        let mut desc_public = PropertyDescriptorDto::default();
        desc_public.display_name = Some("Public".to_string());
        desc_public.sensitive = Some(false);
        desc_public.required = Some(false);

        let mut descriptors = HashMap::new();
        descriptors.insert("password".to_string(), Some(desc_password));
        descriptors.insert("public".to_string(), Some(desc_public));

        let mut dto = ReportingTaskDto::default();
        dto.id = Some("abc".to_string());
        dto.name = Some("PromExporter".to_string());
        dto.r#type = Some("org.x.PrometheusReportingTask".to_string());
        dto.state = Some("RUNNING".to_string());
        dto.scheduling_strategy = Some("TIMER_DRIVEN".to_string());
        dto.scheduling_period = Some("30s".to_string());
        dto.active_thread_count = Some(1);
        dto.validation_status = Some("VALID".to_string());
        dto.validation_errors = Some(vec![]);
        dto.properties = Some(props);
        dto.descriptors = Some(descriptors);

        let mut task_entity = ReportingTaskEntity::default();
        task_entity.id = Some("abc".to_string());
        task_entity.component = Some(dto);

        let mut entity = ReportingTasksEntity::default();
        entity.reporting_tasks = Some(vec![task_entity]);

        let snapshot = ReportingTasksSnapshot::from_entity(entity);
        assert_eq!(snapshot.tasks.len(), 1);
        let row = &snapshot.tasks[0];
        assert_eq!(
            row.properties.get("password"),
            Some(&None),
            "sensitive masked"
        );
        assert_eq!(
            row.properties.get("public"),
            Some(&Some("v".to_string())),
            "non-sensitive preserved"
        );
        assert_eq!(row.state, ReportingTaskState::Running);
    }

    #[test]
    fn from_entity_filters_entries_without_component() {
        use nifi_rust_client::dynamic::types::{ReportingTaskEntity, ReportingTasksEntity};

        let mut task_entity = ReportingTaskEntity::default();
        task_entity.id = Some("x".to_string());
        // component intentionally left None → unusable

        let mut entity = ReportingTasksEntity::default();
        entity.reporting_tasks = Some(vec![task_entity]);

        let snapshot = ReportingTasksSnapshot::from_entity(entity);
        assert_eq!(snapshot.tasks.len(), 0);
    }
}
