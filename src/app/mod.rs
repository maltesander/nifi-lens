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

    let mut state = AppState::new(context_name, detected_version);
    let mut workers = WorkerRegistry::new();
    workers.ensure(state.current_tab, &client, &tx);

    let dispatcher = Arc::new(IntentDispatcher {
        client: client.clone(),
        config: config.clone(),
    });

    terminal
        .draw(|f| ui::render(f, &state))
        .map_err(|source| NifiLensError::TerminalInit { source })?;

    while let Some(event) = rx.recv().await {
        let result = update(&mut state, event, &config);

        if let Some(pending) = result.intent {
            let dispatcher = dispatcher.clone();
            let tx = tx.clone();
            let intent = match pending {
                PendingIntent::SwitchContext(name) => Intent::SwitchContext(name),
                PendingIntent::Quit => Intent::Quit,
            };
            // Intent dispatch runs on the multi-thread runtime via
            // `tokio::spawn`, NOT on the main-thread `LocalSet` that hosts
            // the Overview worker. That means the future below must be
            // `Send`. The intent dispatcher only holds `Arc<RwLock<...>>`
            // and owned intent values, which are all `Send`. If a future
            // intent dispatch needs a `!Send` path (e.g., a direct call
            // into the dynamic client traits), switch to `spawn_local`
            // and accept that the work runs on the UI thread.
            tokio::spawn(async move {
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

        workers.ensure(state.current_tab, &client, &tx);
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
