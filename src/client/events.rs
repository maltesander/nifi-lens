//! Events-tab client helpers: `POST /provenance` + poll `GET /provenance/{id}`
//! + `DELETE /provenance/{id}`.
//!
//! This module wraps NiFi's provenance-search endpoint (distinct from the
//! lineage endpoint used by Tracer). The Events tab is a cluster-wide
//! provenance search with filters for component, flowfile UUID, and
//! time range. All helpers map errors via `classify_or_fallback` so the
//! UI layer only ever sees typed `NifiLensError` variants — matching the
//! pattern established by `src/client/tracer.rs`.

use std::collections::HashMap;
use std::time::SystemTime;

use nifi_rust_client::dynamic::types::{
    ProvenanceDto, ProvenanceEntity, ProvenanceRequestDto, ProvenanceSearchValueDto,
};

use crate::client::NifiClient;
use crate::client::classify_or_fallback;
use crate::client::tracer::summary_from_dto;
use crate::error::NifiLensError;

/// Filter set for a provenance query. Empty fields mean "no filter".
///
/// The Events reducer builds one of these from the filter bar; the
/// client translates it into a NiFi `POST /provenance` request body.
#[derive(Debug, Clone, Default)]
pub struct ProvenanceQuery {
    /// Component (usually processor) identifier to match. Translated to
    /// the `ProcessorID` search term on the server.
    pub component_id: Option<String>,
    /// Flowfile UUID to match. Translated to the `FlowFileUUID` search
    /// term on the server.
    pub flow_file_uuid: Option<String>,
    /// Earliest event time to include in the query, in the server's
    /// native `MM/dd/yyyy HH:mm:ss` format or ISO-8601. Inclusive.
    pub start_time_iso: Option<String>,
    /// Latest event time to include in the query, same format as
    /// `start_time_iso`. Exclusive.
    pub end_time_iso: Option<String>,
    /// Max events to return. Server enforces the cap.
    ///
    /// **0 has a special meaning**: it sends `maxResults: 0` to the
    /// server and also suppresses the `truncated` flag in
    /// [`ProvenancePollResult::Finished`] (since the truncation
    /// comparison at poll time requires a positive cap to be
    /// meaningful). Callers building a default-valued query should
    /// set a non-zero value explicitly.
    pub max_results: u32,
}

/// Handle returned by [`NifiClient::submit_provenance_query`]. Holds
/// everything [`NifiClient::poll_provenance_query`] and
/// [`NifiClient::delete_provenance_query`] need to address the
/// in-flight query on the correct cluster node.
#[derive(Debug, Clone)]
pub struct ProvenanceQueryHandle {
    /// Server-assigned query identifier.
    pub query_id: String,
    /// In cluster mode, NiFi pins the query to a single node and
    /// subsequent poll/delete calls must carry the node id.
    pub cluster_node_id: Option<String>,
}

/// Provenance event-type classification.
///
/// NiFi's provenance API returns event types as wire strings (`"DROP"`,
/// `"RECEIVE"`, etc.). This enum centralizes the closed alphabet plus the
/// table-cell styling. The `event_type: String` field on
/// [`crate::client::ProvenanceEventSummary`] stays as the raw wire value;
/// callers that need styling parse to `EventType` at render time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    Drop,
    Expire,
    Route,
    Receive,
    Send,
    Fetch,
    Download,
    Fork,
    Join,
    Create,
    Clone,
    AttributesModified,
    ContentModified,
    /// Unrecognized or null on the wire.
    Other,
}

impl EventType {
    /// Parse a NiFi-wire event-type string (case-insensitive).
    /// Unrecognized values map to `Other`.
    pub fn from_wire(s: &str) -> Self {
        match s.to_ascii_uppercase().as_str() {
            "DROP" => Self::Drop,
            "EXPIRE" => Self::Expire,
            "ROUTE" => Self::Route,
            "RECEIVE" => Self::Receive,
            "SEND" => Self::Send,
            "FETCH" => Self::Fetch,
            "DOWNLOAD" => Self::Download,
            "FORK" => Self::Fork,
            "JOIN" => Self::Join,
            "CREATE" => Self::Create,
            "CLONE" => Self::Clone,
            "ATTRIBUTES_MODIFIED" => Self::AttributesModified,
            "CONTENT_MODIFIED" => Self::ContentModified,
            _ => Self::Other,
        }
    }

