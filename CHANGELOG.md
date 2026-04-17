# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

### Changed

- **`nifi-rust-client` upgraded to 0.10.1.** API surface flattened
  (`*_api()` accessors dropped the suffix, sub-accessor chains collapsed
  into single calls that take the resource id as the first argument),
  the `traits` module is gone, and `update_run_status_{1,2,3,4}` fold
  into a single `update_run_status` per resource. Lineage and provenance
  submissions and event-detail / content fetches now populate
  `clusterNodeId` from the node pinned at login, so clustered NiFi
  deployments no longer hit `400: The cluster node identifier must be
  specified`.
- Browser detail panes for Connection and Process Group now use the same
  nested sub-panel + `Table` visual language as Processor and Controller
  Service. Process Group sub-panels (Controller services, Child groups,
  Recent bulletins) are focusable via `l`, with per-row `c` (copy id),
  `t` (Events cross-link on Recent bulletins), and `Enter` (drill into
  child group).
- Fuzzy find (`f`) now renders as a table with `Kind · Name · Path · State`
  columns; matched characters in the Name column are highlighted bold +
  accent. Equal-score matches tie-break by kind (processors first).
- Context switcher (`K`) now renders as a table with `Name · URL · Version
  · Active` columns; the active context shows `(*)` in its own column and
  the selected row is highlighted via row style instead of a leading
  chevron.
- Row navigation is now arrow-keys only. `j`/`k` aliases removed from
  every interactive list: Browser tree, Bulletins list, Events list, Tracer
  lineage + latest-events, fuzzy-find modal, context switcher, Browser
  Properties modal scroll. Arrow keys (↑/↓) plus Home / End are the single
  row-nav idiom.
- Browser tree drill-in is now `Enter` / `Right` only; drill-out is
  `Backspace` / `Left` only. The `h` / `l` aliases were dropped so those
  keys can drive the new detail-focus cycle. Breadcrumb mode likewise
  drops its `h`/`l` aliases in favor of arrows.
- Hint bar hints for the Browser tab now switch with detail focus — the
  sticky footer shows tree-context / properties-focused / recent-bulletins-
  focused hint sets via a rewritten `browser::hints(state)`.
- **Bulletins `t` now lands on Events, not Tracer.** The cross-link
  pre-fills `source = component` and `time = last 15m`, then
  auto-runs the provenance query. The old Tracer latest-events
  entry is still reachable from Tracer itself, just no longer from
  Bulletins.
- **Browser processor `t` now lands on Events** via the same
  `JumpToEvents` cross-link.
- **`CrossLink` enum grows two new variants**: `JumpToEvents` (used
  by the retargeted `t` from Bulletins/Browser) and `TraceByUuid`
  (used by the new Events-row `t`). `TraceComponent` stays in the
  enum for backwards compatibility but is no longer emitted from
  the UI.
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
- **UI Reorg Phase 4 — Bulletins redesign.** The Bulletins tab is
  now Layout L: list on top with a multi-line detail pane on the
  bottom. The reducer deduplicates by
  `(source_id, strip_component_prefix(message))`, collapsing
  NiFi's noisy repeat errors into a single row with an `×N` count
  column. Severity chips now display ring counts
  (`[E 87] [W 32] [I 0]`). The list columns changed to
  `time / sev / # / source / pg path / message`; the PG path
  column is resolved via the new `BrowserState::pg_path` helper
  and falls back to a muted UUID tail when the Browser tree has
  not yet been populated. The detail pane shows source name, PG
  path, occurrence count, first-seen / last-seen timestamps, raw
  message (unstripped), source id, group id, and per-row action
  hints.
