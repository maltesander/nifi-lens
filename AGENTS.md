# AGENTS

## Project Overview

`nifi-lens` is a keyboard-driven terminal UI for observing and debugging
Apache NiFi 2.x clusters, powered by
[`nifi-rust-client`](https://docs.rs/nifi-rust-client) via the `dynamic`
feature so one binary works against every supported NiFi version. v0.x
is read-only, multi-cluster (kubeconfig-style context switching), and
forensics-focused — explicitly a *lens*, not a canvas replacement.

Top-level tabs: **Overview**, **Bulletins**, **Browser**, **Events**,
**Tracer**.

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
  `AppEvent::ClusterUpdate`; the main loop applies it to
  `AppState.cluster.snapshot` and fans out
  `AppEvent::ClusterChanged(endpoint)`. Views subscribe to the
  endpoints they need; they never poll directly.
- **View-local workers** handle on-demand detail fetches (Browser
  `/processors/{id}`, Tracer provenance queries, Events content
  fetches). Spawned on tab activation, cancelled on tab switch via
  `WorkerRegistry` (`src/app/worker.rs`); the same registry drives
  `cluster.subscribe(...)` / `unsubscribe(...)` on tab change. Workers
  run via `tokio::task::spawn_local` on the main-thread `LocalSet`
  because `nifi-rust-client` dynamic traits return `!Send` futures.
- **Intent dispatcher** handles one-shot actions (trace a UUID, drill
  into a PG, fetch event content, submit a provenance query). Tasks
  push results back via the same channel.

State is mutated **only on the UI task**. No locks, no races.

**Modal conventions (apply to every full-screen modal unless noted):**
each modal owns a `*ModalVerb` enum that embeds `Common(CommonVerb)`
for shared chords (`Esc`/`/`/`n`/`Shift+N`/`c`/`r`); body keys (`↑/↓/
PgUp/PgDn/Home/End`) are modal-specific. The keymap shadows outer-tab
keys via `input::modal_gate::ModalGate` (one impl per modal in
`src/input/modal_gate.rs`). Search uses `widget::search` primitives.
Below `widget::modal::MIN_WIDTH × MIN_HEIGHT` the modal degrades via
`widget::modal::render_too_small`. v0.1 modals are **read-only**.

### Central cluster store

`ClusterStore` owns eleven endpoint fetchers: `root_pg_status`,
`controller_services`, `controller_status`, `system_diagnostics`,
`bulletins`, `connections_by_pg`, `about`, `cluster_nodes`, `tls_certs`,
`version_control`, `parameter_context_bindings`. Each runs as an
independent `tokio::task::spawn_local` future, pushes
`AppEvent::ClusterUpdate` on success, and sleeps for its base cadence
(scaled adaptively up to `max_interval` based on measured latency,
with ±`jitter_percent/100` jitter).

Snapshot mutation is main-loop-only: the `ClusterUpdate` arm in
`src/app/mod.rs` calls `state.cluster.apply_update(...)` and re-emits
`AppEvent::ClusterChanged(endpoint)`. Views match on the endpoint and
invoke their `redraw_*` reducers.

Seven endpoints are **subscriber-gated** — they park when no view is
subscribed: `root_pg_status`, `controller_services`,
`connections_by_pg`, `cluster_nodes`, `tls_certs`, `version_control`,
`parameter_context_bindings`. `WorkerRegistry::ensure` calls
`cluster.subscribe(endpoint, view)` on tab entry and `unsubscribe`
on tab exit.

Per-PG fan-out fetchers (`version_control`,
`parameter_context_bindings`, `connections_by_pg`) bound concurrent
in-flight HTTP requests via `futures::stream::buffer_unordered(N)`.
`N` defaults to 16 and is configurable via `[polling.cluster]
batch_concurrency` (`0` is treated as `1`).

Context switch: `cluster.shutdown()` aborts every fetcher and the
store is rebuilt with the new `NifiClient` in the main loop's
`pending_worker_restart` branch. Sysdiag nodewise → aggregate
fallback is handled inside the `system_diagnostics` fetcher (logged
on transition; no user-facing banner).

### `nifi-rust-client` integration

All NiFi API access goes through a thin `client` module that:

- Owns the `DynamicClient` (one per active context).
- Exposes high-level helpers per view.
- Centralizes error mapping, retry policy, `tracing` instrumentation.
- Is the single chokepoint for future mocking and caching.

**When an endpoint is missing or awkward, fix it upstream in
`nifi-rust-client` — do not work around it in `nifi-lens`.** The tool
exists partly to surface and drive those library improvements. See
"Dependency on `nifi-rust-client`" below for the local-path workflow.

### Intent pipeline

All user actions route through a single `Intent` enum and a dispatcher.
Write variants exist from day one (so a later write-capable build does
not require restructuring), but no key binding constructs them in
v0.x and `IntentDispatcher::handle_pure` returns
`NifiLensError::WriteIntentRefused` for every write variant. The
`--allow-writes` CLI flag is `#[arg(hide = true)]` and `lib.rs`
rejects it at startup before the runtime spins up — the dispatcher
guard is defense-in-depth.

### Error handling

- **Library-style modules** (`config`, `client`, `intent`): `snafu`
  to match `nifi-rust-client`.
- **Application edge**: errors bubble up to `lib::run()`, which prints
  to stderr and returns a non-zero `ExitCode`. No pretty crash
  formatter is currently installed.
- **In-TUI errors**: transient status-line banner with optional
  detail modal (`Enter` expand, `Esc` dismiss). Never written to
  stdout while the TUI is active — it corrupts the terminal.
- **No panics in data paths.** Panics are only acceptable on config
  parse failures at startup.
- **No `unwrap` / `expect` in production code.** Map `Option`/`Err`
  to `NifiLensError` variants. Tests may `unwrap`.

### Input layer

All keyboard input flows through `src/input/`. A `KeyMap` translates
`crossterm::KeyEvent` into `InputEvent` carrying typed action enums:

- `FocusAction` (Up/Down/Left/Right/PgUp/PgDn/First/Last/Descend/Ascend/NextPane/PrevPane — Tab/BackTab)
- `HistoryAction` (Back/Forward — `Shift+←`/`Shift+→`)
- `TabAction` (Jump(n) — F1–F5)
- `AppAction` (Quit/Help/ContextSwitcher/FuzzyFind/Jump/Paste/Cut — `g`/`v`/`x`)
- `ViewVerb` — wraps per-view enums (`BulletinsVerb`, `BrowserVerb`, `EventsVerb`, `TracerVerb`)

Shared chords (`Refresh`, `Copy`, `OpenSearch`, `SearchNext`,
`SearchPrev`, `Close`) live on `CommonVerb` (`src/input/verb.rs`).
Per-view enums embed a `Common(CommonVerb)` arm and list opt-in
variants in `Verb::all()` — chord/label/hint metadata defined once.
Modal shadow dispatch goes through `input::modal_gate::ModalGate` —
one impl per modal, chained inside `KeyMap::translate`. Adding a
new modal is a single gate impl.

Every enum implements the `Verb` trait, the **single source of truth**
for chord, label, hint text, enabled predicate, and truncation
priority. Hint bar and help modal iterate `Verb::all()` — adding a
keybinding cannot desync the two surfaces.

Views expose a small trait surface (`handle_verb`, `handle_focus`,
`default_cross_link`, `is_text_input_focused`, `handle_text_input`)
instead of raw `KeyEvent` matches. `FocusAction::Descend` =
drill/activate/submit; `Ascend` = leave focused pane / cancel input.
When a view has no local descent target, `Enter` falls back to
`default_cross_link` (Bulletins → Browser).

`F12` dumps the keymap reverse table + subscriber state to the log
(unadvertised debug aid).

Verbs already visible adjacent to the hint strip can opt out via
`Verb::show_in_hint_bar() -> false` while still appearing in `?`
help (Bulletins severity `1`/`2`/`3`; `SearchNext`/`SearchPrev`).

### Adding a new view

1. Create `src/view/<name>/` with `mod.rs`, `state.rs`, and rendering
   (single `render.rs` like Events, or a `render/` submodule like
   Browser). Add `worker.rs` only if on-demand detail fetches needed.
2. Add a `ViewId::<Name>` variant; update `next()` / `prev()` arms.
3. Create `src/app/state/<name>.rs` with a `<Name>Handler` ZST
   implementing `ViewKeyHandler`.
4. Add one arm to `dispatch_handler!` in `src/app/state/mod.rs`.
5. For live cluster data, subscribe/unsubscribe `ClusterEndpoint`s
   in `WorkerRegistry::ensure`. For on-demand fetches, spawn a
   view-local worker.
6. Add a render arm to `src/app/ui.rs`.
7. Add a top-bar label (`src/widget/top_bar.rs`).

All seven steps are mechanical. See "Visual language → Shared
helpers" for reusable layout / modal / filter_bar / scroll helpers.

### Logging

`tracing` + `tracing-subscriber` + `tracing-appender` write a daily-rotated
log under `directories::ProjectDirs` (Linux: `$XDG_STATE_HOME/nifilens/`;
macOS: `~/Library/Caches/nifilens/`; Windows: `%LOCALAPPDATA%\nifilens\cache\`).
Filename `nifilens.log.YYYY-MM-DD`; rotation is date-based (no automatic
size pruning); directory is mode `0700` on Unix.

Level resolution (highest precedence first): `--log-level`, `--debug`
(= debug), `NIFILENS_LOG`, `RUST_LOG`, default `info`. Filter applies
to the `nifi_lens` target; library logs need their own directive.
**Never** writes to stdout/stderr while the TUI is active.

### Overview Components panel

3-row aligned `Components` table: **Process groups** / **Processors**
/ **Controller services**, each with a total plus per-type detail
(state counts for processors and CSes; version-sync drift + port
counts for PGs). PG version slot collapses to `all in sync` when no
PG is stale/locally-modified/sync-failed. Display-only — no focus.

Data sources: processor / PG / port counts from `root_pg_status`;
version-sync from `controller_status`; CS counts from
`ClusterEndpoint::ControllerServices`. The CS fetch is **non-fatal**
— failure degrades the row to a `cs list unavailable` chip while
the rest still render.

### Overview Nodes panel

Joins `/controller/cluster` membership into each row via
`ClusterEndpoint::ClusterNodes`. Rows show a role/status badge
(`[P·]` / `[·C]` / `[PC]` / `[··]` / `[OFF]` / `[DIS]` / `[CON]`),
heartbeat-age column, and dim to `theme::muted()` with `───`
placeholders when disconnected/offloaded. Standalone NiFi servers
return 409 on `/controller/cluster`; the fetcher transparently
serves an empty snapshot and the panel degrades to 4-col.

`error_is_standalone_409` (`cluster/fetcher_tasks.rs`) detects this
via the error's debug repr, matching `"409"`, the `NotClustered`
variant, or the canonical NiFi message "Only a node connected to a
cluster" (the last is needed because some `nifi-rust-client` versions
map the 409 to `NotFound` without the status code in the repr). On
match the fetcher serves an empty snapshot; otherwise the endpoint
shows `Failed`.

Detail modal (`Enter` on a node): four-quadrant dashboard —
identity header (badge + status + roles + heartbeat age + node_id +
joined ts), Resources / Repositories top row (one row per repo with
`used / total` in power-of-1024 units), Events / GC bottom row.
When standalone, Events is hidden and GC fills the bottom row.

### TLS certificate expiry

`ClusterEndpoint::TlsCerts` probes each node's server cert chain
(one `tokio-rustls` handshake per node, permissive verifier, chain
captured from the verifier callback). Results join into
`NodeHealthRow.tls_cert`: `not_after` drives the full-chain render
in the node detail modal and a compact trailing chip on Nodes list
rows (earliest `not_after` as `Nd` or `Ny Mmo` beyond 1y).

Cadence via `[polling.cluster] tls_certs` (default `1h`).
Standalone NiFi probes `ctx.url`'s host+port; HTTP-only contexts
skip probing. `publish_node_addresses` force-wakes the fetcher when
the cluster roster changes.

Severity (hardcoded): expired or `<7d` red/bold; `7..30d` yellow;
`>=30d` muted grey.

### Version control drift

`version_control` fans out `GET /versions/process-groups/{id}` per
PG (using cached IDs from `root_pg_status`) and stores
`BTreeMap<PgId, VersionControlSummary>`. Default cadence `30s`.

Browser tree renders a trailing chip on PG rows whose state ≠
`UP_TO_DATE`: `[STALE]` / `[MODIFIED]` / `[STALE+MOD]` (warning),
`[SYNC-ERR]` (error). Single-chip alphabet — combined-state PGs
render `[STALE+MOD]`, not two chips.

`FlowIndex` entries for ProcessGroup kinds carry an
`Option<VersionControlInformationDtoState>` re-stamped on every
`ClusterChanged(VersionControl)`, so fuzzy-find drift filters
(`:drift`/`:stale`/`:modified`/`:syncerr`) work without a separate
index.

`m` on a versioned PG row opens the **version control modal** —
Identity panel (registry / bucket / branch / flow / version / state
/ explanation) + diff body sectioned by component. Diff data from a
one-shot worker fanning out `versions/process-groups/{id}` +
`process-groups/{id}/local-modifications` in parallel; identity
renders immediately, diff body shows `loading…`. Modal verb adds
`e` (toggle environmental, hidden by default).

### Parameter Contexts modal

`parameter_context_bindings` fans out
`processgroups().get_process_group(id)` per PG and stores
`BTreeMap<PgId, Option<ParameterContextRef>>`. Default cadence `30s`.
Subscriber-gated to Browser views.

`Enter` on the `Parameter context: <name> →` identity row (or `p`
on a PG row) opens the **parameter-context modal** — Identity
header + inheritance chain sidebar + resolved-flat parameters table
(`name | value | from | flags`). Flags: `[O]` override, `[S]`
sensitive, `[P]` provided, `[!]` unresolved. Modal verb adds `←/→`
(chain focus), `t` (toggle by-context view), `s` (show shadowed),
`u` (toggle Used-by panel).

`#{name}` parameter references in processor / CS property values
gain a trailing `→` when the owning PG has a bound context — Enter
opens the modal pre-selected (or synthesises `[!]` for unresolved).
The `##{...}` escape is honoured: `##{literal}` is *not* annotated.
Multi-ref values (`#{a}#{b}`) annotate but open without a preselect.

### Remote Process Groups

RPGs ride on the `RootPgStatus` recursive walk
(`remoteProcessGroupStatusSnapshots` produces `NodeKind::RemoteProcessGroup`
leaves under their parent PG). On-demand detail and sparkline history
go through the same view-local worker as other kinds.

Tree row glyph (`widget::run_icon::transmission_icon`): `▶` accent for
`TRANSMITTING`, `■` muted otherwise; body shows name + `→ target_uri`
chip. Identity pane lists name, parent PG (cross-link), target URI
(prefers plural `targetUris`, falls back to legacy `target_uri`),
target-secure flag, transport protocol, transmission/validation
status. Below the header: optional Validation errors sub-panel
(capped at `layout::VALIDATION_ERROR_ROWS_MAX`), then Input ports
and Output ports tables. Tab/Shift+Tab cycle the focusable sub-panels
(validation errors → input ports → output ports); `c` copies the
focused row. Port rows are not arena nodes — Enter does not descend.

Sparkline rows: `recv` (`receivedCount`), `sent` (`sentCount`),
`rate` (`totalBytesPerSecond`). NiFi does not expose `activeThreads`
for RPGs.

`connections_by_pg` reducer detects `REMOTE_INPUT_PORT` /
`REMOTE_OUTPUT_PORT` connectables and writes the parent RPG's
`group_id` (not the port UUID) into the connection's endpoints, so
`BrowserState::resolve_id` cross-links to the RPG arena entry.

Overview Components has a `Remote PGs` row sourced from
`RemoteProcessGroupCounts`. Fuzzy-find `:rpg` narrows the index to
RPG entries.

### Bulletins ring buffer & detail modal

Rolling in-memory window capped by `[bulletins] ring_size`
(default 5000, range 100..=100_000; ~1–2 MB at default). Fetcher polls
`flow_api().get_bulletin_board(after, limit=1000)` on `[polling.cluster]
bulletins` (default 5s), dedups via the monotonic `id` cursor, drops
from the front at capacity.

Reducer additionally dedupes by `(source_id, message_stem)`: strips
NiFi's `ComponentName[id=<uuid>]` prefix and normalises dynamic
`[...]` regions, so repeating errors collapse into one row with
`×N` count. `Shift+G` cycles grouping (`source+msg` / `source` / `off`);
`g` triggers `AppAction::Jump`.

`i` opens the **detail modal** (full raw message). `Enter` is
intentionally a no-op inside the modal. State lives on
`BulletinsState::detail_modal` (not an app-wide `Modal`);
`open_detail_modal` snapshots `GroupKey` + `GroupDetails` so subsequent
ring mutations don't disturb it.

### Action history modal

`a` on a Browser row whose component has a UUID opens the **action
history modal**: full-screen list of NiFi flow-config audit events
filtered by `sourceId`. Backed by a paginator over `/flow/history`
(worker calls `client.flow().query_history` directly so it can
surface `total` for auto-load gating; the `flow_actions_paginator`
helper in `client::history` wraps `pagination::flow_history_dynamic`
and is reused by the integration test). Rows paginate 100 at a time,
auto-load when viewport bottom is within 10 rows of loaded tail.

State on `BrowserState::action_history_modal`, with a separate
`selected: usize` cursor (the `VerticalScrollState` holds only
viewport offset). Worker (`spawn_action_history_modal_fetch`)
eagerly fetches the first page then sleeps on a
`tokio::sync::Notify` until the reducer wakes it for the next.

Modal verb adds `Enter` (expand selected) and refines `Esc` to
cascade through search → expanded → close. `c` copies the selected
row as TSV.

### Sparkline strip

The Browser detail identity panel for processor / PG / connection /
RPG rows includes a 3-line inline sparkline on the right half. Backed
by `src/client/history.rs::status_history`, which dispatches to the
generated `get_*_status_history` functions and reduces to a
metric-keyed `StatusHistorySeries`.

State on `BrowserState::sparkline` + a `JoinHandle<()>`, re-created
on every selection change to a supported kind via
`AppState::refresh_sparkline_for_selection`. Selection changes to
CS / Port / Folder tear down both. Worker loops on
`config.polling.cluster.status_history` (default `30s`); 404 maps to
`SparklineEndpointMissing` (sticky per selection); other errors
`warn!` and continue. Reducer arms apply each emit only when
`(kind, id)` matches the active selection (defends against stale
emits between worker abort and exit).

`reduce_status_history` reads `aggregateSnapshots` first, then falls
back to summing `nodeSnapshots[*].statusSnapshots` across nodes per
timestamp (NiFi clustered mode often returns an empty aggregate).

Three rows per kind: processor — in / out / task time; PG — in / out
/ queue count; connection — in / out / queue count; RPG — recv / sent
/ rate (`totalBytesPerSecond`).

Layout is **content-driven**: renderer measures identity-line width,
places lines flush left, leaves a 2-cell gap (`SPARKLINE_GAP_COLS`),
gives the remainder to the strip; suppressed entirely when remainder
< `SPARKLINE_MIN_RIGHT_HALF_WIDTH` (12 cells). No focus, no chord.

### Tracer content viewer modal

Full-screen modal opened with `i` on the Tracer Content sub-tab.
State on `AppState.tracer.content_modal`. While open the modal's
`Verb::all()` drives the footer hint strip and the keymap shadows
outer-tab keys.

Streaming via `provenance_content_range(event_id, side, offset, len)`
in 512 KiB chunks; reducer auto-fires the next chunk when the
viewport bottom comes within 100 lines of the decoded tail. Per-side
ceilings live under `[tracer.ceiling]` (keys: `text`, `tabular`,
`diff`; defaults `4 MiB` / `64 MiB` / `16 MiB`; `"0"` → unbounded).

Per-chunk classification uses `classify_text_or_hex_no_pretty` (UTF-8
check + hex fallback, no JSON parse) so chunk arrivals don't block
the UI thread. JSON pretty-print runs **once** off-thread when the
side is fully loaded, dispatched the same way as tabular decode
(reducer emits `PendingIntent::PrettyPrintJson` → `spawn_blocking` →
`TracerPayload::JsonPrettyPrinted`). `pretty_print_json` uses
`serde_transcode` to stream `Deserializer` → `Serializer::pretty`,
avoiding the `serde_json::Value` round-trip — **object key order is
preserved**. Tabular and JSON-pretty are mutually exclusive: tabular
detected by magic bytes, JSON by `looks_like_json` (`{` or `[` first
non-whitespace byte).

Diff mode bounded by `[tracer.ceiling] diff`, using
`similar::TextDiff::from_lines` with 3-line context. Eligibility:
both sides available, MIME pair in the allowlist (or UTF-8 fallback
when neither declares MIME), per-side size ≤ diff ceiling,
non-identical bytes. `Ctrl+↓` / `Ctrl+↑` navigate changes; hunk
header `@@ input Lx · output Ly @@` appends `· N changes`.

Search primitives (`MatchSpan`, `SearchState`, `compute_matches`)
are shared with the Bulletins detail modal via `src/widget/search.rs`.

### Tracer tabular content (Parquet & Avro)

`ContentRender::Tabular { format, schema_summary, body, decoded_bytes,
truncated }` is produced when `classify_content` sees `PAR1` or
`Obj\x01` magic. Decoders live in `src/client/tracer/content.rs`
(Parquet via `ParquetRecordBatchReaderBuilder` +
`arrow::json::LineDelimitedWriter`; Avro via `apache_avro::Reader`).
Decoder errors are caught inside `classify_content`, logged at
`warn!`, and surfaced as `Hex` — the classifier signature stays
infallible.

**Per-side ceiling** resolves after the first chunk arrives (reducer
sniffs magic, records on `SideBuffer.effective_ceiling`). Parquet's
footer lives at EOF, so a ceiling-hit fetch falls back to `Hex` with
a chip; Avro is streamable and degrades via `truncated = true`.

**Diff:** tabular sides diff iff their `format` tags match. Diff
input is `Tabular::body`; schema lines do not contribute hunks; the
`diff` ceiling caps per-side input.

### Poll intervals

Periodic NiFi fetches are owned by `ClusterStore`. Base cadences come
from `[polling.cluster]` in `config.toml` (one key per endpoint
listed under "Central cluster store", plus adaptive knobs
`max_interval` and `jitter_percent`). `status_history` is
selection-scoped (cadences the sparkline worker), not a
`ClusterStore` fetcher. Values use humantime format (`"10s"`,
`"750ms"`, `"2m"`); out-of-range values emit `tracing::warn!` but
are accepted.

Events in-flight query polling (750 ms) and Tracer content
in-flight polling (500 ms) are hardcoded — internal mechanics.

### Visual language

A project-wide bordered-box visual language goes through
`widget::panel::Panel`. Focused panels flip to `BorderType::Thick` +
accent color; unfocused use plain borders + `theme::border_dim()`.
New interactive sub-panels use `↑`/`↓` for row nav — `j`/`k` aliases
are not used app-wide.

Severity rendering (labels, colors, icons) is consolidated in
`widget::severity` and `widget::run_icon`; call these helpers rather
than inline `Color::*`/`Modifier::*` constructors.

Shared helpers:

- `src/widget/modal.rs` — `MIN_WIDTH` / `MIN_HEIGHT`,
  `render_too_small()`, `render_verb_hint_strip<V: Verb>()`. Used by
  every full-screen modal.
- `src/widget/scroll.rs` — `VerticalScrollState` /
  `BidirectionalScrollState` (scroll, page, jump, horizontal math).
- `src/widget/filter_bar.rs` — `FilterChip` + `build_chip_line` for
  horizontal chip rows (Events + Bulletins top rows).
- `src/widget/search.rs` — `SearchState` + `compute_matches`, shared
  by every search-capable modal.
- `src/layout.rs` — `split_header_body_footer` / `split_two_rows` /
  `split_two_cols`.
- `src/bytes.rs` — `KIB` / `MIB` / `GIB` + `format_bytes` /
  `format_bytes_int`. Prefer over raw `N * 1024 * 1024`.
- `src/client/status.rs` — `ProcessorStatus` /
  `ControllerServiceState` typed enums; use `from_wire`, `style`,
  `icon`, etc. rather than matching raw strings.
- `src/timestamp.rs` — `format_age` / `format_age_secs`.
- `src/test_support.rs` — `fresh_state`, `tiny_config`,
  `default_fetch_duration`, `test_backend(height)` and the
  `TEST_BACKEND_*` width constants.

Folders in the Browser tree are a **reducer-only** construct. The
client walker emits a flat list of CS / queue / port / processor
nodes; `apply_tree_snapshot` post-processes each PG's children to
synthesize `Folder(Queues)` / `Folder(ControllerServices)` arena
rows and re-parent the leaves. Folders are never cross-link targets,
never emit detail-fetch requests, and are filtered out of the
fuzzy-find flow index.

### Browser cross-navigation

Any row in a Browser detail sub-panel whose value resolves to a node
in the arena is annotated with a trailing `→`. Pressing Enter
emits `CrossLink::OpenInBrowser`, reusing the reducer arm that
already handles Bulletins → Browser and CS Referencing → Browser
jumps. Resolution goes through `BrowserState::resolve_id`, which
gates on a canonical-UUID shape check before scanning `state.nodes`
(linear scan, once per annotatable row).

Annotated surfaces: connection endpoints (FROM/TO), processor/CS
property values that are UUIDs, processor connection rows (→
opposite endpoint), PG-owned CSes. CS & Port Identity panels
resolve `parent` UUIDs to the owning PG's name display-only.

Selected-relationships on connections are intentionally not surfaced
in the processor Connections section: that data lives on
`ConnectionDTO` (fetched by `browser_connection_detail`), not on the
status snapshot the tree walker reads.

Connection endpoint IDs are NOT populated by the recursive status
endpoint — NiFi leaves `source_id` / `destination_id` null on
`ConnectionStatusSnapshotDto`. The `connections_by_pg` fetcher
backfills them via parallel per-PG `/process-groups/{id}/connections`
calls; per-PG failures are logged and skipped.

### Queue listing panel

Connection detail panes render a flowfile listing in the lower half
when the connection has flowfiles queued. Backed by
`src/client/queues.rs` which wraps NiFi's two-phase listing-request
flow (`POST /flowfile-queues/{id}/listing-requests` → poll
`GET /listing-requests/{request_id}` until `finished` → `DELETE`).

State on `BrowserState::queue_listing`, re-spawned on every
selection change to a Connection with `flow_files_queued > 0`.
`QueueListingHandle::drop` fires-and-forgets `DELETE` against the
recorded request id to free server resources on any navigation; NiFi's
listing-request TTL is the safety net if Drop misses.

NiFi caps the listing at 100 rows server-side; `total > 100` shows
a `[100 / N]` truncation chip. Modal verbs (`BrowserQueueVerb`,
`BrowserPeekVerb`) shadow outer-tab keys. Polling cadence is 500 ms
(not user-configurable); `[browser] queue_listing_timeout` (30s) and
`queue_listing_age_warning` (5m, `0s` disables) are configurable.

### Fuzzy Find

The `Shift+F` modal searches a shared `FlowIndex` built from the
Browser arena (processors, PGs, CSes, connections, ports — folders
excluded). Haystack per entry: `"{name} {kind_label} {group_path}"`
lowercased; nucleo scores, top 50 shown.

A leading colon-prefixed token narrows the corpus before fuzzy
scoring via a `QueryFilter` enum:

- Kind aliases: `:proc`, `:pg`, `:cs`, `:conn`, `:in`, `:out`, `:rpg`.
- Drift aliases (PG-scoped): `:drift`, `:stale`, `:modified`,
  `:syncerr`.

Parsing happens inside `FuzzyFindState::rebuild_matches`; an unknown
`:token` (or any non-leading occurrence) is treated as plain query
text. A read-only chip row above the query line reflects the parsed
filter. There is no chip-toggle keybinding — the query string is
the single source of truth.

## Dependency on `nifi-rust-client`

`nifi-lens` depends on `nifi-rust-client` with the `dynamic` feature:

```toml
nifi-rust-client = { version = "…", features = ["dynamic"] }
```

At the bottom of `Cargo.toml` there is a **commented-out**
`[patch.crates-io]` block:

```toml
# [patch.crates-io]
# nifi-rust-client = { path = "../nifi-rust-client/crates/nifi-rust-client" }
```

**Local development workflow:** uncomment locally to iterate against
an unreleased change in the sibling worktree; recomment before
pushing. A forgotten uncomment will break CI on the first cargo job
(the sibling path doesn't exist on runners). That is the intended
guardrail — do not try to teach CI to tolerate it.

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

**MSRV is `1.88`.** `rust-toolchain.toml` pins `1.93.0` for
development; CI enforces the `1.88` floor via `RUSTUP_TOOLCHAIN`
override. MSRV was raised from 1.85 to 1.88 to pull in `time >=
0.3.47`, which fixes
[RUSTSEC-2026-0009](https://rustsec.org/advisories/RUSTSEC-2026-0009).

Rustdoc discipline: doc comments on `pub` items must not `[`link`]`
to private items. CI fails the doc build on private-link warnings;
use plain backticks.

### Pointing the binary at a local NiFi cluster

Create a config file in the platform config dir (see README.md
"Configuration", or run `nifilens config init`), export the
referenced `password_env`, and run `cargo run -- --context dev`.

### Integration test fixture

The harness at `integration-tests/` brings up a standalone NiFi 2.6.0
(floor) and a 2-node NiFi 2.9.0 cluster (ceiling) with ZooKeeper,
seeds each via `nifilens-fixture-seeder`, and runs `cargo test
--test 'integration_*' -- --ignored` against both:

```bash
./integration-tests/run.sh
```

`run.sh` invokes `scripts/download-nars.sh` first to fetch
`nifi-parquet-nar` (and its transitive `nifi-hadoop-libraries-nar`)
from Maven Central into a gitignored cache. NARs mount per-version
into each NiFi service's `/opt/nifi/nifi-current/nar_extensions/`
— the `apache/nifi` base images don't bundle the standalone Parquet
writer (only Iceberg), so the mount is required for
`diff-parquet-writer`.

Live-dev workflow (fixture stays up, point the TUI at it): `docker
compose -f integration-tests/docker-compose.yml up -d`, run the
seeder with `--skip-if-seeded`, then `cargo run -- --config
integration-tests/nifilens-config.toml --context dev-nifi-2-9-0`.
`--skip-if-seeded` makes re-runs a no-op when the fixture marker PG
(`nifilens-fixture-v8`) is already present.

**Fixture shape** — top-level marker PG `nifilens-fixture-v8` holds
6 child PGs (`orders-pipeline/` centerpiece + `remote-targets/`
sibling + four standalones for hard-to-reach states:
`invalid-pipeline`, `backpressure-pipeline`, `versioned-clean`,
`versioned-modified`), 5 parameter contexts in a 3-tier inheritance
chain (`fixture-pc-platform` → `fixture-pc-orders` →
`fixture-pc-region-{eu,us,apac}`), and 4 root-level CSes (two ENABLED
JSON reader/writer used by `transform/`, two DISABLED/INVALID kept
as modal-rendering exhibits). See `integration-tests/seeder/src/fixture/`
for the authoritative layout.

**Headline narrative**: `transform/UpdateRecord-fx-rate` reads
`#{usd_rate}` from `fixture-pc-orders`; the seeder mutates that
parameter to `"oops"` after topology start (controlled by
`--break-after`), routing every flowfile to `deadletter` and
producing the audit / bulletin / queue evidence the demo investigates.
Default `--break-after 0s` for CI; longer (`5m`, `30m`) for live
demos. Mutation is value-gated and idempotent.

**Diff coverage**: JSON↔JSON (`UpdateRecord-cancel-old`,
`mark-deleted`), JSON↔CSV grayed-out (`ConvertRecord-csv2json`),
Parquet↔Parquet (`sink-us/UpdateRecord-parquet-tag`), Avro↔Avro
(`sink-apac/UpdateRecord-avro-tag`). NiFi often reports
`inputContentClaim == outputContentClaim` on `CONTENT_MODIFIED` even
when bytes differ — always fetch both sides.

Some NiFi processor *property keys* drift between minor versions
even when display names are stable; setting a property by display
name when the real key differs silently turns it into a dynamic
attribute. The seeder handles known cases via
`fixture::custom_text_property_key(version)` and similar helpers.

### Bumping the NiFi ceiling version

When `nifi-rust-client` adds support for a new NiFi version:

1. Update `nifi-rust-client` in the root `Cargo.toml`.
2. Edit `integration-tests/versions.toml`.
3. Edit `integration-tests/docker-compose.yml`.
4. Edit `integration-tests/nifilens-config.toml`.
5. Edit `tests/common/versions.rs` `port_for` match arm.
6. Run `./integration-tests/run.sh` locally to verify.
7. Push. CI's drift check enforces steps 2–4 consistency.

The **floor version 2.6.0 never drops** — it stays pinned forever
so the dynamic client is always tested against the oldest supported
NiFi.

## Release

Releases are driven by
[`cargo-release`](https://crates.io/crates/cargo-release) via
`release/release.sh`, a thin passthrough wrapper. **`cargo-release` is
dry-run by default**; pass `--execute` to perform the release.

```bash
release/release.sh patch              # dry-run
release/release.sh patch --execute    # bump version, rewrite CHANGELOG, commit, tag, push
```

The release commit updates `Cargo.toml` `version`, `Cargo.lock`, and
`CHANGELOG.md` (`## [Unreleased]` → `## [X.Y.Z] — YYYY-MM-DD`, fresh
`## [Unreleased]` inserted, compare link rewritten).

Two workflows fire on every `v*.*.*` tag:

1. `publish-crate.yml` — verifies tag matches `Cargo.toml`, runs the
   full check suite, `cargo publish`es to crates.io using
   `CARGO_REGISTRY_TOKEN`.
2. `release.yml` — autogenerated by cargo-dist. Builds per-target
   archives (Linux x86_64/aarch64 gnu+musl, macOS x86_64/aarch64,
   Windows x86_64); uploads them plus shell / PowerShell installers
   and a Homebrew formula to a GitHub Release; writes notes from the
   `## [X.Y.Z]` CHANGELOG section.

The local machine never publishes; `cargo-release` is configured with
`publish = false` so `CARGO_REGISTRY_TOKEN` lives only in GitHub.

### cargo-dist configuration

Configured in `dist-workspace.toml` and `Cargo.toml`'s `[profile.dist]`.
**Never hand-edit `release.yml`** — it is regenerated. To change
targets / installers / cargo-dist version: edit `dist-workspace.toml`,
run `dist generate` (cargo-dist ≥ 0.28; the binary is `dist`), commit
the regenerated workflow alongside the config.

Homebrew tap (not yet configured): create `maltesander/homebrew-tap`,
add `tap` / `formula` keys to `dist-workspace.toml`'s `[dist]` table,
regenerate, and add a `HOMEBREW_TAP_TOKEN` repo secret with
`contents: write` on the tap repo.

Install once: `cargo install cargo-release --locked && cargo install
cargo-dist --locked` (the cargo-dist binary is named `dist`).

## Documentation Policy

| Audience | Location | Format |
|---|---|---|
| Users | `README.md` | Rendered on GitHub + crates.io |
| Contributors | `AGENTS.md` (this file) | Markdown |
| Version history | `CHANGELOG.md` | Keep a Changelog — rewritten by `cargo-release` |
| API rustdoc | Inline `///` | `cargo doc --no-deps` warning-free |
| Specs and plans | `docs/` locally | Markdown (gitignored) |

**Rules:**

- `docs/` is gitignored. Do **not** hard-link into it from any
  committed file.
- When architecture / patterns change, update `AGENTS.md` in the
  same commit.
- When user-visible behavior changes, update `README.md` and
  `CHANGELOG.md` in the same commit.
- Every new dep goes into `Cargo.toml` in its correct alphabetical
  position.

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
