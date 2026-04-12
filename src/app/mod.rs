//! App run loop, terminal guard, and panic hook.

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

use crate::app::state::{AppState, PendingIntent, update};
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
    let mut workers = WorkerRegistry::new();
    let bulletins_last_id = state.bulletins.last_id;
    workers.ensure(
        state.current_tab,
        &client,
        &tx,
        bulletins_last_id,
        &mut state.browser,
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
        let result = update(&mut state, event, &config);

        // Dispatch tracer followups (e.g. delete a consumed lineage query).
        if let Some(followup) = result.tracer_followup {
            match followup {
                crate::view::tracer::state::Followup::DeleteLineageQuery { query_id } => {
                    let dispatcher = dispatcher.clone();
                    let tx = tx.clone();
                    tokio::task::spawn_local(async move {
                        let outcome = dispatcher
                            .dispatch(Intent::DeleteLineageQuery { query_id })
                            .await;
                        let _ = tx.send(AppEvent::IntentOutcome(outcome)).await;
                    });
                }
            }
        }

        if let Some(pending) = result.intent {
            let dispatcher = dispatcher.clone();
            let tx = tx.clone();
            let intent = match pending {
                PendingIntent::SwitchContext(name) => Intent::SwitchContext(name),
                PendingIntent::JumpTo(link) => Intent::JumpTo(link),
                PendingIntent::Quit => Intent::Quit,
            };
            // Intent dispatch runs on the main-thread `LocalSet` via
            // `spawn_local` so that Phase 4 tracer workers (which also
            // use `spawn_local` internally for `!Send` client futures)
            // are spawned within the correct context.
            tokio::task::spawn_local(async move {
                let outcome = dispatcher.dispatch(intent).await;
                let _ = tx.send(AppEvent::IntentOutcome(outcome)).await;
            });
        }

        if state.should_quit {
            break;
        }

        if result.redraw {
            terminal
                .draw(|f| ui::render(f, &state))
                .map_err(|source| NifiLensError::TerminalInit { source })?;
        }

        let bulletins_last_id = state.bulletins.last_id;
        workers.ensure(
            state.current_tab,
            &client,
            &tx,
            bulletins_last_id,
            &mut state.browser,
        );
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
