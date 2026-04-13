# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **UI Reorg Phase 1 — Chrome refactor.** New 1-row top bar with tabs +
  right-aligned compact identity strip (`[ctx] vX.Y.Z · nodes N/M`).
  Two-row footer (banner + refresh age, then context-sensitive shortcuts
  hint bar). The bordered `" nifi-lens "` title box is gone.
- **UI Reorg Phase 2 — Keybinding rename sweep.** `Ctrl+F` → `f`,
  `Ctrl+K` → `K`, `Alt+←` / `Alt+→` → `[` / `]`, `Shift+B` relabeled
  `B` (handler unchanged). `Ctrl+C` / `Ctrl+Q` quit and emacs
  text-input helpers (`Ctrl+U`, `Ctrl+N`, `Ctrl+P`) preserved. Rule:
  bare lowercase for view-local, bare capital for app-wide, no Ctrl
  chords except quit / text-input helpers.
- **UI Reorg Phase 3 — Overview merge.** Health tab's data and
  presentation merged into Overview as the new Layout 3 dashboard:
  Format-C processor info line, nodes hero zone (per-node heap/GC/load,
  cluster-aggregate repository fill bars), bulletins/noisy 50/50
  split, and unhealthy queues full-width. Overview worker rewritten for
  dual cadence (10s PG status + 30s system diagnostics with nodewise →
  aggregate fallback). Top-bar identity strip now shows real
  `nodes N/M` from the SystemDiag payload.
- **F-key remap.** F1=Overview, F2=Bulletins, F3=Browser, F4=Events,
  F5=Tracer (was: F3=Health, F4=Browser, F5=Tracer).
- Bulletins and Tracer timestamp formatting now route through a shared
  `timestamp` module backed by the `time` crate. This deduplicates two
  fragile byte-sliced parsers and enables the new `[ui]` config.
  (Implementation uses `time` rather than `chrono`; `time` is already a
  dependency and covers every requirement.)
- Inline `Color::*` / `Modifier::*` constructors across the view layer
  have been replaced with calls into `src/theme.rs` helpers. Visual
  output is unchanged except for a handful of principled improvements
  (e.g. GC-delta errors now render bold red).
- `theme::severity_by_pct` helper centralises percentage-threshold style
  mapping.
- **Phase 6 structural cleanup.** Split monolithic `app/state.rs`
  (2,535 lines) into per-view key handler modules behind a
  `ViewKeyHandler` trait. Extracted `ListNavigation` trait for shared
  list navigation math. Genericized worker polling loop for Overview and
  Bulletins. Consolidated inline styles into semantic theme helpers.
- Bumped `nifi-rust-client` from 0.5.0 to 0.7.0 — adds typed provenance
  content bodies, NiFi 2.9.0 support, and `Option<DetectedVersion>`.
- Dropped the `since` field from `CrossLink::TraceComponent`.
- `IntentOutcome::NotImplementedInPhase { phase: 3 }` is no longer
  emitted for `CrossLink::OpenInBrowser`; the dispatcher now returns
  `IntentOutcome::OpenInBrowserTarget` and the reducer handles the
  tab switch + ancestor expansion.
- **`IntentOutcome::NotImplementedInPhase0` → `NotImplementedInPhase {
  phase }`.** Internal refactor; the banner now reports the phase a
  stubbed intent is waiting on.
- **`CrossLink::ComponentId(String)` → `OpenInBrowser { .. }` /
  `TraceComponent { .. }`.** Stronger typing around cross-tab jumps.
- **Crate is now lib + bin.** `src/lib.rs` holds every module;
  `src/main.rs` is a thin wrapper calling `nifi_lens::run()`. Integration
  tests can `use nifi_lens::...` without spawning the binary.
