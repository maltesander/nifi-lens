# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **Architecture**: all periodic NiFi polling is now centralized in a
  single `ClusterStore` (seven per-endpoint fetchers), replacing the
  per-view worker polls that Overview, Browser, and Bulletins used to
  run. Overview and Browser now share a single `root_pg_status`,
  `controller_services`, and per-PG `connections_by_pg` fetch — load
  reduction is proportional to the number of tabs that previously
  duplicated these polls.
- Polling cadences adapt to measured latency (up to `max_interval`,
  default `60s`) and are jittered by ±`jitter_percent/100` (default
  20%) to avoid synchronized bursts across endpoints.
- Three expensive endpoints (`root_pg_status`, `controller_services`,
  `connections_by_pg`) park when no view subscribes to them — i.e.
  while neither Overview nor Browser is the active tab.
- **Overview**: top panel renamed from `Processors` to `Components` and
  expanded into a three-row table (process groups, processors,
  controller services). PG row shows version-sync drift counts (or
  `all in sync` when healthy) and input/output port counts. CS row
  shows per-state counts; degrades to a `cs list unavailable` chip
  when the new `/flow/process-groups/root/controller-services` fetch
  fails. Drops the always-near-zero `THREADS` field.

- **Overview**: nodes panel now lists cluster nodes sorted
  alphabetically by `host:port` (case-insensitive) instead of in the
  order returned by `/system-diagnostics`.

- **Browser**: the `p` properties popup is now a selectable two-column
  table. `↑`/`↓` move the selection, `c` copies the focused row's
  value to the clipboard, and `Enter` on a property whose value is a
  UUID pointing to a known arena node closes the modal and jumps to
  that node in the tree (same cross-link path used by the detail
  pane). A fixed detail strip below the table shows the selected
  row's full value so long values stay readable.

### Fixed

- **Bulletins**: `Shift+R` (clear filters) now also clears the
  session-scoped mute list. Previously muted sources could only be
  unmuted by restarting the binary, because muted rows are hidden and
  could not be reselected to toggle `Shift+M` off.

### Removed

- Per-view Overview, Browser, and Bulletins worker tasks — replaced by
  `ClusterStore` fetchers and `redraw_*` reducers driven off
  `AppEvent::ClusterChanged`.
- Sysdiag nodewise → aggregate fallback banner — the transition is now
  logged to `nifilens.log` rather than surfaced in the TUI. Monitor the
  log if you run mixed-version fleets.

### Security

- Bump `rand` to 0.10, resolving [RUSTSEC-2026-0097][rustsec-0097]
  (soundness advisory for `ThreadRng` under custom loggers).

[rustsec-0097]: https://rustsec.org/advisories/RUSTSEC-2026-0097

### Breaking (config)

- Per-view polling sections `[polling.overview]`, `[polling.browser]`,
  and `[polling.bulletins]` have been replaced by a single
  `[polling.cluster]` section. See `README.md` for the new shape. There
  is no back-compat shim: `config.toml` files that still use the old
  sections will fail to parse.

## [0.4.0] — 2026-04-17

### Added

- **Configurable poll intervals** for Overview, Browser, and Bulletins
  tabs via a new `[polling]` section in `config.toml`. Values use the
  humantime format (`"10s"`, `"750ms"`). Defaults match the previously-
  hardcoded cadences (10s/30s/15s/5s). Out-of-band values log a warning
  but are accepted as-is. Events in-flight query polling and Tracer
  content polling stay on their internal cadences.
- **Browser**: controller services appear as first-class tree nodes under
  their owning PG, bucketed under a collapsible `⚙ Controller services`
  folder.
- **Browser**: queues moved under a collapsible `→ Queues` folder per PG.
- **Browser**: controller-service detail pane shows comments,
  `restricted / deprecated / persistsState` flags, a `Referencing
  components` section (Enter to jump to the referencing component),
  and a `Recent bulletins` section.
- **Browser**: input / output ports have a working detail pane
  (identity + recent bulletins).
- **Browser**: cross-navigation across detail panes. Queue endpoints,
  processor CS-reference properties, and a new processor `Connections`
  section all jump to the referenced component on Enter (reuses the
  existing `OpenInBrowser` cross-link). Rows whose value resolves to a
  known arena node render a trailing `→` marker. Controller Service
  and Port Identity panels now show the parent process group's name
  instead of the raw UUID.
