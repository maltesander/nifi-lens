//! App run loop, terminal guard, and panic hook.

pub mod history;
pub(crate) mod navigation;
pub mod state;
pub mod ui;
pub mod worker;

use std::io::Stdout;
use std::sync::Arc;

use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::{RwLock, mpsc};

use crate::app::state::{AppState, PendingIntent, ViewId, update};
use crate::app::worker::WorkerRegistry;
use crate::client::NifiClient;
use crate::config::Config;
use crate::error::NifiLensError;
use crate::event::AppEvent;
use crate::intent::{Intent, IntentDispatcher};
use crate::logging::StderrToggle;

pub async fn run(
    client: NifiClient,
    config: Config,
    stderr_toggle: StderrToggle,
) -> Result<(), NifiLensError> {
    let detected_version = client.detected_version().clone();
    let context_name = client.context_name().to_string();
    let client = Arc::new(RwLock::new(client));
    let config = Arc::new(config);

    let (tx, mut rx) = mpsc::channel::<AppEvent>(256);
    spawn_input_task(tx.clone());
    spawn_tick_task(tx.clone());

    stderr_toggle.suppress();
    let _terminal_guard = TerminalGuard::enter(stderr_toggle.clone())?;
    install_panic_hook(stderr_toggle.clone());
    let mut terminal = build_terminal()?;

    let mut state = AppState::new(context_name, detected_version, &config);
    // Spawn per-endpoint cluster fetchers once at startup. A context
    // switch tears the store down and respawns inside the main loop's
    // `pending_worker_restart` branch.
    state.cluster.spawn_fetchers(client.clone(), tx.clone());
    let mut workers = WorkerRegistry::new();
    let bulletins_last_id = state.bulletins.last_id;
    workers.ensure(
        state.current_tab,
        &client,
        &tx,
        bulletins_last_id,
        &mut state.browser,
        &mut state.cluster,
        &state.polling,
    );

    let dispatcher = Arc::new(IntentDispatcher {
        client: client.clone(),
        config: config.clone(),
        tx: tx.clone(),
    });

    terminal
        .draw(|f| ui::render(f, &state))
        .map_err(|source| NifiLensError::TerminalInit { source })?;

    while let Some(event) = rx.recv().await {
        // Cluster-store events are owned by the main loop, not the
        // per-view reducer: `update_inner` is synchronous and cannot
        // send a follow-up `ClusterChanged` via `tx`. Intercept those
        // variants here, apply them to `AppState.cluster`, and fan out
        // `ClusterChanged` on the same channel so view reducers can
        // re-derive (Task 1 is a no-op; Tasks 3/5/7 wire reducers).
        let event = match event {
            AppEvent::ClusterUpdate(update) => {
                let endpoint = state.cluster.apply_update(update);
                if tx.send(AppEvent::ClusterChanged(endpoint)).await.is_err() {
                    tracing::debug!("channel closed during ClusterChanged fanout");
                }
                continue;
            }
            AppEvent::ClusterChanged(endpoint) => {
                let mut affects_overview = false;
                match endpoint {
                    crate::cluster::ClusterEndpoint::RootPgStatus => {
                        crate::view::overview::state::redraw_components(&mut state);
                        // Task 6 wires the Browser arena recompute here.
                        affects_overview = true;
                    }
                    _ => tracing::trace!(?endpoint, "cluster changed"),
                }
                // Only redraw when the active tab's projection actually
                // depends on the changed endpoint. Other tabs don't read
                // snapshot data yet (Task 6+ opens that door for Browser).
                if affects_overview && state.current_tab == ViewId::Overview {
                    terminal
                        .draw(|f| ui::render(f, &state))
                        .map_err(|source| NifiLensError::TerminalInit { source })?;
                }
                continue;
            }
            other => other,
        };
        let result = update(&mut state, event, &config);

        // Dispatch tracer followups (e.g. delete a consumed lineage query).
        if let Some(followup) = result.tracer_followup {
            match followup {
                crate::view::tracer::state::Followup::DeleteLineageQuery {
                    query_id,
                    cluster_node_id,
                } => {
                    let dispatcher = dispatcher.clone();
                    let tx = tx.clone();
                    tokio::task::spawn_local(async move {
                        let outcome = dispatcher
                            .dispatch(Intent::DeleteLineageQuery {
                                query_id,
                                cluster_node_id,
                            })
                            .await;
                        let _ = tx.send(AppEvent::IntentOutcome(outcome)).await;
                    });
                }
            }
        }

        if let Some(pending) = result.intent {
            match pending {
                PendingIntent::SaveEventContent(save) => {
                    crate::view::tracer::worker::spawn_save(
                        client.clone(),
                        tx.clone(),
                        save.path,
                        save.event_id,
                        save.side,
                    );
                }
                other => {
                    let intent = match other {
                        PendingIntent::SwitchContext(name) => Some(Intent::SwitchContext(name)),
                        PendingIntent::Goto(link) => Some(Intent::Goto(link)),
                        PendingIntent::Dispatch(intent) => Some(intent),
                        PendingIntent::RunProvenanceQuery { query } => {
                            Some(Intent::RunProvenanceQuery { query })
                        }
                        PendingIntent::Quit => Some(Intent::Quit),
                        _ => {
                            tracing::warn!("unhandled PendingIntent variant");
                            None
                        }
                    };
                    if let Some(intent) = intent {
                        let dispatcher = dispatcher.clone();
                        let tx = tx.clone();
                        tokio::task::spawn_local(async move {
                            let outcome = dispatcher.dispatch(intent).await;
                            let _ = tx.send(AppEvent::IntentOutcome(outcome)).await;
                        });
                    }
                }
            }
        }

        if state.should_quit {
            break;
        }

        if result.redraw {
            terminal
                .draw(|f| ui::render(f, &state))
                .map_err(|source| NifiLensError::TerminalInit { source })?;
        }

        if state.pending_worker_restart {
            workers.invalidate();
            state.pending_worker_restart = false;
            state.cluster.spawn_fetchers(client.clone(), tx.clone());
        }
        let bulletins_last_id = state.bulletins.last_id;
        workers.ensure(
            state.current_tab,
            &client,
            &tx,
            bulletins_last_id,
            &mut state.browser,
            &mut state.cluster,
            &state.polling,
        );

        // After ensure(), re-emit any pending Browser detail request that
        // was dropped because the worker (and detail_tx) didn't exist yet
        // — e.g. when a cross-link lands on Browser from another tab.
        if state.browser.pending_detail_unsent && state.browser.detail_tx.is_some() {
            state.browser.emit_detail_request_for_current_selection();
        }
    }

    workers.shutdown();
    Ok(())
}

