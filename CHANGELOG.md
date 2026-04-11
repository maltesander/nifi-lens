# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Overview tab** — live health dashboard with cluster identity strip,
  global component counts (running / stopped / invalid / disabled),
  15-minute bulletin-rate sparkline (colored by worst severity per
  minute), top-10 unhealthy queue leaderboard sorted by back-pressure
  fill percentage, and top-5 noisy-component leaderboard. Polls every
  10 seconds while the tab is active; the polling task is cancelled on
  tab switch so API load is proportional to what the user is looking at.
- **`NifiClient` wrappers** — `controller_status`, `root_pg_status`,
  `bulletin_board`, and extended `about` covering the four endpoints the
  Overview tab needs. Each returns an owned snapshot struct so the
  reducer stays decoupled from `nifi-rust-client` types.
- **Per-tab `WorkerRegistry`** — app run loop now owns a registry that
  swaps the currently-active per-view worker on tab change. Phase 1
  only spawns for Overview; other tabs get no worker until their own
  phase lands.
- **CLI** — `clap` derive with global flags `--config`, `--context`, `--debug`,
  `--log-level`, `--no-color`, `--allow-writes` (reserved for v2, errors on
  use). Subcommands: `config init`, `config validate`, `version`.
  `--debug` and `--log-level` are mutually exclusive at parse time.
- **Kubeconfig-style config loader** with per-context env-var credentials,
  plaintext fallback (with warning), `0600` permission enforcement on Unix,
  `current_context` override via `--context`, and TLS options
  (`insecure_tls`, `ca_cert_path`).
- **`nifilens config init`** writes a commented template to
  `$XDG_CONFIG_HOME/nifilens/config.toml` (chmod 0600). `--force` overwrites
  an existing file.
- **`nifilens config validate`** parses the config without starting the TUI
  and reports the context count and active context to stderr.
- **`NifiClient` wrapper** around `nifi_rust_client::DynamicClient` with
  `Deref` / `DerefMut` for transparent method forwarding. `connect()`
  handles build → login → version detection; `about()` returns a Phase 0
  `AboutSnapshot`; `context_name()` and `detected_version()` accessors.
- **TLS handling** — system trust store by default; optional per-context
  `ca_cert_path` PEM added as a root certificate; `insecure_tls = true`
  skips certificate verification with a loud warning.
- **Compact rotating logging** via `tracing-subscriber` + `tracing-appender`
  to `$XDG_STATE_HOME/nifilens/nifilens.log`. Env filter priority:
  `--log-level` > `--debug` > `NIFILENS_LOG` > `RUST_LOG` > `info`, always
  scoped to `nifi_lens=<level>` so third-party crates stay quiet.
- **`StderrToggle`** via `tracing-subscriber::reload::Handle`, letting the
  TUI suppress stderr log output while raw mode is active and restore it
  on exit (including panics).
- **ratatui + crossterm render loop** with a single bounded `AppEvent`
  channel (256), a terminal-input task, a 1 s tick task, and state owned
  exclusively by the UI task.
- **`TerminalGuard`** RAII wrapper that enters raw mode + alternate-screen
  and restores them on drop. Installed alongside a panic hook that
  restores the terminal before `color_eyre` prints.
- **Four empty tabs** (Overview / Bulletins / Browser / Tracer) rendering
  named "coming in Phase N" placeholders, with `Tab` / `Shift+Tab` /
  `F1`–`F4` navigation.
- **`Ctrl+K` context switcher modal** that lists all configured contexts,
  highlights the active one, and dispatches `Intent::SwitchContext` on
  Enter. The dispatcher reconnects and swaps the shared `NifiClient`
  behind `Arc<RwLock>`.
- **`?` help modal** listing the global keys (Phase 0 single static
  content; per-view help arrives later).
- **Error banners** in the status bar with expandable detail modal (`e`
  to expand).
- **`Intent` enum** declaring read + write variants. Phase 0 wires `Quit`,
  `SwitchContext`, `RefreshView`; every other variant returns
  `NotImplementedInPhase0 { intent_name }`. Write intents unconditionally
  refuse with `WriteIntentRefused`.
- **Wiremock client wrapper tests** covering the happy path and 401/500
  error surfaces.
- **Docker-backed integration test harness** at `integration-tests/` with
  self-signed TLS, single-user auth, managed authorizer, and an
  `#[ignore]`-gated `tests/integration_connect.rs` that verifies
  `NifiClient::connect` and `about()` against a real NiFi 2.9.0 container.

### Changed

- **Crate is now lib + bin.** `src/lib.rs` holds every module;
  `src/main.rs` is a thin wrapper calling `nifi_lens::run()`. Integration
  tests can `use nifi_lens::...` without spawning the binary.
- **MSRV raised to 1.88** (from 1.85) so `time >= 0.3.47` can land and
  fix [RUSTSEC-2026-0009](https://rustsec.org/advisories/RUSTSEC-2026-0009).
- **`deny.toml`** now allows `BSL-1.0` (clipboard-win / error-code via
  arboard) and ignores `RUSTSEC-2024-0436` (unmaintained `paste`
  transitive via ratatui — no safe upgrade available).

[Unreleased]: https://github.com/maltesander/nifi-lens/commits/HEAD