- **Tracer content-pane preview cap**: bodies over 1 MiB are fetched with
  a `Range: bytes=0-1048575` header and flagged as truncated in the panel
  title. Save re-fetches the full body on demand.
- **Integration fixture**: new `bulky-pipeline` producing ~1.5 MiB
  flowfiles, and an `UpdateAttribute-cleanup` processor in
  `healthy-pipeline/enrich` that exercises the attribute-removed rendering
  path.

### Changed

- Fixture now wires a `ConvertRecord` processor at the start of
  `healthy-pipeline/enrich` referencing `fixture-json-reader` and
  `fixture-json-writer` (both ENABLED at root), so the browser
  CS-referencing integration test has at least one referenced CS on
  NiFi 2.6.0 (previously only `stress-pipeline` on 2.9.0 created
  references, causing the test to fail on the floor version).
- Fixture marker bumped to `nifilens-fixture-v2`; existing clusters will
  be re-seeded automatically on next run.
- `ContentSnapshot` no longer carries raw bytes; `ContentPane::Shown`
  follows suit. The Save action now re-fetches via
  `provenance_content_raw`.
- `ContentRender::Text` now carries a single authoritative `String` plus a
  `pretty_printed: bool` flag, replacing the separate `pretty: String`
  field.

## [0.3.0] — 2026-04-15

### Added

- **Browser: horizontal scroll in detail panes.** When a detail
  sub-panel is focused (`Tab`/`Shift+Tab`), `←`/`→` now scroll the
  content column one character at a time. Applies to Properties (VALUE
  column), Validation errors, Recent bulletins (message column),
  Controller services (type column), and Child groups (name column).
  Each section remembers its own horizontal offset independently.
- **Browser: validation errors in bordered panel.** Processors and
  controller services that have active validation errors now display them
  in a focusable bordered panel instead of inline text, making the list
  navigable and visually distinct.

### Fixed

- **Tracer: save confirmation shown in status bar.** After saving content
  to a file (`s`), the "saved to \<path\>" message now appears in the
  global footer banner (Info severity) instead of being silently
  discarded. Save failures are likewise surfaced as Error banners.
- **Status bar: long messages truncated with `…`.** Banner text wider than
  the available column count is trimmed and suffixed with an ellipsis
  rather than being hard-clipped by the terminal.
- **Tracer: `Updated` attribute class shown in yellow.** Changed
  attributes in the lineage detail pane now render with the warning
  (yellow) colour to distinguish them from added (green) and deleted
  (red) entries.
- **Events: `timestamp_tz` config honoured in detail pane.** Event detail
  rows now respect the `[timestamps] timezone` setting instead of always
  showing UTC.

## [0.2.0] — 2026-04-15

### Changed

- **Keybinding consolidation — all views.** FuzzyFind moves from `f` to
  `Shift+F`. Bulletins: CycleGroupBy `g` → `Shift+G`, TogglePause `p` →
  `Shift+P`, MuteSource `m` → `Shift+M`, ClearFilters `Shift+C` →
  `Shift+R`. Events: all filter-field edit keys move to Shift variants
  (`Shift+D` Time / `Shift+T` Types / `Shift+S` Source / `Shift+U` UUID /
  `Shift+A` Attributes); new-query `n` → `Shift+N`; reset `r` → `Shift+R`;
  bare `r` is now Refresh. No view binds bare `j`, `k`, `[`, or `]`;
  regression tests enforce this.
- **Cross-tab jump redesigned.** The two-key `g <letter>` leader combo is
  replaced by a single `g` that dispatches `AppAction::Jump`. When exactly
  one destination is reachable the jump fires silently; when multiple are
  available the new JumpMenu modal opens (see Added).
- **Tab / Shift+Tab cycle panes within each view.** All tabs (Overview,
  Browser, Events, Tracer) now use Tab/Shift+Tab for intra-view pane focus
  cycling. Tab-bar switching stays on F1–F5 exclusively.

### Added

- **`v` = paste, `x` = cut** from the system clipboard in every text-input
  field (Bulletins `/` search, Events filter fields, Tracer UUID entry).
