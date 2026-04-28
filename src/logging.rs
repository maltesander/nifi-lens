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
///
/// `Clone`-able (via `Arc`) so it can be shared between the `TerminalGuard`
/// drop impl and the panic hook — both need to restore stderr on exit.
#[derive(Clone)]
pub struct StderrToggle {
    inner: std::sync::Arc<StderrToggleInner>,
}

struct StderrToggleInner {
    handle: reload::Handle<Option<DynLayer>, Registry>,
}

impl StderrToggle {
    /// Suppress stderr log output (called before entering raw mode).
    pub fn suppress(&self) {
        let _ = self.inner.handle.modify(|layer| *layer = None);
    }

    /// Restore stderr log output (called after leaving raw mode, including panics).
    pub fn restore(&self) {
        // We cannot literally restore the old layer because Box<dyn Layer>
        // isn't Clone; instead we rebuild a fresh stderr layer.
        let stderr_layer = make_stderr_layer(use_color());
        let _ = self
            .inner
            .handle
            .modify(|layer| *layer = Some(stderr_layer));
    }
}

/// Global color setting, captured at `init()` time from `args.no_color`.
/// `StderrToggle::restore()` reads this value when rebuilding the stderr
/// layer; there is no mid-session color override mechanism.
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

/// Resolve the no-color setting from the CLI flag and the NO_COLOR env var.
/// Per the no-color.org convention, any non-empty NO_COLOR value disables
/// colors; an empty value is treated as unset. The CLI flag takes precedence
/// (if set, colors are always disabled regardless of env).
fn resolve_no_color(cli_no_color: bool, env_no_color: Option<&std::ffi::OsStr>) -> bool {
    cli_no_color || matches!(env_no_color, Some(v) if !v.is_empty())
}

/// Initialize logging. Returns a (worker guard, stderr toggle) tuple; the
/// guard must stay alive for the whole process lifetime or logs will be
/// lost on shutdown.
pub fn init(args: &Args) -> Result<(WorkerGuard, StderrToggle), NifiLensError> {
    let no_color = resolve_no_color(args.no_color, std::env::var_os("NO_COLOR").as_deref());
    USE_COLOR.set(!no_color).ok();

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
    //    A later revision can add a startup pruning pass to keep only the 5
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
    let stderr_boxed: DynLayer = make_stderr_layer(!no_color);
    let (stderr_reload, stderr_handle) = reload::Layer::new(Some(stderr_boxed));
    let stderr_toggle = StderrToggle {
        inner: std::sync::Arc::new(StderrToggleInner {
            handle: stderr_handle,
        }),
    };

    // Subscriber chain:
    //   Registry → .with(stderr_reload) → .with(file_layer) → .with(env_filter)
    //
    // Ordering is load-bearing:
    //
    // - `reload::Layer<L, S>` only implements `Layer<S>` for the exact `S` it
    //   was parameterized with. Placing it innermost pins its `S` to `Registry`
    //   so the type-system is satisfied.
    //
    // - `EnvFilter` placed outermost as a naked `.with(...)` layer acts as a
    //   **global** pre-filter: events that do not match `nifi_lens=<level>`
    //   are short-circuited before reaching either the file layer or the
    //   stderr reload layer. Do NOT additionally use `.with_filter()` on
    //   either inner layer for the same filter — it would double-filter.
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
    resolve_log_dir_from(platform_default_log_dir())
}

fn resolve_log_dir_from(default: Option<PathBuf>) -> Result<PathBuf, NifiLensError> {
    default.ok_or_else(|| NifiLensError::Io {
        source: std::io::Error::other("could not determine log directory for this platform"),
    })
}

// Linux: $XDG_STATE_HOME/nifilens (or ~/.local/state/nifilens).
// macOS: ~/Library/Caches/nifilens.
// Windows: %LOCALAPPDATA%\nifilens\cache.
fn platform_default_log_dir() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "nifilens")?;
    Some(
        dirs.state_dir()
            .unwrap_or_else(|| dirs.cache_dir())
            .to_path_buf(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_log_dir_returns_provided_default() {
        let result = resolve_log_dir_from(Some(PathBuf::from("/var/state/nifilens"))).unwrap();
        assert_eq!(result, PathBuf::from("/var/state/nifilens"));
    }

    #[test]
    fn resolve_log_dir_errors_when_default_missing() {
        let err = resolve_log_dir_from(None).unwrap_err();
        assert!(matches!(err, NifiLensError::Io { .. }));
    }

    #[test]
    fn no_color_resolution() {
        use std::ffi::OsStr;
        // CLI flag wins.
        assert!(resolve_no_color(true, None));
        assert!(resolve_no_color(true, Some(OsStr::new("1"))));
        // Env-only.
        assert!(resolve_no_color(false, Some(OsStr::new("1"))));
        assert!(resolve_no_color(false, Some(OsStr::new("anything"))));
        // Empty env value — treated as unset per no-color.org.
        assert!(!resolve_no_color(false, Some(OsStr::new(""))));
        // Both off.
        assert!(!resolve_no_color(false, None));
    }
}
