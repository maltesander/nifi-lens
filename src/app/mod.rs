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

/// Total slot count of the central `AppEvent` mpsc channel. Producers
/// across input/tick/cluster-fetcher/worker/intent paths feed into it
/// and the single UI loop drains it. Sized for bursty cluster fanouts
/// without backpressuring producers under healthy render cadence.
const APP_EVENT_CHANNEL_CAPACITY: usize = 256;

/// Threshold for the saturation watchdog. When fewer than this many
/// slots remain free, the watchdog emits a `tracing::warn!` once per
/// second until capacity recovers.
const APP_EVENT_CHANNEL_LOW_WATER: usize = 16;

pub async fn run(
    client: NifiClient,
    config: Config,
    stderr_toggle: StderrToggle,
) -> Result<(), NifiLensError> {
    let detected_version = client.detected_version().clone();
    let context_name = client.context_name().to_string();
    let base_url = client.base_url().to_string();
    let client = Arc::new(RwLock::new(client));
    let config = Arc::new(config);

    let (tx, mut rx) = mpsc::channel::<AppEvent>(APP_EVENT_CHANNEL_CAPACITY);
    spawn_input_task(tx.clone());
    spawn_tick_task(tx.clone());
    spawn_channel_saturation_watchdog(
        tx.clone(),
        APP_EVENT_CHANNEL_LOW_WATER,
        APP_EVENT_CHANNEL_CAPACITY,
    );

    stderr_toggle.suppress();
    let _terminal_guard = TerminalGuard::enter(stderr_toggle.clone())?;
    install_panic_hook(stderr_toggle.clone());
    let mut terminal = build_terminal()?;

    let mut state = AppState::new(context_name, detected_version, &config, base_url);
    // Spawn per-endpoint cluster fetchers once at startup. A context
    // switch tears the store down and respawns inside the main loop's
    // `pending_worker_restart` branch.
    state.cluster.spawn_fetchers(client.clone(), tx.clone());
    let mut workers = WorkerRegistry::new();
    workers.ensure(
        state.current_tab,
        &client,
        &tx,
        &mut state.browser,
        &mut state.cluster,
    );

    let dispatcher = Arc::new(IntentDispatcher {
        client: client.clone(),
        config: config.clone(),
        tx: tx.clone(),
    });

    terminal
        .draw(|f| ui::render(f, &mut state))
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
                // A fresh RootPgStatus publishes its PG-id list on the
                // watch channel that feeds the connections-by-PG fetcher.
                // Kept here (not in the `ClusterChanged` branch) so it
                // fires on every update — including ones that don't
                // affect the active tab's projection.
                if matches!(endpoint, crate::cluster::ClusterEndpoint::RootPgStatus) {
                    state.cluster.publish_pg_ids();
                }
                if matches!(
                    endpoint,
                    crate::cluster::ClusterEndpoint::ClusterNodes
                        | crate::cluster::ClusterEndpoint::SystemDiagnostics
                ) {
                    state.cluster.publish_node_addresses();
                }
                if tx.send(AppEvent::ClusterChanged(endpoint)).await.is_err() {
                    tracing::debug!("channel closed during ClusterChanged fanout");
                }
                continue;
            }
            AppEvent::ClusterChanged(endpoint) => {
                use crate::cluster::ClusterEndpoint;
                // Overview cares about eight cluster endpoints (all read-
                // model fields plus Bulletins for the sparkline +
                // noisy-components leaderboard). Task 8 added
                // ControllerStatus, SystemDiagnostics, and About —
                // previously Overview-worker-owned. Task 16/17 adds
                // ClusterNodes for the per-node membership join.
                // Task 15 adds TlsCerts for per-node cert expiry.
                let affects_overview = matches!(
                    endpoint,
                    ClusterEndpoint::RootPgStatus
                        | ClusterEndpoint::ControllerServices
                        | ClusterEndpoint::ControllerStatus
                        | ClusterEndpoint::SystemDiagnostics
                        | ClusterEndpoint::About
                        | ClusterEndpoint::Bulletins
                        | ClusterEndpoint::ClusterNodes
                        | ClusterEndpoint::TlsCerts
                );
                // `VersionControl` is intentionally NOT in this set — Task 13 wires it
                // via a separate `affects_browser_version_control` flag that triggers
                // the cheap `redraw_version_control` re-stamp (FlowIndex.version_state)
                // rather than a full arena rebuild.
                let affects_browser = matches!(
                    endpoint,
                    ClusterEndpoint::RootPgStatus
                        | ClusterEndpoint::ControllerServices
                        | ClusterEndpoint::ConnectionsByPg
                );
                let affects_bulletins = matches!(endpoint, ClusterEndpoint::Bulletins);
                let affects_browser_version_control =
                    matches!(endpoint, ClusterEndpoint::VersionControl);
                let affects_browser_parameter_context_bindings =
                    matches!(endpoint, ClusterEndpoint::ParameterContextBindings);

                if affects_overview {
                    match endpoint {
                        ClusterEndpoint::RootPgStatus | ClusterEndpoint::ControllerServices => {
                            crate::view::overview::state::redraw_components(&mut state);
                        }
                        ClusterEndpoint::ControllerStatus => {
                            // Also affects the Components panel
                            // (process-groups row reads
                            // stale/modified/sync_err), so refresh
                            // components too.
                            crate::view::overview::state::redraw_controller_status(&mut state);
                            crate::view::overview::state::redraw_components(&mut state);
                        }
                        ClusterEndpoint::SystemDiagnostics => {
                            crate::view::overview::state::redraw_sysdiag(&mut state);
                        }
                        ClusterEndpoint::ClusterNodes => {
                            crate::view::overview::state::redraw_cluster_nodes(&mut state);
                        }
                        ClusterEndpoint::TlsCerts => {
                            crate::view::overview::state::redraw_cluster_nodes(&mut state);
                        }
                        ClusterEndpoint::About => {
                            // `About` has no OverviewState mirror today
                            // (the NiFi version isn't rendered in
                            // Overview). Keep the arm so a future
                            // renderer addition has a wired redraw path
                            // without a second `ClusterChanged` touch.
                        }
                        ClusterEndpoint::Bulletins => {
                            crate::view::overview::state::redraw_bulletin_projections(&mut state);
                        }
                        _ => {}
                    }
                }
                // Browser arena rebuild is gated on Browser actually
                // being the active tab. On a 10k-processor cluster this
                // avoids cloning a multi-MB snapshot every 10s once
                // Browser has been visited — Overview subscribes to
                // `RootPgStatus` too, so the raw subscriber count isn't
                // a Browser-specific signal. `current_tab` is the honest
                // gate: Browser re-entry fires its own force-notify, so
                // the next `ClusterChanged` after entry still rebuilds.
                if affects_browser && state.current_tab == ViewId::Browser {
                    // `rebuild_arena_from_cluster` needs `&mut AppState`
                    // (to mutate the Browser arena + flow index) AND a
                    // read of the cluster snapshot. The snapshot lives
                    // inside `AppState.cluster`, so we clone it once to
                    // break the borrow. On a 10k-processor cluster this
                    // snapshot is a handful of MBs — cheap per update,
                    // but only paid while Browser is actually the
                    // subscriber of its endpoints.
                    let snap_snapshot = state.cluster.snapshot.clone();
                    crate::view::browser::state::rebuild_arena_from_cluster(
                        &mut state,
                        &snap_snapshot,
                    );
                }
                if affects_bulletins {
                    crate::view::bulletins::state::redraw_bulletins(&mut state);
                }
                if affects_browser_version_control {
                    let snap_vc = state.cluster.snapshot.clone();
                    crate::view::browser::state::redraw_version_control(&mut state, &snap_vc);
                }
                if affects_browser_parameter_context_bindings
                    && let Some(map) = state
                        .cluster
                        .snapshot
                        .parameter_context_bindings
                        .latest()
                        .cloned()
                {
                    state.browser.apply_parameter_context_bindings(&map);
                }

                let active = state.current_tab;
                let should_redraw = (affects_overview && active == ViewId::Overview)
                    || (affects_browser && active == ViewId::Browser)
                    || (affects_bulletins && active == ViewId::Bulletins)
                    || (affects_browser_version_control && active == ViewId::Browser)
                    || (affects_browser_parameter_context_bindings && active == ViewId::Browser);
                if should_redraw {
                    terminal
                        .draw(|f| ui::render(f, &mut state))
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
                PendingIntent::SpawnModalChunks(requests) => {
                    for req in requests {
                        crate::view::tracer::worker::spawn_modal_chunk(
                            client.clone(),
                            tx.clone(),
                            req.event_id,
                            req.side,
                            req.offset,
                            req.len,
                        );
                    }
                }
                PendingIntent::DecodeTabular {
                    event_id,
                    side,
                    bytes,
                } => {
                    let tx = tx.clone();
                    tokio::task::spawn_blocking(move || {
                        let render = crate::client::tracer::classify_content(bytes);
                        let _ = tx.try_send(AppEvent::Data(crate::event::ViewPayload::Tracer(
                            crate::event::TracerPayload::ContentDecoded {
                                event_id,
                                side,
                                render,
                            },
                        )));
                    });
                }
                PendingIntent::SpawnVersionControlModalFetch { pg_id } => {
                    let h = crate::view::browser::worker::spawn_version_control_modal_fetch(
                        client.clone(),
                        tx.clone(),
                        pg_id,
                    );
                    state.browser.version_modal_handle = Some(h);
                }
                PendingIntent::SpawnParameterContextModalFetch {
                    pg_id,
                    bound_context_id,
                } => {
                    let h = crate::view::browser::worker::spawn_parameter_context_modal_fetch(
                        client.clone(),
                        tx.clone(),
                        pg_id,
                        bound_context_id,
                    );
                    state.browser.parameter_modal_handle = Some(h);
                }
                PendingIntent::SpawnActionHistoryModalFetch {
                    source_id,
                    fetch_signal,
                } => {
                    let h = crate::view::browser::worker::spawn_action_history_modal_fetch(
                        client.clone(),
                        tx.clone(),
                        source_id,
                        fetch_signal,
                    );
                    state.browser.action_history_modal_handle = Some(h);
                }
                PendingIntent::SpawnSparklineFetchLoop { .. } => {
                    // Wired in Task 10.
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
                .draw(|f| ui::render(f, &mut state))
                .map_err(|source| NifiLensError::TerminalInit { source })?;
        }

        if state.pending_worker_restart {
            workers.invalidate();
            state.pending_worker_restart = false;
            state.cluster.spawn_fetchers(client.clone(), tx.clone());
        }
        // Drop the content viewer modal when leaving the Tracer tab so
        // stale in-flight chunks don't update a modal that is no longer
        // visible, and so re-entry always starts from a clean state.
        if workers.active_view() == Some(ViewId::Tracer) && state.current_tab != ViewId::Tracer {
            state.tracer.content_modal = None;
        }
        workers.ensure(
            state.current_tab,
            &client,
            &tx,
            &mut state.browser,
            &mut state.cluster,
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

/// Spawn the terminal-input polling task.
///
/// Runs on a dedicated OS thread, NOT on the tokio runtime: `crossterm::
/// event::poll` is a blocking syscall that never yields. Parking it on
/// a tokio worker would delay — and in quiet-terminal scenarios, hang —
/// runtime shutdown, because the task never reaches an await point for
/// the runtime to cooperatively cancel. The OS thread is detached; when
/// `main` returns, the process exits and the kernel reaps it.
fn spawn_input_task(tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || {
        loop {
            if let Ok(true) = crossterm::event::poll(std::time::Duration::from_millis(100))
                && let Ok(event) = crossterm::event::read()
                && tx.blocking_send(AppEvent::Input(event)).is_err()
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

/// Watch the `AppEvent` channel's remaining capacity and warn once per
/// second while fewer than `low_water` slots remain. Self-rate-limiting:
/// the warn falls silent as soon as capacity recovers. Spawned alongside
/// the input/tick tasks; aborts when the runtime shuts down.
fn spawn_channel_saturation_watchdog(tx: mpsc::Sender<AppEvent>, low_water: usize, total: usize) {
    tokio::task::spawn_local(async move {
        let mut sleep = tokio::time::interval(std::time::Duration::from_secs(1));
        sleep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            sleep.tick().await;
            // `tx.capacity()` returns the *remaining* slots, not the
            // total capacity. The Sender held by the watchdog stays
            // alive for the lifetime of the runtime; no need to check
            // for closed.
            let remaining = tx.capacity();
            if remaining < low_water {
                let in_flight = total.saturating_sub(remaining);
                tracing::warn!(
                    in_flight,
                    capacity = total,
                    "AppEvent channel near saturation — slow render or producer surge"
                );
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

#[cfg(test)]
mod channel_saturation_tests {
    use super::*;
    use tokio::sync::mpsc;
    use tracing_test::traced_test;

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    #[traced_test]
    async fn warns_when_capacity_below_low_water() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (tx, _rx) = mpsc::channel::<AppEvent>(20);
                // Fill 15 of 20 slots → only 5 remaining → below low_water of 16.
                for _ in 0..15 {
                    tx.send(AppEvent::Tick).await.expect("channel open");
                }
                spawn_channel_saturation_watchdog(tx.clone(), 16, 20);

                // Advance virtual time past the first 1s tick.
                tokio::time::advance(std::time::Duration::from_millis(1100)).await;
                tokio::task::yield_now().await;

                logs_assert(|lines: &[&str]| {
                    let any_warn = lines.iter().any(|l| l.contains("near saturation"));
                    if any_warn {
                        Ok(())
                    } else {
                        Err("expected near-saturation warn".into())
                    }
                });
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    #[traced_test]
    async fn no_warn_when_capacity_healthy() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (tx, _rx) = mpsc::channel::<AppEvent>(20);
                // Empty channel → 20 remaining → above low_water of 16.
                spawn_channel_saturation_watchdog(tx.clone(), 16, 20);

                tokio::time::advance(std::time::Duration::from_millis(1100)).await;
                tokio::task::yield_now().await;

                logs_assert(|lines: &[&str]| {
                    let any_warn = lines.iter().any(|l| l.contains("near saturation"));
                    if !any_warn {
                        Ok(())
                    } else {
                        Err("unexpected near-saturation warn".into())
                    }
                });
            })
            .await;
    }
}