- **JumpMenu modal** — `g` opens a scrollable, keyboard-navigable list of
  context-sensitive cross-tab destinations. `↑`/`↓` move the selection,
  `Enter` confirms, `Esc` cancels. Fires immediately when only one
  destination is reachable.
- **Overview interactive panels** — Nodes, Queues, and Noisy panels are
  now focusable `Table` widgets with row selection, scroll-to-cursor, and
  thick-border focus indicators. Tab/Shift+Tab cycles focus between panels.
- **Overview node detail popup** — `Enter` on a selected node row opens a
  two-pane modal with heap/load/threads/uptime summary on the left and GC
  collector table + per-type repository utilization with fill bars on the
  right.
- **Tracer timeline enriched** — lineage rows now show component type and
  per-event detail hints alongside the existing timestamp and event-type
  columns.
- **Bulletins `c`** copies the selected row's raw message to the clipboard.

### Fixed

- Overview: Queues panel `g` → Browser cross-link now navigates to the
  connection's parent process group instead of the connection itself.
- Overview: aligned repository fill bars; replaced aggregate load
  spark-bar with per-CPU strip.
- Overview: fixed bulletins-per-minute rolling window accumulating
  duplicate counts.
- Events: widened type column; replaced the empty `rel` column with an
  event-details column.

### Security

