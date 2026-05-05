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

    // 1. Resolve log directory and create it 0o700 in one syscall on unix
    //    so there is no chmod race between `mkdir` and `set_permissions`.
    let log_dir = resolve_log_dir()?;
    create_log_dir(&log_dir)?;

    // 2. Prune old daily files at startup. tracing-appender's `rolling::daily`
    //    has no built-in retention cap, so we sweep before adding a new file.
    prune_log_files(&log_dir, LOG_FILE_PREFIX, LOG_FILE_RETENTION);

    // 3. Rotating appender. Daily rotation keeps the implementation simple.
    let file_appender = rolling::daily(&log_dir, LOG_FILE_PREFIX);
    let (non_blocking, worker_guard) = tracing_appender::non_blocking(file_appender);

    // 4. Build the filter. --debug and --log-level are mutually exclusive
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

    // 5. File layer (always ANSI-off).
    let file_layer = fmt::layer()
        .compact()
        .with_ansi(false)
        .with_writer(non_blocking);

    // 6. Stderr layer wrapped in a reload handle so we can toggle it off.
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

/// Daily file basename — `tracing-appender` appends `.YYYY-MM-DD`.
const LOG_FILE_PREFIX: &str = "nifilens.log";

/// Maximum number of rotated daily log files to keep.
const LOG_FILE_RETENTION: usize = 14;

/// Create the log directory. On unix the dir is created `0o700` in a
/// single syscall via `DirBuilder::mode`, so there is no chmod race
/// between `mkdir` and `set_permissions`.
fn create_log_dir(log_dir: &std::path::Path) -> Result<(), NifiLensError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder
            .create(log_dir)
            .map_err(|source| NifiLensError::Io { source })?;
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(log_dir).map_err(|source| NifiLensError::Io { source })?;
    }
    Ok(())
}

/// Sweep `dir` for files starting with `prefix` and keep only the
/// `keep` most recent (by mtime). Best-effort — failures are silently
/// ignored: a stale file is preferable to refusing to start the TUI.
fn prune_log_files(dir: &std::path::Path, prefix: &str, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(PathBuf, std::time::SystemTime)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if !name.starts_with(prefix) {
                return None;
            }
            let mtime = entry.metadata().ok()?.modified().ok()?;
            Some((path, mtime))
        })
        .collect();
    if files.len() <= keep {
        return;
    }
    // Newest first; drop everything past `keep`.
    files.sort_by(|a, b| b.1.cmp(&a.1));
    for (path, _) in files.into_iter().skip(keep) {
        let _ = std::fs::remove_file(path);
    }
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

    #[cfg(unix)]
    #[test]
    fn create_log_dir_sets_0700_atomically() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("nested/log");
        create_log_dir(&target).unwrap();
        let mode = std::fs::metadata(&target).unwrap().permissions().mode();
        // The mode includes the file-type bits; mask down to the perm bits.
        assert_eq!(mode & 0o777, 0o700);
    }

    #[test]
    fn prune_log_files_keeps_newest_n() {
        use std::time::{Duration, SystemTime};
        let tmp = tempfile::tempdir().unwrap();
        // Five files with strictly increasing mtimes.
        let mut paths = Vec::new();
        for i in 0..5 {
            let p = tmp
                .path()
                .join(format!("nifilens.log.2026-05-{:02}", i + 1));
            std::fs::write(&p, b"x").unwrap();
            let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + i as u64 * 60);
            filetime::set_file_mtime(&p, filetime::FileTime::from_system_time(mtime)).unwrap();
            paths.push(p);
        }
        // Unrelated file must not be touched.
        let other = tmp.path().join("unrelated.txt");
        std::fs::write(&other, b"x").unwrap();

        prune_log_files(tmp.path(), "nifilens.log", 2);

        // Newest two remain; older three gone.
        assert!(!paths[0].exists());
        assert!(!paths[1].exists());
        assert!(!paths[2].exists());
        assert!(paths[3].exists());
        assert!(paths[4].exists());
        assert!(other.exists());
    }

    #[test]
    fn prune_log_files_noop_when_under_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("nifilens.log.2026-05-01");
        std::fs::write(&p, b"x").unwrap();
        prune_log_files(tmp.path(), "nifilens.log", 14);
        assert!(p.exists());
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