    /// Style used for the event-type cell in the Events table.
    ///
    /// - `DROP` / `EXPIRE` → error + BOLD
    /// - `ROUTE` → accent
    /// - `RECEIVE` / `SEND` / `FETCH` / `DOWNLOAD` → success
    /// - `FORK` / `JOIN` / `CREATE` / `CLONE` / `ATTRIBUTES_MODIFIED` /
    ///   `CONTENT_MODIFIED` → muted
    /// - anything else (`Other`) → default
    pub fn style(self) -> ratatui::style::Style {
        use ratatui::style::Modifier;
        match self {
            Self::Drop | Self::Expire => crate::theme::error().add_modifier(Modifier::BOLD),
            Self::Route => crate::theme::accent(),
            Self::Receive | Self::Send | Self::Fetch | Self::Download => crate::theme::success(),
            Self::Fork
            | Self::Join
            | Self::Create
            | Self::Clone
            | Self::AttributesModified
            | Self::ContentModified => crate::theme::muted(),
            Self::Other => ratatui::style::Style::default(),
        }
    }
}

/// One poll result from [`NifiClient::poll_provenance_query`]. Mirrors
/// NiFi's `finished` / `percentCompleted` fields.
#[derive(Debug, Clone)]
pub enum ProvenancePollResult {
    /// The server is still computing. `percent` is `0..=100`.
    Running { percent: u8 },
    /// The query is complete.
    Finished {
        events: Vec<crate::client::ProvenanceEventSummary>,
        fetched_at: SystemTime,
        /// True when the server reported more matching events than the
        /// `max_results` cap returned.
        truncated: bool,
    },
}

impl NifiClient {
    /// Submits a provenance search query.
    ///
    /// Maps `POST /nifi-api/provenance` with a body built from
    /// [`ProvenanceQuery`] and returns a [`ProvenanceQueryHandle`] that
    /// the caller uses to poll for completion. The `component_id` and
    /// `flow_file_uuid` fields become NiFi search terms named
    /// `ProcessorID` and `FlowFileUUID` respectively. Errors are
    /// classified via `classify_or_fallback`.
    pub async fn submit_provenance_query(
        &self,
        query: &ProvenanceQuery,
    ) -> Result<ProvenanceQueryHandle, NifiLensError> {
        tracing::debug!(
            context = %self.context_name(),
            component_id = ?query.component_id,
            flow_file_uuid = ?query.flow_file_uuid,
            start_time = ?query.start_time_iso,
            end_time = ?query.end_time_iso,
            max_results = query.max_results,
            "submitting provenance query",
        );

        let mut search_terms: HashMap<String, Option<ProvenanceSearchValueDto>> = HashMap::new();
        if let Some(id) = &query.component_id {
            let mut term = ProvenanceSearchValueDto::default();
            term.inverse = Some(false);
            term.value = Some(id.clone());
            search_terms.insert("ProcessorID".to_string(), Some(term));
        }
        if let Some(uuid) = &query.flow_file_uuid {
            let mut term = ProvenanceSearchValueDto::default();
            term.inverse = Some(false);
            term.value = Some(uuid.clone());
            search_terms.insert("FlowFileUUID".to_string(), Some(term));
        }

        let mut request = ProvenanceRequestDto::default();
        // In clustered mode NiFi rejects provenance submissions that
        // don't name a node; DynamicClient pins this at login.
        request.cluster_node_id = self.inner.cluster_node_id().map(String::from);
        request.end_date = query.end_time_iso.clone();
        request.incremental_results = Some(false);
        // saturating cast: u32::MAX > i32::MAX; clamp instead of panic
        request.max_results = Some(i32::try_from(query.max_results).unwrap_or(i32::MAX));
        request.search_terms = if search_terms.is_empty() {
            None
        } else {
            Some(search_terms)
        };
        request.start_date = query.start_time_iso.clone();
        request.summarize = Some(false);

        let mut prov_dto = ProvenanceDto::default();
        prov_dto.request = Some(request);

        // `ProvenanceEntity` is not `#[non_exhaustive]`, so we can use a
        // struct literal here. (`ProvenanceDto` / `ProvenanceRequestDto`
        // / `ProvenanceSearchValueDto` above *are* non_exhaustive and
        // must be built via `Default::default()` + field assignment.)
        let body = ProvenanceEntity {
            provenance: Some(prov_dto),
        };

        // NOTE: submit_provenance_request returns ProvenanceDto, not
        // ProvenanceEntity — unlike the lineage counterpart.
        let prov = self
            .inner
            .provenance()
            .submit_provenance_request(&body)
            .await
            .map_err(|err| {
                classify_or_fallback(self.context_name(), Box::new(err), |source| {
                    NifiLensError::ProvenanceQuerySubmitFailed {
                        context: self.context_name().to_string(),
                        source,
                    }
                })
            })?;

        let cluster_node_id = prov.request.and_then(|r| r.cluster_node_id);
        let query_id = prov
            .id
            .ok_or_else(|| NifiLensError::ProvenanceQuerySubmitFailed {
                context: self.context_name().to_string(),
                source: "server returned no query id".into(),
            })?;

        Ok(ProvenanceQueryHandle {
            query_id,
            cluster_node_id,
        })
    }

