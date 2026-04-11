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
    pub async fn dispatch(&self, intent: Intent) -> Result<IntentOutcome, NifiLensError> {
        tracing::debug!(intent = intent.name(), "dispatching");

        if intent.is_write() {
            return Err(NifiLensError::WriteIntentRefused {
                intent_name: intent.name(),
            });
        }

        match intent {
            Intent::Quit => Ok(IntentOutcome::Quitting),
            Intent::SwitchContext(name) => self.switch_context(name).await,
            Intent::RefreshView(view) => Ok(IntentOutcome::ViewRefreshed { view }),
            Intent::JumpTo(CrossLink::OpenInBrowser { .. }) => {
                Ok(IntentOutcome::NotImplementedInPhase {
                    intent_name: "jump to Browser",
                    phase: 3,
                })
            }
            Intent::JumpTo(CrossLink::TraceComponent { .. }) => {
                Ok(IntentOutcome::NotImplementedInPhase {
                    intent_name: "trace component",
                    phase: 4,
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
    use std::time::SystemTime;

    #[tokio::test]
    async fn cross_link_open_in_browser_returns_phase_3_stub() {
        let (dispatcher, client_leak) = stub_dispatcher();
        let outcome = dispatcher
            .dispatch(Intent::JumpTo(CrossLink::OpenInBrowser {
                component_id: "proc-1".into(),
                group_id: "root".into(),
            }))
            .await
            .unwrap();
        // Prevent drop of the fake Arc — it was created from a leaked Box.
        std::mem::forget(client_leak);
        match outcome {
            IntentOutcome::NotImplementedInPhase { intent_name, phase } => {
                assert_eq!(intent_name, "jump to Browser");
                assert_eq!(phase, 3);
            }
            other => panic!("expected NotImplementedInPhase, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cross_link_trace_component_returns_phase_4_stub() {
        let (dispatcher, client_leak) = stub_dispatcher();
        let outcome = dispatcher
            .dispatch(Intent::JumpTo(CrossLink::TraceComponent {
                component_id: "proc-1".into(),
                since: SystemTime::UNIX_EPOCH,
            }))
            .await
            .unwrap();
        // Prevent drop of the fake Arc — it was created from a leaked Box.
        std::mem::forget(client_leak);
        match outcome {
            IntentOutcome::NotImplementedInPhase { intent_name, phase } => {
                assert_eq!(intent_name, "trace component");
                assert_eq!(phase, 4);
            }
            other => panic!("expected NotImplementedInPhase, got {other:?}"),
        }
    }

    /// Tiny dispatcher with a real Arc<RwLock<NifiClient>> is awkward to
    /// build (requires a live login). Instead, we build the dispatcher
    /// with a stub client constructed via transmute from a leaked raw
    /// allocation. The caller must `std::mem::forget` the returned
    /// `Arc<RwLock<NifiClient>>` handle to prevent a double-free.
    ///
    /// SAFETY: the dispatch paths exercised by these tests never read
    /// the client field. If a future test calls a non-JumpTo arm, it
    /// must build a real dispatcher.
    fn stub_dispatcher() -> (IntentDispatcher, Arc<RwLock<u8>>) {
        use crate::config::Config;
        // Allocate a real Arc<RwLock<u8>> (trivially constructible), then
        // transmute it to Arc<RwLock<NifiClient>>. The Arc internals are
        // identical regardless of T (same pointer / refcount layout), and we
        // guarantee never to deref the inner value.  We return the original
        // Arc<RwLock<u8>> so the caller can `forget` it, avoiding the
        // double-free that would result from dropping both the transmuted copy
        // and the original.
        let real: Arc<RwLock<u8>> = Arc::new(RwLock::new(0u8));
        // Clone to get a second handle; the caller will forget this clone.
        let caller_handle = Arc::clone(&real);
        // SAFETY: Arc<RwLock<u8>> and Arc<RwLock<NifiClient>> have the same
        // representation. We never deref the NifiClient side; we only store
        // it and drop it (via forget) without touching the payload.
        let client: Arc<RwLock<crate::client::NifiClient>> = unsafe { std::mem::transmute(real) }; // same repr; payload never deref'd
        let config = Arc::new(Config {
            current_context: "dev".into(),
            bulletins: Default::default(),
            contexts: vec![],
        });
        (IntentDispatcher { client, config }, caller_handle)
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
