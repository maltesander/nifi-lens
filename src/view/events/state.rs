//! Pure state for the Events tab.

use std::time::SystemTime;

/// Which filter field is currently being edited, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterField {
    /// `t` — time range (e.g. `"last 15m"`, `"last 1h"`, ISO-8601 range).
    Time,
    /// `T` — event type list (comma-separated, e.g. `"DROP,EXPIRE"`).
    Types,
    /// `s` — source component (id or display name).
    Source,
    /// `u` — flowfile UUID.
    Uuid,
    /// `a` — attribute filter (`key=value`).
    Attr,
}

impl FilterField {
    pub fn key(self) -> char {
        match self {
            Self::Time => 't',
            Self::Types => 'T',
            Self::Source => 's',
            Self::Uuid => 'u',
            Self::Attr => 'a',
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Time => "time",
            Self::Types => "type",
            Self::Source => "source",
            Self::Uuid => "file uuid",
            Self::Attr => "attr",
        }
    }
}

/// Filter state for a provenance query. Empty strings mean "no filter"
/// (treated as server-side wildcard / default).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventsFilters {
    pub time: String,
    pub types: String,
    pub source: String,
    pub uuid: String,
    pub attr: String,
}

impl Default for EventsFilters {
    fn default() -> Self {
        Self {
            time: "last 15m".to_string(),
            types: String::new(),
            source: String::new(),
            uuid: String::new(),
            attr: String::new(),
        }
    }
}

impl EventsFilters {
    /// Immutable field accessor matching `FilterField`.
    pub fn get(&self, field: FilterField) -> &str {
        match field {
            FilterField::Time => &self.time,
            FilterField::Types => &self.types,
            FilterField::Source => &self.source,
            FilterField::Uuid => &self.uuid,
            FilterField::Attr => &self.attr,
        }
    }

    /// Mutable field accessor.
    pub fn get_mut(&mut self, field: FilterField) -> &mut String {
        match field {
            FilterField::Time => &mut self.time,
            FilterField::Types => &mut self.types,
            FilterField::Source => &mut self.source,
            FilterField::Uuid => &mut self.uuid,
            FilterField::Attr => &mut self.attr,
        }
    }
}

/// Status of the current provenance query.
#[derive(Debug, Clone)]
pub enum EventsQueryStatus {
    /// No query has run yet, or results have been cleared.
    Idle,
    /// Query is in flight. `submitted_at` is wall-clock.
    Running {
        query_id: Option<String>,
        submitted_at: SystemTime,
        percent: u8,
    },
    /// Query completed successfully.
    Done {
        fetched_at: SystemTime,
        truncated: bool,
        took_ms: u64,
    },
    /// Query failed. The error message is shown in the banner and
    /// as a `status ● failed` chip in the filter bar.
    Failed { error: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_field_labels_match_spec() {
        assert_eq!(FilterField::Time.label(), "time");
        assert_eq!(FilterField::Types.label(), "type");
        assert_eq!(FilterField::Source.label(), "source");
        assert_eq!(FilterField::Uuid.label(), "file uuid");
        assert_eq!(FilterField::Attr.label(), "attr");
    }

    #[test]
    fn filter_field_keys_match_spec() {
        assert_eq!(FilterField::Time.key(), 't');
        assert_eq!(FilterField::Types.key(), 'T');
        assert_eq!(FilterField::Source.key(), 's');
        assert_eq!(FilterField::Uuid.key(), 'u');
        assert_eq!(FilterField::Attr.key(), 'a');
    }

    #[test]
    fn events_filters_default_is_last_15m() {
        let f = EventsFilters::default();
        assert_eq!(f.time, "last 15m");
        assert!(f.types.is_empty());
        assert!(f.source.is_empty());
        assert!(f.uuid.is_empty());
        assert!(f.attr.is_empty());
    }

    #[test]
    fn events_filters_get_and_get_mut_round_trip() {
        let mut f = EventsFilters::default();
        *f.get_mut(FilterField::Source) = "proc-1".into();
        assert_eq!(f.get(FilterField::Source), "proc-1");
    }
}
