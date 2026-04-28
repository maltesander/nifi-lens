# AGENTS

## Project Overview

`nifi-lens` is a keyboard-driven terminal UI for observing and debugging
Apache NiFi 2.x clusters. It is powered by
[`nifi-rust-client`](https://docs.rs/nifi-rust-client) used exclusively via
the `dynamic` feature, so one binary works against every supported NiFi
version. v0.1 is read-only, multi-cluster (kubeconfig-style context
switching), and forensics-focused — explicitly a *lens*, not a canvas
replacement.

Top-level tabs (in order): **Overview**, **Bulletins**, **Browser**,
**Events**, **Tracer**.

## Repository Layout

```text
nifi-lens/
├── Cargo.toml / Cargo.lock     # binary crate; publishable; lock committed
├── rust-toolchain.toml         # dev toolchain pin (1.93.0)
├── rustfmt.toml / clippy.toml / deny.toml / release.toml
├── dist-workspace.toml         # cargo-dist config (binary release pipeline)
├── .pre-commit-config.yaml / .markdownlint.yaml
├── CHANGELOG.md / README.md / AGENTS.md / CLAUDE.md / LICENSE
├── .github/workflows/          # ci.yml, publish-crate.yml, release.yml
├── release/release.sh          # cargo-release wrapper, dry-run by default
├── integration-tests/          # Docker-backed live-cluster fixture
└── src/
    ├── lib.rs                  # public entry: pub fn run() -> ExitCode
    ├── main.rs                 # std::process::exit(nifi_lens::run())
    ├── cli.rs                  # clap derive
    ├── error.rs                # NifiLensError (snafu)
    ├── logging.rs              # tracing-subscriber + rotating file
    ├── theme.rs / timestamp.rs / event.rs / layout.rs / test_support.rs
    ├── config/                 # schema, loader, init
    ├── client/                 # NifiClient wrapper (Deref) + TLS + events
    ├── cluster/                # ClusterStore + fetcher tasks + snapshot + subscriber
    ├── input/                  # KeyMap + typed action enums (FocusAction, Verb, …)
    ├── app/                    # run loop, per-view state reducers, ui, navigation, worker
    ├── intent/                 # Intent enum + IntentDispatcher
    ├── view/                   # per-tab views (overview, bulletins, browser, events, tracer)
    └── widget/                 # status_bar, help_modal, context_switcher, panel, severity, …
```

## Architecture

`nifi-lens` follows a standard "ratatui + tokio" split:

- **Single `tokio` multi-thread runtime with a main-thread `LocalSet`** owns everything.
- **UI loop** runs on the main task. It drains an internal `AppEvent`
  channel, mutates state, and redraws (60 fps cap, only when state changed).
- **Terminal event task** converts `crossterm::Event` → `AppEvent::Input`.
- **Central cluster store** (`src/cluster/ClusterStore`) owns all
  periodic NiFi polling. One fetcher task per endpoint emits
  `AppEvent::ClusterUpdate` into the channel; the main loop applies
  each update to `AppState.cluster.snapshot` and fans out
  `AppEvent::ClusterChanged(endpoint)` so views can re-derive their
  projections. Views never poll directly — they subscribe to the
  endpoints they need.
- **View-local workers** remain for on-demand detail fetches (Browser
  `/processors/{id}`, Tracer provenance queries, Events content
  fetches). They are spawned on tab activation and cancelled on tab
  switch via a `WorkerRegistry` (`src/app/worker.rs`) holding at most
  one `JoinHandle<()>`. The same registry also drives
  `cluster.subscribe(...)` / `unsubscribe(...)` on tab change. Workers
  run via `tokio::task::spawn_local` on the main-thread `LocalSet`
  (wired in `src/lib.rs`) because `nifi-rust-client` dynamic traits
  return `!Send` futures.
- **Intent dispatcher** handles one-shot actions (trace a UUID, drill into
  a process group, fetch content for an event, submit a provenance
  query). It runs tasks off the runtime and pushes results back via the
  same channel.

State is mutated **only on the UI task**. No locks, no races.

### Central cluster store

`src/cluster/ClusterStore` owns eleven endpoint fetchers: `root_pg_status`,
`controller_services`, `controller_status`, `system_diagnostics`,
`bulletins`, `connections_by_pg`, `about`, `cluster_nodes`, `tls_certs`,
`version_control`, `parameter_context_bindings`. Each runs as an independent
`tokio::task::spawn_local` future on the main-thread `LocalSet`, pushes
`AppEvent::ClusterUpdate` on success, and sleeps for its base cadence
(scaled adaptively up to `max_interval` based on measured latency, with
±`jitter_percent/100` jitter) before the next tick.

Snapshot mutation is main-loop-only: the `ClusterUpdate` arm in
`src/app/mod.rs` calls `state.cluster.apply_update(...)` and re-emits
`AppEvent::ClusterChanged(endpoint)`. Views observe the change by
matching on the endpoint and invoking their `redraw_*` reducers.

Seven endpoints are **subscriber-gated** — they park when no view is
subscribed: `root_pg_status`, `controller_services`, `connections_by_pg`,
`cluster_nodes`, `tls_certs`, `version_control`,
`parameter_context_bindings`. `WorkerRegistry::ensure` calls
`cluster.subscribe(endpoint, view)` on tab entry and `unsubscribe(...)`
on tab exit.

Per-PG fan-out fetchers (`version_control`,
`parameter_context_bindings`, `connections_by_pg`) bound concurrent
in-flight HTTP requests via `futures::stream::buffer_unordered(N)`.
`N` defaults to 16 and is configurable via `[polling.cluster]
batch_concurrency`. Setting `0` is treated as `1`.

Context switch: `cluster.shutdown()` aborts every fetcher and the store
is rebuilt with the new `NifiClient` in the main loop's
`pending_worker_restart` branch.

Sysdiag nodewise → aggregate fallback is handled inside the
`system_diagnostics` fetcher (logged to `nifilens.log` on transition;
no user-facing banner).

### `nifi-rust-client` integration

All NiFi API access goes through a thin `client` module that:

- Owns the `DynamicClient` (one per active context).
- Exposes high-level helpers for the handful of operations each view needs.
- Centralizes error mapping, retry policy, and `tracing` instrumentation.
- Is the single chokepoint for future mocking and caching.

**When an endpoint is missing or awkward, fix it upstream in
`nifi-rust-client` — do not work around it in `nifi-lens`.** The tool
exists partly to surface and drive those library improvements.

For the `Cargo.toml` dependency declaration and the local-path
development workflow, see "Dependency on `nifi-rust-client`" below.

### Intent pipeline

All user actions route through a single `Intent` enum and a dispatcher.
Write variants exist in the enum from day one so a later write-capable
build does not require restructuring, but no key binding constructs
them in v0.1 and `IntentDispatcher::handle_pure` returns
`NifiLensError::WriteIntentRefused` for every write variant
unconditionally. The `--allow-writes` CLI flag is registered with
`#[arg(hide = true)]` so it doesn't show in `--help`, and `lib.rs`
rejects the flag at startup with a clear "writes not implemented"
error before the runtime even spins up — the runtime guard at the
dispatcher is defense-in-depth.

### Error handling

- **Library-style modules** (`config`, `client`, `intent`): `snafu`, to
  match `nifi-rust-client`.
- **Application edge**: `color-eyre` for pretty crash reports.
- **In-TUI errors**: surfaced as a transient status-line banner with an
  optional detail modal (`Enter` to expand, `Esc` to dismiss). Never
  printed to stdout while the TUI is active — it corrupts the terminal.
- **No panics in data paths.** Panics are only acceptable on config parse
  failures at startup.
- **No `unwrap` / `expect` in production code.** Map `Option`/`Err` to
  `NifiLensError` variants. Tests may `unwrap`.

### Input layer

All keyboard input flows through `src/input/`. A `KeyMap` translates
`crossterm::KeyEvent` into `InputEvent` values carrying typed action enums:

- `FocusAction` (Up/Down/Left/Right/PgUp/PgDn/First/Last/Descend/Ascend/NextPane/PrevPane — Tab/BackTab)
- `HistoryAction` (Back/Forward — `Shift+←`/`Shift+→`)
- `TabAction` (Jump(n) — F1–F5)
- `AppAction` (Quit/Help/ContextSwitcher/FuzzyFind/Jump/Paste/Cut — `g`/`v`/`x` for Jump/Paste/Cut)
- `ViewVerb` — wraps per-view enums (`BulletinsVerb`, `BrowserVerb`, `EventsVerb`, `TracerVerb`)

Every enum implements the `Verb` trait, which is the **single source of
truth** for the chord, label, hint text, enabled predicate, and
truncation priority. The hint bar and help modal are generators that
iterate `Verb::all()` — adding a keybinding cannot desync the two
surfaces.

Views expose a small trait surface (`handle_verb`, `handle_focus`,
`default_cross_link`, `is_text_input_focused`, `handle_text_input`)
instead of raw `KeyEvent` matches. `FocusAction::Descend` is "drill
into the selected thing / activate / submit"; `FocusAction::Ascend` is
"leave the focused pane / cancel pending input". Rule 1a: when a view
has no local descent target, `Enter` falls back to the view's
`default_cross_link` — in practice this is only Bulletins, which
defaults to Browser so historical `Enter`-jumps-to-Browser muscle
memory is preserved.

`F12` dumps the keymap reverse table to the log file. Unadvertised in
the help modal; use it when debugging "why doesn't key X do anything".

Verbs whose shortcut is already visible adjacent to the status-bar hint
strip can opt out of the strip via `Verb::show_in_hint_bar() -> false`
while still appearing in `?` help and being dispatched by the keymap.
Used by the Bulletins severity filters (`1`/`2`/`3`), surfaced by the
`[E n] [W n] [I n]` chips, and by `BulletinsVerb::SearchNext` /
`SearchPrev` which only make sense inside the detail modal.

### Adding a new view

1. Create `src/view/<name>/` with `mod.rs`, `state.rs`, and rendering
   (single `render.rs` or a `render/` submodule for larger views —
   Browser uses the latter). Add `worker.rs` only if the view needs
   on-demand detail fetches (Browser, Events, Tracer do; Overview and
   Bulletins consume the shared cluster snapshot). Mirror Events for
   the small case or Browser for the render-submodule case.
2. Add a `ViewId::<Name>` variant. Update `ViewId::next()` /
   `ViewId::prev()` cycle arms.
3. Create `src/app/state/<name>.rs` with a `<Name>Handler` zero-sized
   type implementing `ViewKeyHandler`.
4. Add one arm to the `dispatch_handler!` macro in
   `src/app/state/mod.rs`.
5. If the view needs live cluster data, subscribe to the relevant
   `ClusterEndpoint`s in `WorkerRegistry::ensure`'s arm for this view,
   and unsubscribe on tab exit. For on-demand detail fetches, spawn
   a view-local worker.
6. Add a render arm to `src/app/ui.rs`'s render dispatch.
7. Add a top-bar label (`src/widget/top_bar.rs`).

All seven steps are mechanical. `dispatch_handler!` means cross-cutting
key-handling dispatch is a single touch, not five.

Where applicable, new views should:

- Use `src/layout.rs` helpers for fixed-header / body / footer splits.
- Use `src/widget/filter_bar.rs` for filter-chip rows.
- Compose `src/widget/scroll.rs` state for modal scroll fields.
- Use `crate::client::status::*` typed enums rather than stringly-typed
  processor / controller-service state matching.

See "Visual language" for the full shared-helper catalogue.

### Logging

`tracing` + `tracing-subscriber` + `tracing-appender` write a daily-rotated
log to the platform state directory resolved via `directories::ProjectDirs`:

- Linux: `$XDG_STATE_HOME/nifilens/` (or `~/.local/state/nifilens/`)
- macOS: `~/Library/Caches/nifilens/`
- Windows: `%LOCALAPPDATA%\nifilens\cache\`

The on-disk filename carries a date suffix — `nifilens.log.YYYY-MM-DD`
— per `tracing_appender::rolling::daily`. To follow the current day's
log:

```bash
tail -f "$(ls -t ~/.local/state/nifilens/nifilens.log.* | head -1)"
```

Old day files accumulate; rotation is purely date-based (no automatic
size pruning). The log directory is created with mode `0700` on Unix.

Level resolution (highest precedence first):

1. `--log-level <off|error|warn|info|debug|trace>` CLI flag.
2. `--debug` CLI flag (equivalent to `--log-level debug`).
3. `NIFILENS_LOG` env var (humantime / `tracing` filter directive).
4. `RUST_LOG` env var (same shape, conventional fallback).
5. Default `info`.

The filter applies to the `nifi_lens` target; library logs from
`nifi-rust-client` etc. require their own filter directive.

**Never** writes to stdout or stderr while the TUI is active.

`F12` dumps the keymap reverse table and per-endpoint subscriber state
to the log file (one `info` line per chord, plus one structured event
per `ClusterEndpoint`). No visible UI change. See "Input layer" for
the user-facing intent.

### Overview Components panel

The top panel of the Overview tab is a 3-row aligned table titled
`Components`: **Process groups** / **Processors** / **Controller
services**. Each row carries a total count plus per-type detail
(per-state counts for processors and CSes; version-sync drift +
input/output port counts for PGs). The PG row collapses its versioning
slot to `all in sync` when no PG is stale, locally modified, or
sync-failed; otherwise it expands to three numeric slots. Display-only
— no focus, no selection.

**Data sources:** processor / PG / port counts come from the shared
`root_pg_status` snapshot. Version-sync counts come from
`controller_status`. Controller-service counts come from a dedicated
`ClusterEndpoint::ControllerServices` fetcher running
`get_controller_services_from_group("root", false, true, false, None)`.
That fetch is **non-fatal**: failure degrades the CS row to a
`cs list unavailable` chip while the other rows still render.

### Overview Nodes panel

The Nodes panel joins `/controller/cluster` membership into each row
via `ClusterEndpoint::ClusterNodes`. Rows show a role/status badge
(`[P·]` / `[·C]` / `[PC]` / `[··]` / `[OFF]` / `[DIS]` / `[CON]`), a
heartbeat-age column, and dim to `theme::muted()` with `───`
placeholders when the node is disconnected or offloaded. Standalone
NiFi servers return 409 on `/controller/cluster`; the fetcher
transparently serves an empty snapshot and the panel degrades to a
4-column layout (no badge, no heartbeat).

`error_is_standalone_409` (in `cluster/fetcher_tasks.rs`) detects this
shape via the error's debug repr. Conservative match on three markers:
the literal `"409"`, an explicit `NotClustered` variant from
`nifi-rust-client`, OR the canonical NiFi message text "Only a node
connected to a cluster". The third matcher is necessary because
`nifi-rust-client` 0.11.0 maps NiFi 2.6.0's 409 response to a
`NotFound` variant whose debug repr contains the message but not the
status code. If any of the three match, the fetcher serves an empty
snapshot; otherwise the endpoint shows as `Failed`.

The detail modal (`Enter` on a node) renders a four-quadrant dashboard:
identity header (badge + status word + roles + heartbeat age + node_id +
joined timestamp), a Resources / Repositories top row (Repositories
shows one row per physical repo with `used / total` in power-of-1024
human units), and an Events / GC bottom row. When standalone, Events
is hidden and GC fills the bottom row.

### TLS certificate expiry

The `ClusterEndpoint::TlsCerts` fetcher probes each node's server
certificate chain (one `tokio-rustls` handshake per node, permissive
verifier, chain captured from the verifier callback). Results join
into `NodeHealthRow.tls_cert`: `not_after` drives the full-chain render
in the node detail modal and a compact trailing chip on Nodes list
rows. The chip shows the earliest `not_after` as days (`Nd`) or
years+months (`Ny Mmo` beyond 1y), muted grey when healthy, blank when
no data.

Cadence via `[polling.cluster] tls_certs` (default `1h`). Standalone
NiFi probes `ctx.url`'s host+port; HTTP-only contexts skip probing
entirely with a one-time `info` log. `publish_node_addresses`
force-wakes the fetcher when the cluster roster changes so probing
catches up immediately.

Severity thresholds (hardcoded for v0.1):

- expired or `<7d` → red / bold
- `7..30d` → yellow
- `>=30d` → muted grey

### Version control drift

The `version_control` endpoint fetcher fans out
`GET /versions/process-groups/{id}` per PG (using PG IDs cached in the
`root_pg_status` snapshot) and stores a
`BTreeMap<PgId, VersionControlSummary>` in the cluster snapshot.
Default cadence `30s` via `[polling.cluster] version_control`.

The Browser tree renders a trailing chip on PG rows whose state ≠
`UP_TO_DATE`: `[STALE]` / `[MODIFIED]` / `[STALE+MOD]` (warning) and
`[SYNC-ERR]` (error). Single-chip alphabet — combined-state PGs render
`[STALE+MOD]`, not two chips.

`FlowIndex` entries for ProcessGroup kinds carry an
`Option<VersionControlInformationDtoState>` re-stamped on every
`ClusterChanged(VersionControl)`, so fuzzy-find drift filters
(`:drift` / `:stale` / `:modified` / `:syncerr` — see "Fuzzy Find")
work without a separate index.

Pressing `m` on a versioned PG row in Browser opens the **version
control modal**: a full-screen overlay with an Identity panel
(registry / bucket / branch / flow / version / state / state
explanation) and a diff body sectioned by component. Diff data comes
from a one-shot view-local worker that fans out
`versions/process-groups/{id}` + `process-groups/{id}/local-modifications`
in parallel. Identity is rendered immediately from the cluster snapshot;
the diff body shows `loading…` until the worker completes.

Modal-scoped chords use a separate `VersionControlModalVerb` enum that
shadows outer-tab keys while the modal is open: `Esc` close, `↑`/`↓`/
`PgUp`/`PgDn`/`Home`/`End` scroll, `/` search, `n`/`Shift+N` next/prev,
`c` copy, `e` toggle environmental (hidden by default), `r` refresh.
Search uses the shared `widget::search` primitives. Below 60×20 the
modal degrades to a single muted line `terminal too small`.

Read-only — no commit / revert / update-version actions in v0.1.

### Parameter Contexts modal

The `parameter_context_bindings` endpoint fetcher fans out
`processgroups().get_process_group(id)` per PG (using PG IDs cached in
the `root_pg_status` snapshot) and stores a
`BTreeMap<PgId, Option<ParameterContextRef>>` in the cluster snapshot.
Default cadence `30s` via `[polling.cluster] parameter_context_bindings`.
Subscriber-gated to Browser views only.

`Enter` (Descend) on the `Parameter context: <name> →` identity row in
a PG detail pane, or `p` on a PG row in the tree, opens the
**parameter-context modal**: a full-screen overlay with an Identity
header, an inheritance chain sidebar, and a resolved-flat parameters
table with `name | value | from | flags`. Flag chips: `[O]` override,
`[S]` sensitive, `[P]` provided, `[!]` unresolved.

Modal-scoped chords use a separate `ParameterContextModalVerb` enum
that shadows outer-tab keys while the modal is open: `Esc` close,
`↑`/`↓`/`PgUp`/`PgDn`/`Home`/`End` scroll, `←`/`→` move chain focus,
`t` toggle by-context view, `s` show shadowed, `u` toggle Used-by
panel, `/` search, `n`/`Shift+N` next/prev match, `c` copy, `r`
refresh. Search uses the shared `widget::search` primitives. Below
60×20 the modal degrades to a single muted line `terminal too small`.

`#{name}` parameter references in processor / controller-service
property values gain a trailing `→` annotation when the owning PG has
a bound context — pressing Enter opens the modal pre-selected to that
parameter (or with `[!]` synthesised for unresolved names). The
`##{...}` escape is honoured: `##{literal}` is *not* annotated.
Multi-ref values (`#{a}#{b}`) annotate but open without a preselect.

Read-only — no edits / creates / submits in v0.1.

### Bulletins ring buffer & detail modal

The Bulletins tab holds a rolling in-memory window of recently-seen
bulletins, capped by `[bulletins] ring_size` in `config.toml` (default
5000, range 100..=100_000; ~1–2 MB at default). The cluster store's
bulletins fetcher polls `flow_api().get_bulletin_board(after, limit=1000)`
on the `[polling.cluster] bulletins` cadence (default 5s), dedups via
the monotonic `id` cursor, and drops from the front when the ring
exceeds capacity. Bulletins views subscribe to
`ClusterEndpoint::Bulletins` and mirror the shared ring into their own
state via `redraw_bulletins`.

Rows are additionally deduplicated by `(source_id, message_stem)` —
the reducer strips NiFi's `ComponentName[id=<uuid>]` prefix and
normalizes dynamic `[...]` regions before hashing, so repeating errors
from the same component collapse into a single row with an `×N` count.
Grouping mode is cycled by `Shift+G` (`source+msg` / `source` / `off`).
`g` triggers `AppAction::Jump` — a context-sensitive cross-tab jump menu.

`i` on a selected bulletin opens a full-screen **detail modal** showing
the full raw message with scroll (`↑↓`/`PgUp`/`PgDn`/`Home`/`End`),
plain-substring `/`-search with `n`/`N` cycling, and `c` to copy. `Esc`
closes. `Enter` is intentionally a no-op inside the modal (committing a
search with Enter used to fall through to a Browser jump; use `g` on
the main tab instead). The modal lives as `BulletinsState::detail_modal`
(not an app-wide `Modal`); `open_detail_modal` snapshots the
`GroupKey` + `GroupDetails` so subsequent ring mutations don't disturb
the open modal.

### Action history modal

Pressing `a` on a Browser row whose component has a UUID
(processor / PG / connection / CS / port) opens the **action history
modal** — a full-screen overlay listing NiFi flow-configuration audit
events filtered by `sourceId`. Backed by a paginator over
`/flow/history` (the worker calls `client.flow().query_history`
directly so it can surface `total` for auto-load gating; the
`flow_actions_paginator` helper in `client::history` wraps
`pagination::flow_history_dynamic` from `nifi-rust-client` and is
reused by the integration test). Rows are paginated 100 at a time and
auto-load when scrolling brings the viewport within 10 rows of the
loaded tail.

State lives on `BrowserState::action_history_modal:
Option<ActionHistoryModalState>`. The state carries a separate
`selected: usize` cursor (the `widget::scroll::VerticalScrollState`
holds only viewport offset, no row selection). The view-local worker
(`spawn_action_history_modal_fetch`) eagerly fetches the first page
then sleeps on a `tokio::sync::Notify` until the reducer wakes it for
the next page.

Modal-scoped chords use a separate `ActionHistoryModalVerb` enum that
shadows outer-tab keys via the keymap shadow gate (mirroring
version-control / parameter-context modals): `Esc` close (cascades
through search → expanded → close), `↑`/`↓`/`PgUp`/`PgDn`/`Home`/
`End` scroll, `Enter` expand selected, `/` search, `n`/`Shift+N`
next/prev match, `c` copy selected row as TSV, `r` refresh from
offset 0. Search shares the `widget::search::SearchState` primitive;
the renderer swaps the hint strip for a `/<query>_` prompt while
input is active and styles the current-match row with
`theme::accent()` + bold.

Below 60×20 the modal degrades to a single muted line `terminal too
small` (matches existing modals).

Read-only — no revert / replay actions in v0.1.

### Sparkline strip

The Browser detail identity panel for processor / PG / connection
rows includes a 3-line inline sparkline on the right half. Backed by
`src/client/history.rs::status_history` which dispatches to the
generated `get_*_status_history` functions and reduces
`StatusHistoryEntity` to a metric-keyed `StatusHistorySeries`.

State lives on `BrowserState::sparkline:
Option<SparklineState>` plus `sparkline_handle:
Option<JoinHandle<()>>`. The state and handle are re-created on every
selection change to a supported kind (processor / PG / connection)
via `AppState::refresh_sparkline_for_selection`, which emits
`PendingIntent::SpawnSparklineFetchLoop`. Selection changes to CS /
Port / Folder rows tear down both the state and the handle.

Worker (`spawn_sparkline_fetch_loop`) loops on the cadence
`config.polling.cluster.status_history` (default `30s`); 404 from
NiFi maps to `AppEvent::SparklineEndpointMissing` (sticky per
selection until the user moves to another row); other errors log at
`warn!` and continue.

Reducer arms in `app::state::update_inner` apply each emit only when
`(kind, id)` matches the active selection — defends against stale
emits between worker abort and exit. UpdateResult carries a
`sparkline_followup: Option<PendingIntent>` that selection-change
paths fold in alongside the primary intent.

Render via the shared `widget::sparkline::render_sparkline_row`
helper (label + glyphs + 'peak N' suffix), iterated three times per
kind:

| Component | Row 1 | Row 2 | Row 3 |
|---|---|---|---|
| Processor | `in` flowfiles | `out` flowfiles | `task` time |
| PG | `in` flowfiles | `out` flowfiles | `queue` count |
| Connection | `in` flowfiles | `out` flowfiles | `queue` count |

Below 24 cells of identity-inner width (2× `SPARKLINE_MIN_RIGHT_HALF_WIDTH`)
the strip is suppressed and the identity panel reverts to full width
(responsive fallback). No focus, no chord — purely periodic display.

### Tracer content viewer modal

Full-screen modal opened with `i` on the Tracer Content sub-tab.
State lives on `AppState.tracer.content_modal: Option<ContentModalState>`.
Modal-scoped keys use `ContentModalVerb`; while open, the modal's
`Verb::all()` drives the footer hint strip and help modal section, and
the keymap shadows outer-tab keys.

Streaming: `provenance_content_range(event_id, side, offset, len)`
fetches 512 KiB chunks. A reducer auto-fires the next chunk when the
viewport bottom comes within 100 lines of the decoded tail. Per-side
ceilings are configured via the `[tracer.ceiling]` nested table (keys:
`text`, `tabular`, `diff`; defaults `4 MiB` / `64 MiB` / `16 MiB`;
`"0"` → unbounded). The legacy `modal_streaming_ceiling` flat key is
honored for one release with a deprecation warn.

Diff mode is bounded by `[tracer.ceiling] diff` and uses
`similar::TextDiff::from_lines` with 3-line context. Diff eligibility
requires both sides available, MIME pair matching the allowlist (or
UTF-8 fallback when neither side declares a MIME), declared size ≤ the
diff ceiling per side, and non-identical bytes. `Ctrl+↓` / `Ctrl+↑`
navigate between changed regions; the rendered hunk header
(`@@ input Lx · output Ly @@`) appends `· N changes` from the change
index.

Search primitives (`MatchSpan`, `SearchState`, `compute_matches`) are
shared with the Bulletins detail modal via `src/widget/search.rs`.

### Tracer tabular content (Parquet & Avro)

`ContentRender::Tabular { format, schema_summary, body, decoded_bytes, truncated }`
is produced when `classify_content` sees `PAR1` or `Obj\x01` magic in
the first four bytes. Decoders live in `src/client/tracer/content.rs`
(`decode_parquet` via `ParquetRecordBatchReaderBuilder` +
`arrow::json::LineDelimitedWriter`; `decode_avro` via `apache_avro::Reader` +
`from_value::<serde_json::Value>`). Decoder errors are caught inside
`classify_content`, logged at `warn!`, and surfaced as `Hex` —
classifier signature stays infallible.

**Per-side ceiling** is resolved after the first chunk arrives (the
reducer sniffs magic and records the resolved ceiling on
`SideBuffer.effective_ceiling`). Parquet's footer lives at EOF, so a
ceiling-hit fetch cannot decode at all and falls back to `Hex` with a
chip identifying the cause (`parquet truncated at N MiB — raise
[tracer.ceiling] tabular or use "s" to save`). Avro is streamable and
degrades gracefully via `truncated = true`.

**Diff:** Tabular sides diff iff their `format` tags match. Diff input
is `Tabular::body`; schema lines do not contribute hunks. The `diff`
ceiling caps the per-side input fed into `similar::TextDiff::from_lines`.

Parquet/avro fixture chains live under `diff-pipeline` — see
"Integration test fixture" below.

### Poll intervals

All periodic NiFi fetches are owned by `src/cluster/ClusterStore`.
Base cadences come from `[polling.cluster]` in `config.toml` (keys:
`root_pg_status`, `controller_services`, `controller_status`,
`system_diagnostics`, `bulletins`, `cluster_nodes`, `tls_certs`,
`connections_by_pg`, `version_control`, `parameter_context_bindings`,
`status_history`, `about`, plus the adaptive knobs `max_interval`
and `jitter_percent`). For scaling and subscriber-gating behavior,
see "Central cluster store". `status_history` is selection-scoped
rather than cluster-wide — it cadences the per-row sparkline worker
(see "Sparkline strip"), not a `ClusterStore` fetcher.

Values use the humantime format (`"10s"`, `"750ms"`, `"2m"`). The
loader emits a `tracing::warn!` (into the rotating log file) for values
outside the recommended range but accepts them as-is — no hard
rejection.

Events in-flight query polling (750 ms) and Tracer content in-flight
polling (500 ms) stay hardcoded — those are internal query mechanics,
not user cadences.

### Visual language

A single project-wide bordered-box visual language goes through
`widget::panel::Panel`. Focused panels flip to `BorderType::Thick` plus
an accent color; unfocused panels use plain borders and
`theme::border_dim()`. New interactive sub-panels should use arrow keys
(`↑`/`↓`) for row navigation — `j`/`k` aliases are not used app-wide.

Severity rendering (labels, colors, icons) is consolidated in
`widget::severity` and `widget::run_icon`; call these helpers rather
than reintroducing inline `Color::*`/`Modifier::*` constructors.

Shared helpers for modal, filter-bar, layout, and typed-state code:

- `src/widget/scroll.rs` — `VerticalScrollState` /
  `BidirectionalScrollState` primitives composed by both full-screen
  modals. Covers scroll-by / page-up-down / jump-top-bottom / horizontal
  scroll math; callers hold content dimensions and drive the widget.
- `src/widget/filter_bar.rs` — `FilterChip` + `build_chip_line` for
  horizontal chip rows (Events + Bulletins top rows). Tab-specific
  second rows stay in their render modules.
- `src/widget/search.rs` — `SearchState` + `compute_matches`, shared
  by both full-screen detail modals.
- `src/layout.rs` — `split_header_body_footer` / `split_two_rows` /
  `split_two_cols` helpers for common fixed-plus-flex split shapes.
- `src/bytes.rs` — `KIB` / `MIB` / `GIB` unit constants plus
  `FIXTURE_HEAP_*` test-fixture baselines. Prefer these over raw
  `N * 1024 * 1024` literals.
- `src/client/status.rs` — `ProcessorStatus` + `ControllerServiceState`
  typed enums. Use `from_wire(&str)` for case-insensitive parsing and
  the `style()` / `badge_style()` / `referencing_style()` / `icon()`
  methods rather than matching on raw strings.
- `src/timestamp.rs` — `format_age(Option<Duration>)` for
  `SystemTime`-derived ages and `format_age_secs(u64)` for
  already-computed second counts (e.g. NiFi heartbeat ages).
- `src/test_support.rs` — `fresh_state` / `tiny_config` plus
  `default_fetch_duration()` and `test_backend(height)` (with the
  `TEST_BACKEND_WIDTH` / `_SHORT` / `_MEDIUM` / `_TALL` constants).

Folders in the Browser tree are a **reducer-only** construct. The
client walker emits a flat list of CS / queue / port / processor
nodes; `apply_tree_snapshot` post-processes each PG's children to
synthesize `Folder(Queues)` / `Folder(ControllerServices)` arena rows
and re-parent the leaves. Folders are never cross-link targets, never
emit detail-fetch requests, and are filtered out of the fuzzy-find
flow index.

### Browser cross-navigation

Any row rendered in a Browser detail sub-panel whose value resolves to
a node in the arena is annotated with a trailing `→`. Pressing Enter
(Descend) on such a row emits `CrossLink::OpenInBrowser`, reusing the
reducer arm that already handles Bulletins → Browser and CS Referencing
→ Browser jumps. Resolution goes through `BrowserState::resolve_id`,
which gates on a canonical-UUID shape check before scanning
`state.nodes` (linear scan, once per annotatable row).

Annotated surfaces include connection endpoints (FROM/TO),
processor/CS property values that are UUIDs (typically CS references),
processor connection rows (→ opposite endpoint), and PG-owned
controller services. CS & Port Identity panels resolve `parent` UUIDs
to the owning PG's name display-only (parent is always reachable via
Left/Ascend).