    /// Polls an in-flight provenance query.
    ///
    /// Maps `GET /nifi-api/provenance/{id}`. Returns
    /// [`ProvenancePollResult::Running`] until the server reports
    /// `finished = true`, then [`ProvenancePollResult::Finished`] with
    /// the decoded event list. Errors are classified via
    /// `classify_or_fallback`.
    pub async fn poll_provenance_query(
        &self,
        handle: &ProvenanceQueryHandle,
    ) -> Result<ProvenancePollResult, NifiLensError> {
        tracing::debug!(
            context = %self.context_name(),
            query_id = %handle.query_id,
            "polling provenance query",
        );

        let prov = self
            .inner
            .provenance()
            .get_provenance(
                &handle.query_id,
                handle.cluster_node_id.as_deref(),
                Some(false),
                Some(false),
            )
            .await
            .map_err(|err| {
                classify_or_fallback(self.context_name(), Box::new(err), |source| {
                    NifiLensError::ProvenanceQueryPollFailed {
                        context: self.context_name().to_string(),
                        query_id: handle.query_id.clone(),
                        source,
                    }
                })
            })?;

        let finished = prov.finished.unwrap_or(false);
        let percent = prov.percent_completed.unwrap_or(0).clamp(0, 100) as u8;

        if !finished {
            return Ok(ProvenancePollResult::Running { percent });
        }

        let requested_max = prov
            .request
            .as_ref()
            .and_then(|r| r.max_results)
            .unwrap_or(0);
        let results = prov.results.unwrap_or_default();
        let total = results.total_count.unwrap_or(0);
        let events = results
            .provenance_events
            .unwrap_or_default()
            .into_iter()
            .map(summary_from_dto)
            .collect::<Vec<_>>();
        // Truncated iff the server reported more matches than the
        // requested cap.
        let truncated = requested_max > 0 && total > i64::from(requested_max);

        Ok(ProvenancePollResult::Finished {
            events,
            fetched_at: SystemTime::now(),
            truncated,
        })
    }

    /// Deletes a provenance query from the NiFi server.
    ///
    /// Maps `DELETE /nifi-api/provenance/{id}`. Releases server-side
    /// resources once the caller is done reading the events. Errors are
    /// classified via `classify_or_fallback`. Delete failures are
    /// typically logged at warn level and never surfaced to the user.
    pub async fn delete_provenance_query(
        &self,
        handle: &ProvenanceQueryHandle,
    ) -> Result<(), NifiLensError> {
        tracing::debug!(
            context = %self.context_name(),
            query_id = %handle.query_id,
            "deleting provenance query",
        );

        self.inner
            .provenance()
            .delete_provenance(&handle.query_id, handle.cluster_node_id.as_deref())
            .await
            .map(|_| ())
            .map_err(|err| {
                classify_or_fallback(self.context_name(), Box::new(err), |source| {
                    NifiLensError::ProvenanceQueryDeleteFailed {
                        context: self.context_name().to_string(),
                        query_id: handle.query_id.clone(),
                        source,
                    }
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Modifier, Style};

    #[test]
    fn event_type_from_wire_case_insensitive() {
        assert_eq!(EventType::from_wire("DROP"), EventType::Drop);
        assert_eq!(EventType::from_wire("drop"), EventType::Drop);
        assert_eq!(EventType::from_wire("Drop"), EventType::Drop);
        assert_eq!(
            EventType::from_wire("attributes_modified"),
            EventType::AttributesModified,
        );
        assert_eq!(EventType::from_wire("UNKNOWN_THING"), EventType::Other);
        assert_eq!(EventType::from_wire(""), EventType::Other);
    }

    #[test]
    fn event_type_styles() {
        // DROP / EXPIRE → error + BOLD
        let err_bold = crate::theme::error().add_modifier(Modifier::BOLD);
        assert_eq!(EventType::Drop.style(), err_bold);
        assert_eq!(EventType::Expire.style(), err_bold);

        // ROUTE → accent
        assert_eq!(EventType::Route.style(), crate::theme::accent());

        // RECEIVE / SEND / FETCH / DOWNLOAD → success
        let success = crate::theme::success();
        assert_eq!(EventType::Receive.style(), success);
        assert_eq!(EventType::Send.style(), success);
        assert_eq!(EventType::Fetch.style(), success);
        assert_eq!(EventType::Download.style(), success);

        // FORK / JOIN / CREATE / CLONE / ATTRIBUTES_MODIFIED /
        // CONTENT_MODIFIED → muted
        let muted = crate::theme::muted();
        assert_eq!(EventType::Fork.style(), muted);
        assert_eq!(EventType::Join.style(), muted);
        assert_eq!(EventType::Create.style(), muted);
        assert_eq!(EventType::Clone.style(), muted);
        assert_eq!(EventType::AttributesModified.style(), muted);
        assert_eq!(EventType::ContentModified.style(), muted);

        // Other → default
        assert_eq!(EventType::Other.style(), Style::default());
    }
}