fn build_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>, NifiLensError> {
    let backend = CrosstermBackend::new(std::io::stdout());
    Terminal::new(backend).map_err(|source| NifiLensError::TerminalInit { source })
}

fn spawn_input_task(tx: mpsc::Sender<AppEvent>) {
    tokio::spawn(async move {
        loop {
            if let Ok(true) = crossterm::event::poll(std::time::Duration::from_millis(100))
                && let Ok(event) = crossterm::event::read()
                && tx.send(AppEvent::Input(event)).await.is_err()
            {
                return;
            }
        }
    });
}

fn spawn_tick_task(tx: mpsc::Sender<AppEvent>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            interval.tick().await;
            if tx.send(AppEvent::Tick).await.is_err() {
                return;
            }
        }
    });
}

struct TerminalGuard {
    stderr_toggle: StderrToggle,
}

impl TerminalGuard {
    fn enter(stderr_toggle: StderrToggle) -> Result<Self, NifiLensError> {
        enable_raw_mode().map_err(|source| NifiLensError::TerminalInit { source })?;
        execute!(std::io::stdout(), EnterAlternateScreen)
            .map_err(|source| NifiLensError::TerminalInit { source })?;
        Ok(Self { stderr_toggle })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        self.stderr_toggle.restore();
    }
}

fn install_panic_hook(stderr_toggle: StderrToggle) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        stderr_toggle.restore();
        previous(info);
    }));
}
