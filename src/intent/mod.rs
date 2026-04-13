//! Intent pipeline: enum + dispatcher.

use std::sync::Arc;

use tokio::sync::{RwLock, mpsc};

use crate::app::state::ViewId;
use crate::client::NifiClient;
use crate::config::Config;
use crate::error::NifiLensError;
use crate::event::{AppEvent, IntentOutcome};

#[derive(Debug, Clone)]
pub enum Intent {
    // Wired in Phase 0.
    SwitchContext(String),
    RefreshView(ViewId),
    Quit,

    // Declared for Phase 1+; dispatcher returns NotImplementedInPhase.
    OpenProcessGroup(String),
    TraceFlowfile(String), // UUID as string — Phase 4 introduces `uuid::Uuid`
    FetchEventContent {
        event_id: i64,
        side: ContentSide,
    },
    JumpTo(CrossLink),

    // Phase 4 intents.
    CancelLineageQuery,
    DeleteLineageQuery {
        query_id: String,
        cluster_node_id: Option<String>,
    },
    LoadEventDetail {
        event_id: i64,
    },
    RefreshLatestEvents {
        component_id: String,
    },
    RefreshLineage {
        uuid: String,
    },

    // Phase 6 events intent.
    RunProvenanceQuery {
        query: crate::client::ProvenanceQuery,
    },

    // Write intents — declared; dispatcher refuses unconditionally in Phase 0.
    StartProcessor(String),
    StopProcessor(String),
    EnableControllerService(String),
    DisableControllerService(String),
    EmptyQueue(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentSide {
    Input,
    Output,
}

#[derive(Debug, Clone)]
pub enum CrossLink {
    /// From Bulletins `Enter`: open Browser on the selected bulletin's
    /// component. Phase 3 wires this.
    OpenInBrowser {
        component_id: String,
        group_id: String,
    },
    /// From Bulletins/Browser `t` (pre-Phase 6): Tracer latest-events
    /// landing with a component-filtered query.
    ///
    /// Phase 6 retargets Bulletins/Browser `t` to
    /// [`CrossLink::JumpToEvents`] instead. This variant is retained
    /// for backwards compatibility and is still wired through the
    /// dispatcher; future phases may prune it.
    TraceComponent { component_id: String },
    /// From Bulletins/Browser `t` (Phase 6+): open Events pre-filled
    /// with a component-sourced query and a 15-minute time window.
    JumpToEvents { component_id: String },
    /// From Events result row `t`: open Tracer and auto-run a lineage
    /// query on the selected event's flowfile uuid.
    TraceByUuid { uuid: String },
}

impl Intent {
    pub fn name(&self) -> &'static str {
        match self {
            Self::SwitchContext(_) => "SwitchContext",
            Self::RefreshView(_) => "RefreshView",
            Self::Quit => "Quit",
            Self::OpenProcessGroup(_) => "OpenProcessGroup",
            Self::TraceFlowfile(_) => "TraceFlowfile",
            Self::FetchEventContent { .. } => "FetchEventContent",
            Self::JumpTo(CrossLink::OpenInBrowser { .. }) => "jump to Browser",
            Self::JumpTo(CrossLink::TraceComponent { .. }) => "trace component",
            Self::JumpTo(CrossLink::JumpToEvents { .. }) => "jump to Events",
            Self::JumpTo(CrossLink::TraceByUuid { .. }) => "trace by uuid",
            Self::CancelLineageQuery => "CancelLineageQuery",
            Self::DeleteLineageQuery { .. } => "DeleteLineageQuery",
            Self::LoadEventDetail { .. } => "LoadEventDetail",
            Self::RefreshLatestEvents { .. } => "RefreshLatestEvents",
            Self::RefreshLineage { .. } => "RefreshLineage",
            Self::RunProvenanceQuery { .. } => "run provenance query",
            Self::StartProcessor(_) => "StartProcessor",
            Self::StopProcessor(_) => "StopProcessor",
            Self::EnableControllerService(_) => "EnableControllerService",
            Self::DisableControllerService(_) => "DisableControllerService",
            Self::EmptyQueue(_) => "EmptyQueue",
        }
    }

    pub fn is_write(&self) -> bool {
        matches!(
            self,
            Self::StartProcessor(_)
                | Self::StopProcessor(_)
                | Self::EnableControllerService(_)
                | Self::DisableControllerService(_)
                | Self::EmptyQueue(_)
        )
    }
}

pub struct IntentDispatcher {
    pub client: Arc<RwLock<NifiClient>>,
    pub config: Arc<Config>,
    pub tx: mpsc::Sender<AppEvent>,
}

impl IntentDispatcher {
    /// Intent arms that don't touch the client. Factored out so tests
    /// can exercise them without building a dispatcher (which would
    /// otherwise require a live NifiClient). Returns `None` for any
    /// intent that needs the client.
    fn handle_pure(intent: &Intent) -> Option<Result<IntentOutcome, NifiLensError>> {
        if intent.is_write() {
            return Some(Err(NifiLensError::WriteIntentRefused {
                intent_name: intent.name(),
            }));
        }
        match intent {
            Intent::Quit => Some(Ok(IntentOutcome::Quitting)),
            Intent::RefreshView(view) => Some(Ok(IntentOutcome::ViewRefreshed { view: *view })),
            Intent::JumpTo(CrossLink::OpenInBrowser {
                component_id,
                group_id,
            }) => Some(Ok(IntentOutcome::OpenInBrowserTarget {
                component_id: component_id.clone(),
                group_id: group_id.clone(),
            })),
            // TraceComponent is dispatched in `dispatch()` to spawn the
            // latest-events worker alongside the tab switch.
            Intent::JumpTo(CrossLink::TraceComponent { .. }) => None,
            Intent::JumpTo(CrossLink::JumpToEvents { component_id }) => {
                Some(Ok(IntentOutcome::EventsLandingOn {
                    component_id: component_id.clone(),
                }))
            }
            // TraceByUuid spawns the lineage worker; not pure.
            Intent::JumpTo(CrossLink::TraceByUuid { .. }) => None,
            _ => None,
        }
    }