- Updated `rustls-webpki` to 0.103.12 (fixes
  [RUSTSEC-2026-0098](https://rustsec.org/advisories/RUSTSEC-2026-0098)
  and
  [RUSTSEC-2026-0099](https://rustsec.org/advisories/RUSTSEC-2026-0099)).

## [0.1.0] — 2026-04-14

Initial public release. Condensed summary of the development phases
that landed in this tag.

### Added

- **Five tabs** — Overview (cluster health dashboard with sparkline,
  queue / repository fill, per-node heap/GC/load, noisiest components),
  Bulletins (live cluster-wide tail with severity / source dedup and
  per-source mute), Browser (PG tree with per-node detail panes for
  Processor / Connection / Process Group / Controller Service / Ports,
  cross-navigation `→` jumps, properties modal), Events (cluster-wide
  provenance search with filter bar, result detail pane), Tracer (paste
  a flowfile UUID → lineage timeline → tabbed Attributes / Input /
  Output detail with content preview and save).
- **Multi-cluster** — kubeconfig-style `~/.config/nifilens/config.toml`
  with `[[contexts]]`, `current_context`, `Shift+K` to switch at
  runtime. `0600` permission enforcement.
- **Auth variants** — `[contexts.auth]` sub-table with `type =
  password | token | mtls`; `password_env` / `token_env` env-var
  indirection; `proxied_entities_chain` for proxy deployments.
- **TLS** — system trust store by default; optional per-context
  `ca_cert_path` PEM; `insecure_tls = true` with a loud warning.
- **CLI** — `clap` derive with `--config`, `--context`, `--debug`,
  `--log-level`, `--no-color`, `--allow-writes` (reserved). Subcommands:
  `config init`, `config validate`, `version`.
- **Input layer** — typed action enums (`FocusAction`, `HistoryAction`,
  `TabAction`, `AppAction`, per-view `ViewVerb`) plus a shared `Verb`
  trait. Hint bar and help modal are both generated from `Verb`, so
  adding a keybinding updates both surfaces.
- **Cross-tab navigation** — `g` opens a context-sensitive jump menu;
  `Enter` on a Bulletins / Events row lands on the matching component;
  `t` on a row traces it; arena cross-links decorate detail rows with
  a trailing `→`. Tab history via `Shift+←` / `Shift+→` with selection
  restore.
- **Fuzzy find** — global `Shift+F` modal, nucleo-backed, kind · name ·
  path · state columns with highlighted matches.
- **Bulletins ring buffer** — configurable via `[bulletins] ring_size`
  (default 5000; 100..=100_000). Dedup by `(source_id, message_stem)`
  with dynamic `[...]` normalization collapses repeat errors into a
  single `×N` row.
- **Configurable poll cadences** — `[polling.cluster]` per-endpoint
  humantime values (`"10s"`, `"750ms"`). Adaptive scaling up to
  `max_interval` and ±`jitter_percent` jitter.
- **`[ui]` config** — `timestamp_format` (`short` / `iso` / `human`),
  `timestamp_tz` (`utc` / `local`).
- **Visual language** — project-wide bordered-box via
  `widget::panel::Panel`; focused panels flip to thick borders in the
  accent color. `widget::severity` / `widget::run_icon` / `widget::gauge`
  centralize severity labels, run-state glyphs, and fill / spark bars.
- **NifiClient wrapper** around `nifi_rust_client::DynamicClient`
  (`Deref` / `DerefMut`) with typed snapshot helpers for the seven
  endpoints the UI needs; clustered-NiFi `clusterNodeId` pinned at
  login.
- **Central `ClusterStore`** — owns all periodic fetchers (one task per
  endpoint), subscriber-gated for expensive endpoints, snapshot
  mutation only on the UI task. Views subscribe; no per-view pollers.
- **Per-tab `WorkerRegistry`** — on-demand detail fetches swap with
  tab activation / exit.
- **Rotating logging** via `tracing-subscriber` + `tracing-appender` to
  `$XDG_STATE_HOME/nifilens/nifilens.log`; env-filter priority chain
  (`--log-level` > `--debug` > `NIFILENS_LOG` > `RUST_LOG` > `info`).
  `StderrToggle` suppresses stderr output while raw mode is active.
- **TerminalGuard** RAII wrapper + panic hook so the terminal always
  restores cleanly before `color_eyre` prints.
- **Error banners** — transient status-line banners with expandable
  detail modal; never writes to stdout / stderr while the TUI is
  active.
- **`Intent` pipeline** — enum with read + write variants. Write
  intents unconditionally refuse without `--allow-writes` (reserved
  for v2).
- **Crate is lib + bin.** `src/lib.rs` holds every module; `src/main.rs`
  is a thin `nifi_lens::run()` wrapper. Integration tests link against
  the library.
- **Multi-version integration fixture** — `integration-tests/run.sh`
  boots NiFi 2.6.0 (floor) and 2.9.0 (2-node cluster), seeds both via
  `nifilens-fixture-seeder`, runs the `#[ignore]`-gated suite, tears
  down. Versions driven from a single `versions.toml` source of truth
  (compile-time `FIXTURE_VERSIONS` const via `build.rs`); CI drift
  check enforces consistency.
- **Wiremock client tests** for happy path and 401 / 500 surfaces.

### Changed

- **MSRV raised to 1.88** (from 1.85) for `time >= 0.3.47`
  ([RUSTSEC-2026-0009](https://rustsec.org/advisories/RUSTSEC-2026-0009)).
- **`nifi-rust-client`** tracked from 0.5 → 0.10.1 over the release —
  API surface flattened, `traits` module gone, typed provenance
  content bodies, NiFi 2.9.0 support, `clusterNodeId` handling.
- **Row navigation** standardized on `↑`/`↓` + `Home`/`End`. No view
  binds bare `j`, `k`, `[`, or `]`; regression tests enforce this.
- **Keybinding convention** — bare lowercase for view-local, bare
  capital for app-wide, `Ctrl` reserved for quit + text-input helpers.
- **`deny.toml`** allows `BSL-1.0` (arboard transitive) and ignores
  `RUSTSEC-2024-0436` (unmaintained `paste` transitive via ratatui —
  no safe upgrade available).

### Security

- `rustls-webpki` pinned via upstream to pick up fixes later released
  in 0.2.0.

### Notes

- **Read-only.** v0.1 ships no write paths; `--allow-writes` is
  reserved and unused.
- **Intentional omissions for later work.** Per-node repository
  drill-in, processor thread leaderboard, and queue time-to-full
  predictions are known unshipped polish items.

[Unreleased]: https://github.com/maltesander/nifi-lens/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/maltesander/nifi-lens/releases/tag/v0.4.0
[0.3.0]: https://github.com/maltesander/nifi-lens/releases/tag/v0.3.0
[0.2.0]: https://github.com/maltesander/nifi-lens/releases/tag/v0.2.0
[0.1.0]: https://github.com/maltesander/nifi-lens/releases/tag/v0.1.0