Selected-relationships on connections are intentionally not surfaced
in the processor Connections section: that data lives on
`ConnectionDTO` (fetched by `browser_connection_detail`), not on the
status snapshot the tree walker reads.

Connection endpoint IDs (`source_id` / `destination_id` on
`NodeStatusSummary::Connection`) are NOT populated by the recursive
status endpoint — NiFi leaves those fields null on
`ConnectionStatusSnapshotDto`. `browser_tree` therefore fires a
parallel `/process-groups/{pg_id}/connections` fetch per PG after the
status walk (parallelized with `futures::future::join_all`), builds a
`connection_id → (source_id, destination_id)` map, and backfills the
arena's Connection rows. Per-PG fetch failures are logged and skipped.

### Fuzzy Find

The `Shift+F` modal searches a shared `FlowIndex` built from the
Browser arena (processors, PGs, controller services, connections,
ports — folders excluded). The haystack per entry is
`"{name} {kind_label} {group_path}"` lowercased; nucleo scores and the
top 50 are shown.

A leading colon-prefixed token narrows the corpus before fuzzy scoring
via a `QueryFilter` enum:

- Kind aliases: `:proc`, `:pg`, `:cs`, `:conn`, `:in`, `:out`.
- Drift aliases (PG-scoped): `:drift`, `:stale`, `:modified`, `:syncerr`.

