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
        match raw {
            "RUNNING" => Self::Running,
            "STOPPED" => Self::Stopped,
            "DISABLED" => Self::Disabled,
            other => {
                tracing::warn!(value = %other, "unknown reporting-task state");
                Self::Stopped
            }
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
        match raw {
            "VALID" => Self::Valid,
            "INVALID" => Self::Invalid,
            "VALIDATING" => Self::Validating,
            other => {
                tracing::warn!(value = %other, "unknown reporting-task validation status");
                Self::Validating
            }
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
