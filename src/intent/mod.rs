//! Intent pipeline: enum + dispatcher.

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::app::state::ViewId;
use crate::client::NifiClient;
use crate::config::Config;
use crate::error::NifiLensError;
use crate::event::IntentOutcome;

#[derive(Debug, Clone)]
pub enum Intent {
    // Wired in Phase 0.
    SwitchContext(String),
    RefreshView(ViewId),
    Quit,

    // Declared for Phase 1+; dispatcher returns NotImplementedInPhase.
    OpenProcessGroup(String),
    TraceFlowfile(String), // UUID as string — Phase 4 introduces `uuid::Uuid`
    FetchEventContent { event_id: u64, side: ContentSide },
    JumpTo(CrossLink),

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
    /// From Bulletins `t`: open Tracer with a component-filtered query
    /// seeded from the bulletin's component and timestamp. Phase 4
    /// wires this.
    TraceComponent {
        component_id: String,
        since: std::time::SystemTime,
    },
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
            Intent::JumpTo(CrossLink::TraceComponent { .. }) => {
                Some(Ok(IntentOutcome::NotImplementedInPhase {
                    intent_name: intent.name(),
                    phase: 4,
                }))
            }
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

        let password = match &context.credentials {
            crate::config::Credentials::EnvVar { password_env } => std::env::var(password_env)
                .map_err(|_| NifiLensError::MissingPasswordEnv {
                    context: context.name.clone(),
                    var: password_env.clone(),
                })?,
            crate::config::Credentials::Plain { password } => password.clone(),
        };

        let resolved = crate::config::ResolvedContext {
            name: context.name.clone(),
            url: context.url.clone(),
            username: context.username.clone(),
            password,
            version_strategy: context.version_strategy,
            insecure_tls: context.insecure_tls,
            ca_cert_path: context.ca_cert_path.clone(),
        };

        let new_client = NifiClient::connect(&resolved).await?;
        let new_version = new_client.detected_version().clone();
        *self.client.write().await = new_client;
        Ok(IntentOutcome::ContextSwitched { new_version })
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
    fn cross_link_trace_component_returns_phase_4_stub() {
        use std::time::SystemTime;
        let outcome = IntentDispatcher::handle_pure(&Intent::JumpTo(CrossLink::TraceComponent {
            component_id: "proc-1".into(),
            since: SystemTime::UNIX_EPOCH,
        }))
        .expect("JumpTo must be handled by handle_pure")
        .expect("JumpTo stubs return Ok(...)");
        match outcome {
            IntentOutcome::NotImplementedInPhase { intent_name, phase } => {
                assert_eq!(intent_name, "trace component");
                assert_eq!(phase, 4);
            }
            other => panic!("expected NotImplementedInPhase, got {other:?}"),
        }
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
                since: std::time::SystemTime::UNIX_EPOCH,
            })
            .name(),
            "trace component"
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
                since: std::time::SystemTime::UNIX_EPOCH,
            })
            .is_write()
        );
    }
}