- **UI Reorg Phase 5 — Browser declutter & detail enrichment.**
  The Browser tree drops all trailing status summaries
  (`● 5 ○ 2 ⚠ 0 ⌀ 1`, connection fill, CS state) in favor of
  clean `indent + marker + glyph + name` rows. PG tree markers
  are now colored by a rolled-up health signal: any descendant
  processor `INVALID` → red, `STOPPED` → yellow, else green.
  Per-kind detail panes are rewritten with labeled sections:
  PG detail shows processors / threads / queued / controller
  services / child groups / recent bulletins; Connection detail
  leads with a prominent fill gauge (via the existing
  `widget::gauge::fill_bar` helper) colored by percent;
  Controller Service detail gains a state chip at the top;
  Processor detail gains a "Recent bulletins (N for this
  processor)" section. The Browser render signature is widened
  to receive the bulletin ring, resolving the Phase 3 edge case
  where PG-scoped recent bulletins always showed 0.
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
- Input layer is now driven by typed action enums (`FocusAction`,
  `HistoryAction`, `TabAction`, `AppAction`, `GoTarget`, per-view
  `ViewVerb`) plus a shared `Verb` trait. Every keybinding is the
  single source of truth for its hint-bar entry and help-modal line.
- History back/forward moves from `[`/`]` to `Shift+←`/`Shift+→`.
- Cross-tab jumps are now a two-key leader combo `g <letter>`
  (`g b` Browser, `g e` Events, `g t` Tracer). Replaces Bulletins `t`,
  Browser `t`, Events `t`/`g`, Tracer `t`.
- Bulletins severity filters move from `e`/`w`/`i` to `1`/`2`/`3`.
  Group-by moves from `g` to `Y`.
- Browser breadcrumb mode (`b`) removed — use history navigation.
- Browser properties modal moves from `e` to `p`.
- Tracer detail pane is now tabbed (Attributes | Input | Output) with
  `←`/`→` cycling; `i`/`o`/`a` letter shortcuts removed.
- Tracer attribute-diff toggle moves from `a` to `d`.
- Hint bar and help modal are now generated from the `Verb` trait;
  adding a keybinding automatically updates both surfaces.
- No view binds `j` or `k`. No view binds bare `[` or `]`. Regression
  tests enforce this.

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
- **Bulletins `B` (consecutive-group toggle).** Replaced by `g`
  cycling through group-by modes (`source+msg` / `source` / `off`).
  `source+msg` is the new default and handles non-consecutive
  dedup that the old `B` toggle missed.
- **Bulletins `g` vim jump-to-oldest.** `g` is now the group-mode
  cycle. `Home` still works for jump-to-oldest; the vim `g`/`G`
  pair is deliberately asymmetric now.
- Global `e` chord for expanding the error banner. When the banner is
  shown it becomes the outermost focus; `Enter` expands it, `Esc`
  dismisses.
- Browser interactive breadcrumb focus mode (the static breadcrumb
  strip at the top of the Browser panel is preserved).

### Added

- Single project-wide bordered-box visual language via `widget::panel::Panel`.
  Overview zones, Browser detail panes, and the outer sub-panes of Bulletins /
  Events / Tracer all render inside titled bordered `Panel`s. Focused panels
  flip to thick borders in the accent color.
- Browser detail focus cycle — `l` from the tree on a Processor or Controller
  Service enters detail focus at section 0, `l` cycles sections (wraps),
  `h` or `Esc` returns to the tree. Arrow keys navigate rows in the focused
  section.
- `c` in detail focus copies the focused row's property value or bulletin
  message to the clipboard.
- `t` on a focused Recent-bulletins row opens the Events tab pre-filtered
  to that processor (reuses the existing Bulletins `t` cross-link).
- `widget::severity` consolidates the previously-duplicated
  `format_severity_label` / `severity_style` helpers from three render
  leaves (`pg`, `processor`, `bulletins/render`).
- `widget::panel::Panel` — a builder on top of `ratatui::widgets::Block` that
  centralises the Phase 7 panel style.