    pub async fn dispatch(&self, intent: Intent) -> Result<IntentOutcome, NifiLensError> {
        tracing::debug!(intent = intent.name(), "dispatching");

        if let Some(outcome) = Self::handle_pure(&intent) {
            return outcome;
        }

        match intent {
            Intent::SwitchContext(name) => self.switch_context(name).await,

            // --- Phase 4 tracer intents ---
            Intent::JumpTo(CrossLink::TraceComponent { component_id }) => {
                crate::view::tracer::worker::spawn_latest_events(
                    self.client.clone(),
                    self.tx.clone(),
                    component_id.clone(),
                );
                Ok(IntentOutcome::TracerLandingOn { component_id })
            }
            Intent::JumpTo(CrossLink::TraceByUuid { uuid }) => {
                let handle = crate::view::tracer::worker::spawn_lineage(
                    self.client.clone(),
                    self.tx.clone(),
                    uuid.clone(),
                );
                Ok(IntentOutcome::TracerLineageStarted {
                    uuid,
                    abort: handle.abort_handle(),
                })
            }
            Intent::TraceFlowfile(uuid) => {
                let handle = crate::view::tracer::worker::spawn_lineage(
                    self.client.clone(),
                    self.tx.clone(),
                    uuid.clone(),
                );
                Ok(IntentOutcome::TracerLineageStarted {
                    uuid,
                    abort: handle.abort_handle(),
                })
            }
            Intent::RefreshLatestEvents { component_id } => {
                crate::view::tracer::worker::spawn_latest_events(
                    self.client.clone(),
                    self.tx.clone(),
                    component_id,
                );
                Ok(IntentOutcome::ViewRefreshed {
                    view: ViewId::Tracer,
                })
            }
            Intent::RefreshLineage { uuid } => {
                let handle = crate::view::tracer::worker::spawn_lineage(
                    self.client.clone(),
                    self.tx.clone(),
                    uuid.clone(),
                );
                Ok(IntentOutcome::TracerLineageStarted {
                    uuid,
                    abort: handle.abort_handle(),
                })
            }
            Intent::LoadEventDetail { event_id } => {
                crate::view::tracer::worker::spawn_event_detail(
                    self.client.clone(),
                    self.tx.clone(),
                    event_id,
                );
                Ok(IntentOutcome::ViewRefreshed {
                    view: ViewId::Tracer,
                })
            }
            Intent::FetchEventContent { event_id, side } => {
                let client_side = match side {
                    ContentSide::Input => crate::client::ContentSide::Input,
                    ContentSide::Output => crate::client::ContentSide::Output,
                };
                crate::view::tracer::worker::spawn_content(
                    self.client.clone(),
                    self.tx.clone(),
                    event_id,
                    client_side,
                );
                Ok(IntentOutcome::ViewRefreshed {
                    view: ViewId::Tracer,
                })
            }
            Intent::DeleteLineageQuery {
                query_id,
                cluster_node_id,
            } => {
                crate::view::tracer::worker::spawn_delete_lineage(
                    self.client.clone(),
                    query_id,
                    cluster_node_id,
                );
                Ok(IntentOutcome::ViewRefreshed {
                    view: ViewId::Tracer,
                })
            }
            Intent::CancelLineageQuery => Ok(IntentOutcome::ViewRefreshed {
                view: ViewId::Tracer,
            }),

            // --- Phase 6 events intent ---
            Intent::RunProvenanceQuery { query } => {
                let _handle = crate::view::events::worker::spawn_query(
                    self.client.clone(),
                    self.tx.clone(),
                    query,
                );
                // Fire-and-forget; the worker emits its own payloads.
                Ok(IntentOutcome::ViewRefreshed {
                    view: ViewId::Events,
                })
            }

            other => Ok(IntentOutcome::NotImplementedInPhase {
                intent_name: other.name(),
                phase: 0,
            }),
        }
    }