Parsing happens inside `FuzzyFindState::rebuild_matches`; an unknown
`:token` (or any non-leading occurrence) is treated as plain query
text. A read-only chip row above the query line reflects the parsed
filter. There is no chip-toggle keybinding — the query string is the
single source of truth.

## Dependency on `nifi-rust-client`

`nifi-lens` depends on `nifi-rust-client` with the `dynamic` feature,
declared in `Cargo.toml`:

```toml
nifi-rust-client = { version = "…", features = ["dynamic"] }
```

At the bottom of `Cargo.toml` there is a **commented-out**
`[patch.crates-io]` block:

```toml
# [patch.crates-io]
# nifi-rust-client = { path = "../nifi-rust-client/crates/nifi-rust-client" }
```

**Local development workflow:**

1. When iterating against an unreleased change in the sibling
   `../nifi-rust-client` worktree, uncomment the block locally.
2. Run `cargo build` — Cargo now picks up the local path.
3. **Before pushing**, recomment the block.

A forgotten uncomment will break CI on the first cargo job (the sibling
path does not exist on GitHub runners). That is the intended guardrail —
do not try to teach CI to tolerate it.

**Dependencies are kept alphabetically sorted** in `Cargo.toml`. New
deps land in the correct position, never appended at the bottom.

## Build & Test

