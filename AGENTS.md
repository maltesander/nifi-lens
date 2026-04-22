# AGENTS

## Project Overview

`nifi-lens` is a keyboard-driven terminal UI for observing and debugging
Apache NiFi 2.x clusters. It is powered by
[`nifi-rust-client`](https://docs.rs/nifi-rust-client) used exclusively via
the `dynamic` feature, so one binary works against every supported NiFi
version in the fleet. v0.1 is read-only, multi-cluster (kubeconfig-style
context switching), and forensics-focused — it is explicitly a *lens*, not
a canvas replacement.

Top-level tabs (in order): **Overview**, **Bulletins**, **Browser**,
**Events**, **Tracer**.

## Repository Layout

```text
nifi-lens/
├── Cargo.toml                # binary crate; publishable metadata
├── Cargo.lock                # committed
├── rust-toolchain.toml       # dev toolchain pin (1.93.0)
├── rustfmt.toml              # fmt config
├── clippy.toml               # clippy config
├── deny.toml                 # cargo-deny config
├── release.toml              # cargo-release config
├── .pre-commit-config.yaml
├── .markdownlint.yaml
├── CHANGELOG.md              # Keep a Changelog
├── README.md                 # user-facing
├── AGENTS.md                 # this file
├── CLAUDE.md                 # agent rules
├── LICENSE                   # Apache-2.0
├── .github/workflows/
│   ├── ci.yml                # fmt, clippy, test, doc, msrv, deny, pre-commit
│   └── release.yml           # tag-triggered publish + GitHub Release
├── release/
│   └── release.sh            # cargo-release wrapper, dry-run by default
├── integration-tests/        # Docker-backed live-cluster fixture
└── src/
    ├── lib.rs                # public entry: pub fn run() -> ExitCode
    ├── main.rs               # thin wrapper: std::process::exit(nifi_lens::run())
    ├── cli.rs                # clap derive: Args, Command, ConfigAction, LogLevel
    ├── error.rs              # NifiLensError (snafu, full variant set)
    ├── logging.rs            # tracing-subscriber + rotating file + StderrToggle
    ├── theme.rs              # color / style constants
    ├── timestamp.rs          # TimestampFormat / TimestampTz parsing and formatting
    ├── event.rs              # AppEvent, IntentOutcome, ViewPayload
    ├── layout.rs              # shared ratatui layout helpers
    ├── test_support.rs       # fresh_state / tiny_config helpers for widget tests
    ├── config/               # schema, loader, init
    ├── client/               # NifiClient wrapper (Deref) + TLS + events
    ├── cluster/              # ClusterStore + fetcher tasks + snapshot + subscriber
    ├── input/                # KeyMap + typed action enums (FocusAction, Verb, …)
    ├── app/                  # run loop, per-view state reducers, ui, navigation, worker
    ├── intent/               # Intent enum + IntentDispatcher
    ├── view/                 # per-tab views (overview, bulletins, browser, events, tracer)
    └── widget/               # status_bar, help_modal, context_switcher, panel, severity, …
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

`src/cluster/ClusterStore` owns seven endpoint fetchers:
`root_pg_status`, `controller_services`, `controller_status`,
`system_diagnostics`, `bulletins`, `connections_by_pg`, `about`. Each
runs as an independent `tokio::task::spawn_local` future on the main-
thread `LocalSet`, pushes `AppEvent::ClusterUpdate` on success, and
sleeps for its (adaptive, jittered) base cadence before the next tick.

Snapshot mutation is main-loop-only: the `ClusterUpdate` arm in
`src/app/mod.rs` calls `state.cluster.apply_update(...)` and re-emits
`AppEvent::ClusterChanged(endpoint)`. Views observe the change by
matching on the endpoint and invoking their `redraw_*` reducers.

Three endpoints are **subscriber-gated** — they park when no view is
subscribed: `root_pg_status`, `controller_services`,
`connections_by_pg`. `WorkerRegistry::ensure` calls
`cluster.subscribe(endpoint, view)` on tab entry and
`unsubscribe(endpoint, view)` on tab exit.

Context switch: `cluster.shutdown()` aborts every fetcher and the
store is rebuilt with the new `NifiClient` in the main loop's
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

### Intent pipeline

All user actions route through a single `Intent` enum and a dispatcher.
Write variants exist in the enum from day one so a later write-capable
build does not require restructuring, but no key binding constructs them
in v0.1 and the dispatcher refuses to execute them without an
`--allow-writes` CLI flag (which v0.1 does not expose).

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

`F12` dumps the keymap reverse table (every registered chord and its
enum source) to the log file. Unadvertised in the help modal; use it
when debugging "why doesn't key X do anything".

Verbs whose shortcut is already visible adjacent to the status-bar hint
strip can opt out of the strip via `Verb::show_in_hint_bar() -> false`
while still appearing in `?` help and being dispatched by the keymap.
Used by the Bulletins severity filters (`1`/`2`/`3`), which are
surfaced by the `[E n] [W n] [I n]` chips, and by
`BulletinsVerb::SearchNext` / `SearchPrev` which only make sense inside
the detail modal.

### Adding a new view

1. Create `src/view/<name>/` with `mod.rs`, `state.rs`, and rendering
   (either a single `render.rs` or a `render/` submodule for larger
   views — Browser uses the latter). Add `worker.rs` only if the view
   needs on-demand detail fetches (Browser, Events, Tracer do;
   Overview and Bulletins don't — they consume the shared cluster
   snapshot). Mirror an existing view like Events for the small case
   or Browser for the render-submodule case.
2. Add a new `ViewId::<Name>` variant. Update `ViewId::next()` and
   `ViewId::prev()` cycle arms.
3. Create `src/app/state/<name>.rs` with a `<Name>Handler` zero-sized
   type implementing `ViewKeyHandler`.
4. Add one arm to the `dispatch_handler!` macro in
   `src/app/state/mod.rs`.
5. If the view needs live cluster data, subscribe to the relevant
   `ClusterEndpoint`s in `WorkerRegistry::ensure`'s arm for this view,
   and unsubscribe on tab exit (see the existing Overview / Browser /
   Bulletins arms in `src/app/worker.rs`). For on-demand detail
   fetches, spawn a view-local worker as before.
6. Add a render arm to `src/app/ui.rs`'s render dispatch.
7. Add a top-bar label (`src/widget/top_bar.rs`).

All seven steps are mechanical. `dispatch_handler!` means
cross-cutting key-handling dispatch is a single touch, not five.

### Logging

`tracing` + `tracing-subscriber` + `tracing-appender` write to
`~/.local/state/nifilens/nifilens.log` (rotating, 5 files × 10 MB).
Default level `info`; `--debug` raises to `debug`. **Never** writes to
stdout or stderr while the TUI is active.

### Overview Components panel

The top panel of the Overview tab is a 3-row aligned table titled
`Components`: **Process groups** / **Processors** / **Controller
services**. Each row carries a total count plus per-type detail
(per-state counts for processors and CSes; version-sync drift +
input/output port counts for PGs). The PG row collapses its
versioning slot to `all in sync` when no PG is stale, locally
modified, or sync-failed; otherwise it expands to three numeric
slots. Display-only — no focus, no selection.

**Data sources:** processor / PG / port counts are derived from the
shared `root_pg_status` snapshot in `ClusterStore`. Version-sync
counts come from `controller_status` (also in the snapshot).
Controller-service counts come from a dedicated
`ClusterEndpoint::ControllerServices` fetcher that runs
`get_controller_services_from_group("root", false, true, false, None)`
on its own cadence. That fetch is **non-fatal**: failure degrades the
CS row to a `cs list unavailable` chip while the other rows still
render.

### Bulletins ring buffer

The Bulletins tab holds a rolling in-memory window of recently-seen
bulletins. The cap is controlled by `[bulletins] ring_size` in
`config.toml` (default 5000, valid range 100..=100_000). Memory budget
at the default is ~1–2 MB. The cluster store's bulletins fetcher polls
`flow_api().get_bulletin_board(after, limit=1000)` on the
`[polling.cluster] bulletins` cadence (default 5s), dedups via the
monotonic `id` cursor, and drops from the front when the ring exceeds
its capacity. Bulletins views subscribe to
`ClusterEndpoint::Bulletins` and mirror the shared ring into their own
state via `redraw_bulletins`.

Rows in the list are additionally deduplicated by
`(source_id, message_stem)` — the reducer strips NiFi's
`ComponentName[id=<uuid>]` prefix and normalizes dynamic `[...]` regions
before hashing, so repeating errors from the same component collapse
into a single row with an `×N` count column. Grouping mode is cycled by
`Shift+G` (`source+msg` / `source` / `off`). `g` triggers
`AppAction::Jump` — a context-sensitive cross-tab jump menu.

### Bulletins detail modal

`i` on a selected bulletin opens a full-screen detail modal showing the
full raw message with scroll (`↑↓`/`PgUp`/`PgDn`/`Home`/`End`),
plain-substring `/`-search with `n`/`N` cycling, `c` to copy, and
`Enter` to jump to the source in Browser. `Esc` closes. The modal lives
as `BulletinsState::detail_modal` (not an app-wide `Modal`) because it
carries per-view semantics. `open_detail_modal` captures a `GroupKey`
and `GroupDetails` snapshot at open time; subsequent ring mutations do
not disturb the open modal.

### Tracer content viewer modal

Full-screen modal opened with `i` on the Tracer Content sub-tab.
State lives on `AppState.tracer.content_modal: Option<ContentModalState>`.
Modal-scoped keys use `ContentModalVerb` (a separate enum from
`TracerVerb`); when the modal is open, the modal's `Verb::all()`
generator drives the footer hint strip and help modal section, and
the keymap shadows outer-tab keys.

Streaming: `provenance_content_range(event_id, side, offset, len)`
fetches 512 KiB chunks. A reducer auto-fires the next chunk when the
viewport bottom comes within 100 lines of the decoded tail. Per-side
ceiling defaults to 4 MiB (`[tracer] modal_streaming_ceiling`);
`"0"` → unbounded.

Diff mode is bounded at 512 KiB per side (fixed, not configurable)
and uses `similar::TextDiff::from_lines` with 3-line context. Diff
eligibility requires both sides available, MIME pair matching the
allowlist (or UTF-8 fallback when neither side declares a MIME),
declared size ≤ 512 KiB per side, and non-identical bytes.

Search primitives (`MatchSpan`, `SearchState`, `compute_matches`) are
shared with the Bulletins detail modal via `src/widget/search.rs`.

Parquet / tabular viewers are **out of scope**; tracked as a follow-up
spec.

### Poll intervals

All periodic NiFi fetches are owned by `src/cluster/ClusterStore`.
Base cadences come from `[polling.cluster]` in `config.toml` (keys:
`root_pg_status`, `controller_services`, `controller_status`,
`system_diagnostics`, `bulletins`, `connections_by_pg`, `about`, plus
the adaptive knobs `max_interval` and `jitter_percent`). The fetcher
scales intervals adaptively (up to `max_interval`) based on measured
latency, adds ±`jitter_percent/100` jitter, and parks expensive
endpoints (`root_pg_status`, `controller_services`,
`connections_by_pg`) when no view is subscribed.

Values use the humantime format (`"10s"`, `"750ms"`, `"2m"`). The
loader emits a `tracing::warn!` (into the rotating log file) for
values outside the recommended range but accepts them as-is — no
hard rejection.

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

Folders in the Browser tree are a **reducer-only** construct. The
client walker emits a flat list of CS / queue / port / processor
nodes; `apply_tree_snapshot` post-processes each PG's children to
synthesize `Folder(Queues)` / `Folder(ControllerServices)` arena
rows and re-parent the leaves. Folders are never cross-link targets,
never emit detail-fetch requests, and are filtered out of the fuzzy-
find flow index.

### Browser cross-navigation

Any row rendered in a Browser detail sub-panel whose value resolves to
a node in the arena is annotated with a trailing `→`. Pressing Enter
(Descend) on such a row emits `CrossLink::OpenInBrowser`, reusing the
reducer arm that already handles Bulletins → Browser and CS
Referencing → Browser jumps. Resolution goes through
`BrowserState::resolve_id`, which gates on a canonical-UUID shape
check before scanning `state.nodes` — a linear scan the renderer
performs once per annotatable row.

Jumpable surfaces:

- Connection detail: `Endpoints` section (FROM/TO rows → source /
  destination component).
- Processor / Controller Service detail: `Properties` rows whose value
  is a UUID pointing to a known arena node (typically CS references).
- Processor detail: the `Connections` section (→ opposite endpoint).
- Process Group detail: the `Controller services` section (→ owned
  CS). The CS is always a child of the current PG in the arena, so
  the intent uses the PG's id as `group_id`.
- Controller Service & Port Identity panels resolve `parent` /
  `parent group` UUIDs to the owning PG's name (display only — parent
  is always reachable via Left/Ascend).

Selected-relationships on connections are intentionally not surfaced
in the processor Connections section: that data lives on
`ConnectionDTO` (fetched by `browser_connection_detail`), not on the
status snapshot the tree walker reads. Surfacing them would cost a
second fetch per connection.

Connection endpoint IDs (`source_id` / `destination_id` on
`NodeStatusSummary::Connection`) are NOT populated by the recursive
status endpoint — NiFi leaves those fields null on
`ConnectionStatusSnapshotDto`. `browser_tree` therefore fires a
parallel `/process-groups/{pg_id}/connections` fetch per PG after
the status walk, builds a `connection_id → (source_id,
destination_id)` map, and backfills the arena's Connection rows.
Per-PG fetch failures are logged and skipped — affected connections
simply render without the `→` marker on their endpoints. The extra
fetches are cheap (one per PG, parallelized with
`futures::future::join_all`).

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

**MSRV is `1.88`.** The `rust-toolchain.toml` pins `1.93.0` for
development; CI enforces the `1.88` floor via `RUSTUP_TOOLCHAIN`
override. MSRV was raised from 1.85 to 1.88 to pull in
`time >= 0.3.47`, which fixes
[RUSTSEC-2026-0009](https://rustsec.org/advisories/RUSTSEC-2026-0009).

Rustdoc discipline: doc comments on `pub` items must not `[`link`]` to
private items. CI fails the doc build on private-link warnings; use
plain backticks instead.

### Pointing the binary at a local NiFi cluster

Create `~/.config/nifilens/config.toml` as shown in `README.md`, export
the referenced `password_env` variable, and run
`cargo run -- --context dev`.

### Integration test fixture

The integration harness at `integration-tests/` brings up a standalone
NiFi 2.6.0 (floor) and a 2-node NiFi 2.9.0 cluster (ceiling) with
ZooKeeper, seeds each with a rich fixture via `nifilens-fixture-seeder`,
and runs `cargo test --test 'integration_*' -- --ignored` against both.
One command does it all:

```bash
./integration-tests/run.sh
```

**Live-dev workflow** — the fixture stays up, point the TUI at it, and
iterate without re-seeding:

```bash
docker compose -f integration-tests/docker-compose.yml up -d
export NIFILENS_IT_PASSWORD=adminpassword123
cargo run -p nifilens-fixture-seeder -- \
    --config integration-tests/nifilens-config.toml \
    --context dev-nifi-2-6-0 --skip-if-seeded
cargo run -p nifilens-fixture-seeder -- \
    --config integration-tests/nifilens-config.toml \
    --context dev-nifi-2-9-0 --skip-if-seeded
cargo run -- --config integration-tests/nifilens-config.toml \
    --context dev-nifi-2-9-0
```

`--skip-if-seeded` makes re-runs of the seeder a no-op when the fixture
marker PG (`nifilens-fixture-v2`) is already present, so iterating on
`nifi-lens` itself doesn't reset the fixture state.

The fixture is five process groups (`healthy-pipeline` with nested
`ingest` / `enrich` children, `noisy-pipeline`, `backpressure-pipeline`,
`invalid-pipeline`, and `bulky-pipeline`) plus four controller services
(`fixture-json-reader` ENABLED, `fixture-json-writer` ENABLED,
`fixture-csv-reader` DISABLED, `fixture-broken-writer` INVALID/DISABLED),
all under a top-level marker PG named `nifilens-fixture-v2`.
`bulky-pipeline` produces ~1.5 MiB flowfiles at a low rate, providing
content for Tracer truncation testing. The `healthy-pipeline/enrich`
processor chain starts with a `ConvertRecord` processor that references
`fixture-json-reader` and `fixture-json-writer`, followed by
`UpdateAttribute-enrich`, `UpdateAttribute-cleanup`, and `LogAttribute-INFO`,
exercising CS-referencing coverage on all NiFi versions including the 2.6.0
floor. When the detected NiFi version is >= 2.9.0, the seeder also creates
`stress-pipeline` — a longer branching flow with ConvertRecord (JSON to
CSV), UpdateRecord, RouteOnAttribute (hot/normal split), and dual ControlRate
bottlenecks for sustained queue backpressure, plus four additional controller
services. Bumping the marker name invalidates stale fixtures automatically on
the next seed pass.

### Bumping the NiFi ceiling version

When `nifi-rust-client` adds support for a new NiFi version:

1. Update `nifi-rust-client` in the root `Cargo.toml`.
2. Edit `integration-tests/versions.toml` — replace or append the new
   version.
3. Edit `integration-tests/docker-compose.yml` — add/replace the service
   block.
4. Edit `integration-tests/nifilens-config.toml` — add/replace the
   context.
5. Edit `tests/common/versions.rs` — add/replace the `port_for` match
   arm.
6. Run `./integration-tests/run.sh` locally to verify.
7. Push. CI's drift check enforces steps 2–4 consistency.

The **floor version 2.6.0 never drops** — it stays pinned forever so
the dynamic client is always tested against the oldest supported NiFi.

## Release

Releases are driven by
[`cargo-release`](https://crates.io/crates/cargo-release) via
`release/release.sh`, which is a thin passthrough wrapper.

**`cargo-release` is dry-run by default.** Adding `--execute` performs
the release.

| Command | Effect |
|---|---|
| `release/release.sh patch` | Dry-run a patch release. Prints the plan, touches nothing. |
| `release/release.sh minor` | Dry-run a minor release. |
| `release/release.sh major` | Dry-run a major release. |
| `release/release.sh patch --execute` | Bump version, rewrite `CHANGELOG.md`, commit, tag, push. |

**What the release commit updates:**

- `Cargo.toml` `version`
- `Cargo.lock` (cascades automatically)
- `CHANGELOG.md`: `## [Unreleased]` becomes `## [X.Y.Z] — YYYY-MM-DD`; a
  fresh empty `## [Unreleased]` stanza is inserted above it; the compare
  link at the bottom is rewritten.

**What happens after the tag is pushed:**

`.github/workflows/release.yml` triggers on `v*.*.*` tags and:

1. Verifies the tag matches the `Cargo.toml` version.
2. Runs the full check suite (`fmt`, `clippy`, `test`, `doc`, `deny`).
3. `cargo publish`es to crates.io using the `CARGO_REGISTRY_TOKEN`
   secret.
4. Creates a GitHub Release with release notes extracted from the
   `## [X.Y.Z]` section of `CHANGELOG.md`.

The local machine never publishes. `cargo-release` is configured with
`publish = false` so the `CARGO_REGISTRY_TOKEN` secret can live only in
GitHub.

### Installing `cargo-release`

```bash
cargo install cargo-release --locked
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