    async fn switch_context(&self, name: String) -> Result<IntentOutcome, NifiLensError> {
        // Resolve the target context from our Arc<Config>. Env-var credentials
        // are resolved here, same as loader::load does on startup.
        let context = self
            .config
            .contexts
            .iter()
            .find(|c| c.name == name)
            .ok_or_else(|| NifiLensError::UnknownContext {
                name: name.clone(),
                available: self
                    .config
                    .contexts
                    .iter()
                    .map(|c| c.name.clone())
                    .collect(),
            })?;

        let auth = crate::config::loader::resolve_auth(&context.name, &context.auth)?;

        let resolved = crate::config::ResolvedContext {
            name: context.name.clone(),
            url: context.url.clone(),
            auth,
            version_strategy: context.version_strategy,
            insecure_tls: context.insecure_tls,
            ca_cert_path: context.ca_cert_path.clone(),
            proxied_entities_chain: context.proxied_entities_chain.clone(),
        };

        let new_context_name = resolved.name.clone();
        let new_client = NifiClient::connect(&resolved).await?;
        let new_version = new_client.detected_version().clone();
        *self.client.write().await = new_client;
        Ok(IntentOutcome::ContextSwitched {
            new_context_name,
            new_version,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::event::IntentOutcome;

    #[test]
    fn cross_link_open_in_browser_returns_target_outcome() {
        let outcome = IntentDispatcher::handle_pure(&Intent::JumpTo(CrossLink::OpenInBrowser {
            component_id: "proc-1".into(),
            group_id: "root".into(),
        }))
        .expect("JumpTo must be handled by handle_pure")
        .expect("JumpTo returns Ok(...)");
        match outcome {
            IntentOutcome::OpenInBrowserTarget {
                component_id,
                group_id,
            } => {
                assert_eq!(component_id, "proc-1");
                assert_eq!(group_id, "root");
            }
            other => panic!("expected OpenInBrowserTarget, got {other:?}"),
        }
    }

    #[test]
    fn cross_link_trace_component_not_handled_by_handle_pure() {
        // TraceComponent is dispatched in `dispatch()` (not `handle_pure`)
        // because it needs to spawn the latest-events worker.
        let result = IntentDispatcher::handle_pure(&Intent::JumpTo(CrossLink::TraceComponent {
            component_id: "proc-1".into(),
        }));
        assert!(
            result.is_none(),
            "TraceComponent should not be handled by handle_pure"
        );
    }

    #[test]
    fn name_for_each_variant() {
        assert_eq!(Intent::Quit.name(), "Quit");
        assert_eq!(Intent::RefreshView(ViewId::Overview).name(), "RefreshView");
        assert_eq!(Intent::StartProcessor("x".into()).name(), "StartProcessor");
        assert_eq!(Intent::SwitchContext("dev".into()).name(), "SwitchContext");
        assert_eq!(
            Intent::OpenProcessGroup("x".into()).name(),
            "OpenProcessGroup"
        );
        assert_eq!(Intent::TraceFlowfile("uuid".into()).name(), "TraceFlowfile");
        assert_eq!(
            Intent::FetchEventContent {
                event_id: 1,
                side: ContentSide::Input
            }
            .name(),
            "FetchEventContent"
        );
        assert_eq!(
            Intent::JumpTo(CrossLink::OpenInBrowser {
                component_id: "x".into(),
                group_id: "root".into(),
            })
            .name(),
            "jump to Browser"
        );
        assert_eq!(
            Intent::JumpTo(CrossLink::TraceComponent {
                component_id: "x".into(),
            })
            .name(),
            "trace component"
        );
        assert_eq!(Intent::CancelLineageQuery.name(), "CancelLineageQuery");
        assert_eq!(
            Intent::DeleteLineageQuery {
                query_id: "q1".into(),
                cluster_node_id: None,
            }
            .name(),
            "DeleteLineageQuery"
        );
        assert_eq!(
            Intent::LoadEventDetail { event_id: 1 }.name(),
            "LoadEventDetail"
        );
        assert_eq!(
            Intent::RefreshLatestEvents {
                component_id: "x".into()
            }
            .name(),
            "RefreshLatestEvents"
        );
        assert_eq!(
            Intent::RefreshLineage { uuid: "u".into() }.name(),
            "RefreshLineage"
        );
        assert_eq!(Intent::StopProcessor("x".into()).name(), "StopProcessor");
        assert_eq!(
            Intent::EnableControllerService("x".into()).name(),
            "EnableControllerService"
        );
        assert_eq!(
            Intent::DisableControllerService("x".into()).name(),
            "DisableControllerService"
        );
        assert_eq!(Intent::EmptyQueue("x".into()).name(), "EmptyQueue");
    }

    #[test]
    fn handle_pure_returns_events_landing_on_for_jump_to_events() {
        let outcome = IntentDispatcher::handle_pure(&Intent::JumpTo(CrossLink::JumpToEvents {
            component_id: "proc-42".into(),
        }))
        .expect("arm is pure")
        .expect("no error");
        match outcome {
            IntentOutcome::EventsLandingOn { component_id } => {
                assert_eq!(component_id, "proc-42");
            }
            other => panic!("expected EventsLandingOn, got {other:?}"),
        }
    }

    #[test]
    fn handle_pure_returns_none_for_trace_by_uuid() {
        // TraceByUuid requires the worker to spawn, so it's not pure.
        let outcome = IntentDispatcher::handle_pure(&Intent::JumpTo(CrossLink::TraceByUuid {
            uuid: "abc-123".into(),
        }));
        assert!(outcome.is_none(), "TraceByUuid is not pure; must dispatch");
    }

    #[test]
    fn is_write_detects_write_variants() {
        assert!(Intent::StartProcessor("x".into()).is_write());
        assert!(Intent::StopProcessor("x".into()).is_write());
        assert!(Intent::EnableControllerService("x".into()).is_write());
        assert!(Intent::DisableControllerService("x".into()).is_write());
        assert!(Intent::EmptyQueue("x".into()).is_write());
        assert!(!Intent::Quit.is_write());
        assert!(!Intent::RefreshView(ViewId::Overview).is_write());
        assert!(!Intent::SwitchContext("dev".into()).is_write());
        assert!(!Intent::OpenProcessGroup("x".into()).is_write());
        assert!(!Intent::TraceFlowfile("uuid".into()).is_write());
        assert!(
            !Intent::FetchEventContent {
                event_id: 1,
                side: ContentSide::Output
            }
            .is_write()
        );
        assert!(
            !Intent::JumpTo(CrossLink::OpenInBrowser {
                component_id: "x".into(),
                group_id: "root".into(),
            })
            .is_write()
        );
        assert!(
            !Intent::JumpTo(CrossLink::TraceComponent {
                component_id: "x".into(),
            })
            .is_write()
        );
        assert!(!Intent::CancelLineageQuery.is_write());
        assert!(
            !Intent::DeleteLineageQuery {
                query_id: "q1".into(),
                cluster_node_id: None,
            }
            .is_write()
        );
        assert!(!Intent::LoadEventDetail { event_id: 1 }.is_write());
        assert!(
            !Intent::RefreshLatestEvents {
                component_id: "x".into()
            }
            .is_write()
        );
        assert!(!Intent::RefreshLineage { uuid: "u".into() }.is_write());
    }
}