| When | Command |
|---|---|
| After small changes | `cargo check`, `cargo build` |
| Run the binary | `cargo run` |
| After changing a module | `cargo test <module>` |
| Before committing | `cargo test --all-features && pre-commit run --all-files` |
| Full clippy | `cargo clippy --all-targets --all-features -- -D warnings` |
| Format check | `cargo fmt --all -- --check` |
| Rustdoc (warning-free) | `RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps` |
| Dependency audit | `cargo deny check` |
| MSRV check | `RUSTUP_TOOLCHAIN=1.88 cargo check --all-features` |

**MSRV is `1.88`.** `rust-toolchain.toml` pins `1.93.0` for development;
CI enforces the `1.88` floor via `RUSTUP_TOOLCHAIN` override. MSRV was
raised from 1.85 to 1.88 to pull in `time >= 0.3.47`, which fixes
[RUSTSEC-2026-0009](https://rustsec.org/advisories/RUSTSEC-2026-0009).

Rustdoc discipline: doc comments on `pub` items must not `[`link`]` to
private items. CI fails the doc build on private-link warnings; use
plain backticks instead.

### Pointing the binary at a local NiFi cluster

Create a config file in the platform config dir (see README.md
"Configuration" for the per-OS path, or run `nifilens config init`),
export the referenced `password_env` variable, and run
`cargo run -- --context dev`.

### Integration test fixture

The integration harness at `integration-tests/` brings up a standalone
NiFi 2.6.0 (floor) and a 2-node NiFi 2.9.0 cluster (ceiling) with
ZooKeeper, seeds each via `nifilens-fixture-seeder`, and runs
`cargo test --test 'integration_*' -- --ignored` against both. One
command does it all:

```bash
./integration-tests/run.sh
```

`run.sh` invokes `scripts/download-nars.sh` first to fetch
`nifi-parquet-nar` (and its transitive `nifi-hadoop-libraries-nar`
dependency) from Maven Central into a gitignored cache. The NARs are
mounted per-version into each NiFi service's
`/opt/nifi/nifi-current/nar_extensions/` autoload directory. The
`apache/nifi` base images don't bundle the standalone Parquet writer
— only the Iceberg-specific variant — so this mount is required for
`diff-parquet-writer` to enable.

For a live-dev workflow (fixture stays up, point the TUI at it,
iterate without re-seeding), use
`docker compose -f integration-tests/docker-compose.yml up -d`, run the
seeder with `--skip-if-seeded`, and `cargo run -- --config
integration-tests/nifilens-config.toml --context dev-nifi-2-9-0`.
`--skip-if-seeded` makes re-runs a no-op when the fixture marker PG
(`nifilens-fixture-v7`) is already present.

**Fixture inventory** — top-level marker PG `nifilens-fixture-v7`
contains nine process groups, two parameter contexts, and four
top-level controller services:

- PGs: `healthy-pipeline` (with nested `ingest`/`enrich`),
  `noisy-pipeline`, `backpressure-pipeline`, `invalid-pipeline`,
  `bulky-pipeline`, `diff-pipeline`, `versioned-clean`,
  `versioned-modified`, `parameterized-pipeline`.
- Parameter contexts: `fixture-pc-base` (with `kafka_bootstrap`,
  `retry_max`, sensitive `db_password`) and `fixture-pc-prod`
  (inherits from `fixture-pc-base`, overrides `retry_max`, adds
  `region`). `parameterized-pipeline` is bound to `fixture-pc-prod`.
- CSes: `fixture-json-reader` ENABLED, `fixture-json-writer` ENABLED,
  `fixture-csv-reader` DISABLED, `fixture-broken-writer`
  INVALID/DISABLED.

What each pipeline exercises:

- `healthy-pipeline/enrich` starts with `ConvertRecord` referencing
  `fixture-json-reader`/`-writer`, then `UpdateAttribute-enrich` →
  `UpdateAttribute-cleanup` → `LogAttribute-INFO` — exercises
  CS-referencing coverage on all NiFi versions including 2.6.0.
- `parameterized-pipeline` is bound to `fixture-pc-prod` (which
  inherits `fixture-pc-base`). Contains `LogAttribute-parameterized`
  (`Log Payload = "connecting to #{kafka_bootstrap}"`, param reference;
  `Log Prefix = "##{literal_text}"`, escape — should NOT be annotated)
  and `UpdateAttribute-parameterized` (dynamic properties
  `broker = "#{kafka_bootstrap}"` and `max_retries = "#{retry_max}"` —
  exercises `#{name}` cross-link annotations in the processor properties
  modal). Exercises the Browser parameter-context modal (`p`) and the
  `#{name}` cross-link annotation logic in T16.
- `versioned-clean` and `versioned-modified` are committed to a NiFi
  Registry bucket on seed; `versioned-modified` then has one property
  mutated locally so it shows `[MODIFIED]` (or `[STALE+MOD]` after a
  registry-side update) drift, exercising the version-control modal.
- `bulky-pipeline` produces ~1.5 MiB random-text flowfiles at a low
  rate — content for Tracer streaming / truncation testing.
- `diff-pipeline` generates ~180 KiB structured JSON flowfiles (1000
  sensor records from an embedded `diff_payload.json` asset) and
  exercises the Tracer content viewer's diff tab via three sink
  chains:
  - JSON↔CSV: `UpdateRecord-json` → `ConvertRecord` →
    `UpdateRecord-csv` → `LogAttribute-INFO`. Mid-stage produces
    diffable JSON↔JSON and CSV↔CSV pairs; the JSON↔CSV stage is
    grayed out (mime mismatch).
  - Parquet: `ConvertRecord-parquet` → `UpdateRecord-parquet` →
    `UpdateRecord-parquet-mark-deleted` → `LogAttribute-parquet`.
  - Avro: `ConvertRecord-avro` → `UpdateRecord-avro` →
    `UpdateRecord-avro-mark-deleted` → `LogAttribute-avro`.

  `UpdateRecord-{fmt}` rewrites WARN status rows (≈⅓ of records);
  `UpdateRecord-{fmt}-mark-deleted` rewrites `/id` on
  `SENSOR-0500…0999` to `DELETED-5xx…9xx` (≈½ of records). Both
  emit `CONTENT_MODIFIED` events with same-format input/output
  claims, so the diff renders real per-row changes. Note: NiFi often
  reports `inputContentClaim == outputContentClaim` on
  `CONTENT_MODIFIED` even when bytes differ — claim ID is a logical
  handle, not a content hash. Always fetch both sides.
  `diff-pipeline` owns its own scoped controller services
  (`diff-{json,csv,parquet,avro}-{reader,writer}`,
  `diff-csv-writer-out`).

All fixture pipelines work on the 2.6.0 floor. Some NiFi processor
*property keys* drift between minor versions even when display names
are stable — setting a property by display name when the real key
differs silently turns it into a dynamic attribute. The seeder handles
known cases via `fixture::custom_text_property_key(version)` and
similar helpers; bumping the marker name invalidates stale fixtures
automatically.

### Bumping the NiFi ceiling version

When `nifi-rust-client` adds support for a new NiFi version:

1. Update `nifi-rust-client` in the root `Cargo.toml`.
2. Edit `integration-tests/versions.toml`.
3. Edit `integration-tests/docker-compose.yml`.
4. Edit `integration-tests/nifilens-config.toml`.
5. Edit `tests/common/versions.rs` `port_for` match arm.
6. Run `./integration-tests/run.sh` locally to verify.
7. Push. CI's drift check enforces steps 2–4 consistency.

The **floor version 2.6.0 never drops** — it stays pinned forever so
the dynamic client is always tested against the oldest supported NiFi.

## Release

Releases are driven by
[`cargo-release`](https://crates.io/crates/cargo-release) via
`release/release.sh`, a thin passthrough wrapper.

**`cargo-release` is dry-run by default.** Adding `--execute` performs
the release.

| Command | Effect |
|---|---|
| `release/release.sh patch` | Dry-run a patch release. Prints the plan, touches nothing. |
| `release/release.sh minor` | Dry-run a minor release. |
| `release/release.sh major` | Dry-run a major release. |
| `release/release.sh patch --execute` | Bump version, rewrite `CHANGELOG.md`, commit, tag, push. |

**The release commit updates** `Cargo.toml` `version`, `Cargo.lock`
(cascades automatically), and `CHANGELOG.md` (`## [Unreleased]` becomes
`## [X.Y.Z] — YYYY-MM-DD`; a fresh `## [Unreleased]` stanza is inserted
above; the compare link at the bottom is rewritten).

**After the tag is pushed**, two workflows fire on every `v*.*.*` tag
and run independently:

1. `.github/workflows/publish-crate.yml` — verifies tag matches
   `Cargo.toml`, runs the full check suite, `cargo publish`es to
   crates.io using `CARGO_REGISTRY_TOKEN`.
2. `.github/workflows/release.yml` — autogenerated by cargo-dist.
   Builds per-target archives for Linux (x86_64 / aarch64, gnu + musl),
   macOS (x86_64 / aarch64), and Windows (x86_64); uploads them plus a
   shell installer, PowerShell installer, and Homebrew formula to a
   GitHub Release; writes release notes from the `## [X.Y.Z]` CHANGELOG
   section.

The local machine never publishes. `cargo-release` is configured with
`publish = false` so `CARGO_REGISTRY_TOKEN` lives only in GitHub.

### cargo-dist configuration

The binary release pipeline is configured in `dist-workspace.toml` and
`Cargo.toml`'s `[profile.dist]`. Never hand-edit
`.github/workflows/release.yml` — it's regenerated. To change targets,
installers, or the cargo-dist version: edit `dist-workspace.toml`, run
`dist generate` (cargo-dist ≥ 0.28; binary is `dist`), commit the
regenerated workflow alongside the config change.

Homebrew tap (optional, not yet configured): create a
`maltesander/homebrew-tap` repo, add `tap = "..."` and
`formula = "..."` keys to `dist-workspace.toml`'s `[dist]` table,
regenerate, and add a `HOMEBREW_TAP_TOKEN` repo secret with
`contents: write` on the tap repo.

### Installing `cargo-release` and `cargo-dist`

```bash
cargo install cargo-release --locked
cargo install cargo-dist --locked   # binary is `dist`
```

## Documentation Policy

| Audience | Location | Format |
|---|---|---|
| Users (install, usage, config, screencasts) | `README.md` | Rendered on GitHub + crates.io |
| Contributors (architecture, patterns, procedures) | `AGENTS.md` (this file) | Markdown |
| Version history | `CHANGELOG.md` | Keep a Changelog — rewritten by `cargo-release` |
| API rustdoc | Inline `///` comments | `cargo doc --no-deps` must be warning-free (CI enforces) |
| Design specs and implementation plans | `docs/` locally | Markdown (gitignored) |

**Rules:**

- `docs/` is gitignored. Do **not** hard-link into it from any committed
  file. Specs and plans are private to the working copy.
- When architecture or patterns change, update `AGENTS.md` in the same
  commit.
- When user-visible behavior changes, update `README.md` and
  `CHANGELOG.md` in the same commit.
- Every new dependency goes into `Cargo.toml` in its correct
  alphabetical position.

## References

| Resource | URL |
|---|---|
| `nifi-rust-client` docs | <https://docs.rs/nifi-rust-client> |
| `ratatui` book | <https://ratatui.rs/> |
| `snafu` docs | <https://docs.rs/snafu> |
| NiFi 2.x REST API | <https://nifi.apache.org/nifi-docs/rest-api.html> |
| Keep a Changelog | <https://keepachangelog.com/en/1.1.0/> |
| Semantic Versioning | <https://semver.org/spec/v2.0.0.html> |
| `cargo-release` docs | <https://github.com/crate-ci/cargo-release> |