- **MSRV raised to 1.88** (from 1.85) so `time >= 0.3.47` can land and
  fix [RUSTSEC-2026-0009](https://rustsec.org/advisories/RUSTSEC-2026-0009).
- **`deny.toml`** now allows `BSL-1.0` (clipboard-win / error-code via
  arboard) and ignores `RUSTSEC-2024-0436` (unmaintained `paste`
  transitive via ratatui — no safe upgrade available).

### Removed

- **Health tab.** Merged into Overview. The `view/health/` directory,
  `app/state/health.rs`, the `HealthPayload` / `ViewPayload::Health`
  variants, the `ViewId::Health` variant, and the `health: HealthState`
  field on `AppState` are all gone. F3 → Health binding replaced with
  F3 → Browser.
- **Per-node repository breakdown.** The old Health "Repositories"
  detail pane showed per-node fill bars on row selection. Layout 3
  shows only the cluster aggregate row. Per-node drill-in is a future
  detail-pane feature.
- **Processor thread leaderboard.** The old Health "Processors"
  category showed a top-N processor-by-active-threads leaderboard.
  Layout 3 has no equivalent panel — the processor info line shows
  aggregate counts only.
- **Queue time-to-full predictions and stalled badge.** The old Health
  "Queues" category showed `~30s` / `~2m` / `stable` / `∞ (stalled)`
  hints derived from server-side backpressure timestamps. The Layout
  3 unhealthy queues table shows fill / queue / src→dst / ffile count
  only. The data is still in the API; restoring the column is a
  future polish item.
- **Queue Enter-cross-link to Browser.** The old Health queue rows
  supported `Enter` to jump to the connection in Browser. The Layout
  3 unhealthy queues table is non-interactive.
- **Bordered tab bar with `" nifi-lens "` title.** Replaced by the
  Phase 1 1-row top bar.
- **Old Ctrl/Alt chord bindings.** Replaced by Phase 2 bare-letter
  equivalents.

### Added

- Tab history with `Alt+Left`/`Alt+Right` for cross-link back/forward
  navigation, including selection restore (Browser remembers which
  component was selected).
- Interactive breadcrumb bar in Browser detail pane showing the path from
  root to the selected node. Press `b` to enter, `h`/`l` to navigate
  segments, `Enter` to jump to an ancestor, `Esc` to cancel.
- Context-sensitive sticky footer hint line showing relevant keybindings
  for the current view and mode. Always visible below the status bar.
- **BREAKING:** config auth moved from top-level `username`/`password_env`
  to `[contexts.auth]` sub-table with `type` discriminator (`password`,
  `token`, `mtls`). See README for migration examples.
- Token auth (`type = "token"`) for pre-obtained JWT tokens via `token_env`
  or `token`.
- mTLS auth (`type = "mtls"`) with `client_identity_path`.
- `proxied_entities_chain` context field for NiFi proxy deployments.
- **Health tab (Phase 5):** cluster-wide operational dashboard with four
  categories — queue backpressure leaderboard with server-predicted
  time-to-full, repository fill bars, per-node heap/GC/load strips,
  and processor thread leaderboard. Two-pane layout with severity
  indicators. `Enter` on a queue or processor row jumps to Browser.
- **Tracer tab (Phase 4):** paste a flowfile UUID → lineage timeline →
  per-event attribute diff and input/output content preview. Bulletins
  and Browser `t` cross-links land on a latest-provenance-events mini
  list.
- Browser tab: PG tree + per-node detail panes (Processor, Connection,
  Process Group, Controller Service) with 15-second recursive tree
  refresh and on-demand detail fetches.
- Global `Ctrl+F` fuzzy find (nucleo-backed), lazy-seeded on first
  Browser entry.
- `Enter` on a Bulletins row now lands on the matching component in
  Browser (replaces the Phase 3 stub banner).
- Properties modal (`e`) for Processor and Controller Service nodes.
- `c` keybind to copy the selected node's id to the clipboard
  (`arboard`).
- `r` keybind to force-refresh the Browser tree.
- Widget: `src/widget/fuzzy_find.rs` backed by `nucleo 0.5`.
- **Bulletins tab.** Cluster-wide live bulletin tail with severity
  toggles (`e`/`w`/`i`), component-type cycling (`T`), free-text filter
  (`/`), auto-scroll pause (`p`) with a `+N new` badge, and
  `Enter`/`t` cross-link stubs that emit Phase 3 / Phase 4 intents.
- **`[bulletins] ring_size` config knob.** Optional; default 5000;
  valid range 100..=100_000. Controls the size of the in-memory ring
  the Bulletins tab keeps.
- **Per-node repository breakdown.** The Health tab's Repositories
  category now supports `j`/`k` navigation. Selecting a repository row
  shows per-node fill bars in a detail pane, replacing the former
  aggregate-only display.
- **Per-view help modal sections.** The `?` help modal now renders a
  per-tab keybind section below the global keys.
- **Multi-version integration test fixture.** `integration-tests/run.sh`
  now boots NiFi 2.6.0 and 2.8.0 simultaneously and seeds both via a new
  `nifilens-fixture-seeder` workspace binary, producing a realistic flow
  with running pipelines, back-pressured queues, multi-severity bulletins,
  nested process groups, and varied controller services. The harness now
  drives the fixture so local development and CI see identical state.
  Integration tests loop over every supported NiFi version from a single
  `integration-tests/versions.toml` source of truth, generated into a
  compile-time `FIXTURE_VERSIONS` const by a new root `build.rs`. CI
  gains a drift check and a dedicated integration job. See `AGENTS.md`
  for the live-dev workflow and the ceiling-version bump procedure.
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
- Browser tree now shows per-processor run-state icons (● running,
  ◌ stopped, ⚠ invalid, ⌀ disabled, ◐ validating) so component state is
  visible without opening the detail pane.
- Health Nodes table renders Load as a 4-character spark-bar gauge
  coloured by severity (warning ≥ 1×cores, error ≥ 1.5×cores).
- Bulletins tab supports `Shift+B` to collapse consecutive same-source
  bulletins into a single row with a `[×N]` count badge.
- New `[ui]` config section with `timestamp_format`
  (`short` / `iso` / `human`) and `timestamp_tz` (`utc` / `local`). See
  `README.md` for the reference.

### Fixed

- Health tab: stalled queues (items queued, zero throughput) now show
  `∞ (stalled)` in red instead of the misleading `stable` label.
- Health tab: per-node diagnostics fall back to cluster aggregate with a
  warning banner when the nodewise fetch fails.
- Health tab: pressing `Enter` on repository or node rows now shows an
  info hint instead of silently doing nothing.
- Help modal: removed stale "Phase 3/4 stub" annotations from
  keybinding descriptions.

### Internal

- New `src/timestamp.rs` module owns all wire-format timestamp
  parsing and display formatting.
- New `src/widget/gauge.rs` module owns `fill_bar` (moved from
  `health::render`) and a new `spark_bar` helper.
- `NodeDiagnostics` and `NodeHealthRow` gain an `available_processors`
  field to drive the Health Load spark-bar max.

### Dependencies

- Bump `nifi-rust-client` from 0.7.0 to 0.8.0.

[Unreleased]: https://github.com/maltesander/nifi-lens/commits/HEAD
