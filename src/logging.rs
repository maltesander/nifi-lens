//! Logging initialization.
//!
//! Two-stage: before the TUI enters raw mode, logs go to both a rotating
//! file and stderr (with optional ANSI). Once raw mode starts, the stderr
//! layer is removed via a reload handle so log output cannot corrupt the
//! terminal. On exit (including panics), the stderr layer is restored.

use std::path::PathBuf;
use std::sync::OnceLock;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling;
use tracing_subscriber::Registry;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::reload;
use tracing_subscriber::util::SubscriberInitExt;

use crate::NifiLensError;
use crate::cli::Args;

type DynLayer = Box<dyn tracing_subscriber::Layer<Registry> + Send + Sync>;

/// Handle for toggling the stderr layer on and off (used by the terminal guard).
pub struct StderrToggle(reload::Handle<Option<DynLayer>, Registry>);

impl StderrToggle {
    /// Suppress stderr log output (called before entering raw mode).
    pub fn suppress(&self) {
        let _ = self.0.modify(|layer| *layer = None);
    }

    /// Restore stderr log output (called after leaving raw mode).
    pub fn restore(&self) {
        // We cannot literally restore the old layer because Box<dyn Layer>
        // isn't Clone; instead we rebuild a fresh stderr layer.
        let stderr_layer = make_stderr_layer(use_color());
        let _ = self.0.modify(|layer| *layer = Some(stderr_layer));
    }
}

static USE_COLOR: OnceLock<bool> = OnceLock::new();

fn use_color() -> bool {
    *USE_COLOR.get().unwrap_or(&true)
}

fn make_stderr_layer(color: bool) -> DynLayer {
    Box::new(
        fmt::layer()
            .compact()
            .with_ansi(color)
            .with_writer(std::io::stderr),
    )
}

/// Initialize logging. Returns a (worker guard, stderr toggle) tuple; the
/// guard must stay alive for the whole process lifetime or logs will be
/// lost on shutdown.
pub fn init(args: &Args) -> Result<(WorkerGuard, StderrToggle), NifiLensError> {
    USE_COLOR.set(!args.no_color).ok();

    // 1. Resolve log directory.
    let log_dir = resolve_log_dir()?;
    std::fs::create_dir_all(&log_dir).map_err(|source| NifiLensError::Io { source })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&log_dir)
            .map_err(|source| NifiLensError::Io { source })?
            .permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(&log_dir, perms).map_err(|source| NifiLensError::Io { source })?;
    }

    // 2. Rotating appender. Daily rotation keeps the implementation simple.
    //    A later phase can add a startup pruning pass to keep only the 5
    //    most recent files if size-based rotation is still unavailable in
    //    the installed `tracing-appender` version.
    let file_appender = rolling::daily(&log_dir, "nifilens.log");
    let (non_blocking, worker_guard) = tracing_appender::non_blocking(file_appender);

    // 3. Build the filter. --debug and --log-level are mutually exclusive
    //    at the CLI layer, so we don't worry about precedence between them.
    let level = if let Some(level) = args.log_level {
        level.as_tracing_filter().to_string()
    } else if args.debug {
        "debug".to_string()
    } else if let Ok(env) = std::env::var("NIFILENS_LOG") {
        env
    } else if let Ok(env) = std::env::var("RUST_LOG") {
        env
    } else {
        "info".to_string()
    };
    let filter_str = format!("nifi_lens={level}");
    let env_filter = EnvFilter::try_new(&filter_str).map_err(|err| NifiLensError::LoggingInit {
        source: Box::new(err),
    })?;

    // 4. File layer (always ANSI-off).
    let file_layer = fmt::layer()
        .compact()
        .with_ansi(false)
        .with_writer(non_blocking);

    // 5. Stderr layer wrapped in a reload handle so we can toggle it off.
    let stderr_boxed: DynLayer = make_stderr_layer(!args.no_color);
    let (stderr_reload, stderr_handle) = reload::Layer::new(Some(stderr_boxed));
    let stderr_toggle = StderrToggle(stderr_handle);

    // The reload layer must be placed first (innermost) so that its subscriber
    // type parameter S resolves to Registry, not a Layered<...> wrapper.
    // EnvFilter and file layer are added on top of it.
    Registry::default()
        .with(stderr_reload)
        .with(file_layer)
        .with(env_filter)
        .try_init()
        .map_err(|err| NifiLensError::LoggingInit {
            source: Box::new(err),
        })?;

    Ok((worker_guard, stderr_toggle))
}

fn resolve_log_dir() -> Result<PathBuf, NifiLensError> {
    if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        return Ok(PathBuf::from(xdg).join("nifilens"));
    }
    let home = std::env::var("HOME").map_err(|_| NifiLensError::Io {
        source: std::io::Error::new(std::io::ErrorKind::NotFound, "HOME not set"),
    })?;
    Ok(PathBuf::from(home).join(".local/state/nifilens"))
}