- **Events tab** *(UI Reorg Phase 6)* — new cluster-wide provenance
  search tab. Filter bar with `t time` / `T type` / `s source` /
  `u file uuid` / `a attr` inline editors; results list colored by
  event type; detail pane for the selected row. `Enter` runs the
  query, `n` clears, `r` resets filters, `L` raises the 500-event
  cap to 5000. `j`/`k` navigate results; `Esc` returns to the
  filter bar. From a selected row, `t` traces the flowfile in
  Tracer, `g` jumps to the component in Browser, `c` copies the
  uuid.
- **`src/client/events.rs`** — new `ProvenanceQuery` helper wrapping
  NiFi's `POST /provenance` + poll + delete lifecycle, following
  the same `classify_or_fallback` pattern as the tracer client.
- **`view::bulletins::state::recent_for_source_id` and
  `recent_for_group_id`** — pure ring filters returning up to N
  newest bulletins matching a source or group id. Used by the
  Browser detail panes.
- **`BrowserState::PgHealth`**, **`pg_health_rollup`**, and
  **`ChildPgSummary` / `child_process_groups`** — new state
  helpers feeding the tree marker colorization and PG detail
  child-groups listing.
- **Bulletins `g` (cycle group mode).** Cycles
  `source+msg` → `source` → `off` → wrap. Default is `source+msg`.
- **Bulletins `m` (mute source).** Toggles the selected row's
  `source_id` in a session-scoped mute list. Muted rows are
  hidden from the list and counted in a `muted: N` badge on the
  chip row.
- **`BrowserState::pg_path`** — new helper that resolves a
  `group_id` to a human-readable breadcrumb path by walking the
  flow arena. Used by the new Bulletins PG path column.
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
- **Bulletins: grouped rows now collapse across dynamic flowfile
  attributes.** The dedup key used by `g group: source+msg` was
  stem-only — `FlowFile[filename=A]` and `FlowFile[filename=B]` from
  the same processor never collapsed. A new
  `normalize_dynamic_brackets` helper replaces each `[...]` region in
  the stem with `[…]` so same-shaped messages fold into one row with
  a real occurrence count. The detail pane still shows the latest raw
  message verbatim.
- **Events: query-failure no longer displays twice and clears on tab
  switch.** The mid-pane `"query failed: …"` paragraph in the results
  list is gone; the global footer banner is the single source of
  truth for errors. Leaving the Events tab resets a stale `Failed`
  status back to `Idle` so returning shows a clean slate. A new
  global `Esc` at the top level dismisses the status banner.
- **Events: provenance query `400 Message body is malformed`
  resolved.** `build_query` now emits `startDate` in the
  `MM/dd/yyyy HH:mm:ss UTC` format NiFi 2.x actually accepts. The
  previous format had no timezone suffix, so every query submission
  failed with a 400 before a single row could come back.
- **Footer: `nifi-lens vX.Y.Z` is now visible on every frame.** The
  persistent nodewise-diagnostics warning used to hide the crate
  version, which lived in the status-bar left slot. The version
  moves to the hint-bar right edge (below the refresh-age
  indicator), and the sysdiag fallback warning now fires only when
  the mode transitions from nodewise to aggregate rather than on
  every 30 s poll.

### Internal

- New `src/timestamp.rs` module owns all wire-format timestamp
  parsing and display formatting.
- New `src/widget/gauge.rs` module owns `fill_bar` (moved from
  `health::render`) and a new `spark_bar` helper.
- `NodeDiagnostics` and `NodeHealthRow` gain an `available_processors`
  field to drive the Health Load spark-bar max.

### Dependencies

- Bump `nifi-rust-client` from 0.7.0 to 0.8.0.

[Unreleased]: https://github.com/maltesander/nifi-lens/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/maltesander/nifi-lens/releases/tag/v0.4.0
[0.3.0]: https://github.com/maltesander/nifi-lens/releases/tag/v0.3.0
[0.2.0]: https://github.com/maltesander/nifi-lens/releases/tag/v0.2.0
[0.1.0]: https://github.com/maltesander/nifi-lens/releases/tag/v0.1.0
