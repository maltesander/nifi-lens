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

User-facing behaviour, configuration, keybindings, install, and the
fixture walkthrough live in `README.md`. This file is the
contributor-facing companion and intentionally avoids duplicating that
content.

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
    ├── bytes.rs / theme.rs / timestamp.rs / event.rs / layout.rs / test_support.rs
    ├── config/                 # schema, loader, init
    ├── client/                 # NifiClient wrapper (Deref) + TLS + events
    │   └── tracer/             # provenance content fetch + tabular/text classifiers
    ├── cluster/                # ClusterStore + fetcher tasks + snapshot + subscriber
    ├── input/                  # KeyMap + typed action enums (FocusAction, Verb, …)
    ├── app/                    # run loop, per-view state reducers, ui, navigation, worker
    ├── intent/                 # Intent enum + IntentDispatcher
    ├── view/                   # per-tab views (overview, bulletins, browser, events, tracer)
    └── widget/                 # status_bar, help_modal, context_switcher, panel, severity, …
```

## Architecture

`nifi-lens` is a "ratatui + tokio" TUI. A single multi-thread `tokio`
runtime owns everything; the main UI task is parked on the OS main
thread via `rt.block_on(...)` because ratatui's `Terminal` is
naturally single-thread, but **state is mutated only on the UI task**
(no locks, no races) by convention, not by `!Send` constraints. The
UI loop drains an internal `AppEvent` channel, mutates state, and
redraws (60 fps cap, only on state change). A terminal task converts
`crossterm::Event` → `AppEvent::Input`. All cluster polling is owned
by `ClusterStore` (below); on-demand detail fetches go through
view-local workers under `WorkerRegistry` (`src/app/worker.rs`),
which also drives `cluster.subscribe` / `unsubscribe` on tab change.
All workers use `tokio::spawn`; wrapper futures on `NifiClient` are
`Send` (asserted by `tests/send_regression.rs`). RAII guards that
fire-and-forget cleanup HTTP DELETEs from `Drop` use
`app::cleanup::spawn_cleanup` so they no-op silently if the runtime
is gone. User actions route through a single `Intent` enum +
dispatcher.

**Modal conventions:** every full-screen modal owns a `*ModalVerb`
enum that embeds `Common(CommonVerb)` for shared chords
(`Esc`/`/`/`n`/`Shift+N`/`c`/`r`); body keys are modal-specific.
Outer-tab keys are shadowed via `input::modal_gate::ModalGate` (one
impl per modal). Search uses `widget::search`. Below
`widget::modal::MIN_WIDTH × MIN_HEIGHT` the modal degrades via
`render_too_small`. v0.1 modals are **read-only**.

### Central cluster store

`ClusterStore` owns twelve fetchers (`root_pg_status`,
`controller_services`, `controller_status`, `system_diagnostics`,
`bulletins`, `connections_by_pg`, `about`, `cluster_nodes`,
`tls_certs`, `version_control`, `parameter_context_bindings`,
`reporting_tasks`). Each runs as an independent `tokio::spawn`
future, emits `AppEvent::ClusterUpdate` on success, and sleeps for its base
cadence — scaled adaptively up to `max_interval` based on measured
latency, with ±`jitter_percent/100` jitter. Cadences live under
`[polling.cluster]` (humantime); see README "Configuration".

Snapshot mutation is main-loop-only: the `ClusterUpdate` arm in
`src/app/mod.rs` calls `state.cluster.apply_update(...)` and re-emits
`AppEvent::ClusterChanged(endpoint)`; views match on the endpoint and
invoke their `redraw_*` reducers.

Eight endpoints are **subscriber-gated** — they park when no view
subscribes: `root_pg_status`, `controller_services`,
`connections_by_pg`, `cluster_nodes`, `tls_certs`, `version_control`,
`parameter_context_bindings`, `reporting_tasks`.
`WorkerRegistry::ensure` calls `cluster.subscribe(endpoint, view)` on
tab entry and `unsubscribe` on tab exit.

Per-PG fan-out fetchers (`version_control`,
`parameter_context_bindings`, `connections_by_pg`) bound concurrent
in-flight requests via `futures::stream::buffer_unordered(N)` —
default 16, `[polling.cluster] batch_concurrency` (`0` → `1`).

Context switch: `cluster.shutdown()` aborts every fetcher and the
store is rebuilt with the new `NifiClient` in the main loop's
`pending_worker_restart` branch. Sysdiag nodewise → aggregate fallback
is handled inside the `system_diagnostics` fetcher.

`status_history` (sparkline) is selection-scoped, not a `ClusterStore`
fetcher. Events in-flight query polling (750 ms) and Tracer content
in-flight polling (500 ms) are hardcoded.

### `nifi-rust-client` integration

All NiFi API access goes through a thin `client` module that owns the
`DynamicClient` (one per active context), exposes high-level helpers
per view, and centralises error mapping, retry policy, and `tracing`
instrumentation.

**When an endpoint is missing or awkward, fix it upstream in
`nifi-rust-client` — do not work around it in `nifi-lens`.** The tool
exists partly to surface and drive those library improvements. See
"Dependency on `nifi-rust-client`" for the local-path workflow.

### Intent pipeline

All user actions route through a single `Intent` enum and dispatcher.
Write variants exist from day one, but no v0.x key binding constructs
them and `IntentDispatcher::handle_pure` returns
`NifiLensError::WriteIntentRefused` for every write variant. The
`--allow-writes` CLI flag is `#[arg(hide = true)]` and `lib.rs`
rejects it at startup before the runtime spins up — the dispatcher
guard is defense-in-depth.

### Error handling

- **Library-style modules** (`config`, `client`, `intent`): `snafu`
  to match `nifi-rust-client`.
- **Application edge**: errors bubble up to `lib::run()`, which prints
  to stderr and returns a non-zero `ExitCode`.
- **In-TUI errors**: transient status-line banner with optional detail
  modal. Never written to stdout while the TUI is active — it
  corrupts the terminal.
- **No panics in data paths.** Panics are only acceptable on config
  parse failures at startup.
- **No `unwrap` / `expect` in production code.** Map `Option`/`Err` to
  `NifiLensError` variants. Tests may `unwrap`.

### Input layer

All keyboard input flows through `src/input/`. `KeyMap` translates
`crossterm::KeyEvent` into `InputEvent` carrying typed action enums:
`FocusAction` (Up/Down/Left/Right/PgUp/…/Descend/Ascend/NextPane/PrevPane),
`HistoryAction` (Back/Forward — `Shift+←`/`Shift+→`), `TabAction`
(F1–F5), `AppAction` (Quit/Help/ContextSwitcher/FuzzyFind/Jump/Paste/Cut),
and `ViewVerb` wrapping per-view enums.

Shared chords live on `CommonVerb`; per-view enums embed a
`Common(CommonVerb)` arm. Modal shadow dispatch goes through
`ModalGate` — one impl per modal, chained inside `KeyMap::translate`.
Adding a new modal is a single gate impl.

Every enum implements the `Verb` trait, the **single source of truth**
for chord, label, hint text, enabled predicate, and truncation
priority. Hint bar and help modal both iterate `Verb::all()` — adding
a keybinding cannot desync the two surfaces. Verbs already visible
adjacent to the hint strip can opt out via `show_in_hint_bar() ->
false` while still appearing in `?` help.

Views expose a small trait surface (`handle_verb`, `handle_focus`,
`default_cross_link`, `is_text_input_focused`, `handle_text_input`)
instead of raw `KeyEvent` matches. `FocusAction::Descend` =
drill/activate/submit; `Ascend` = leave focused pane / cancel input.
When a view has no local descent target, `Enter` falls back to
`default_cross_link` (Bulletins → Browser).

`F12` dumps the keymap reverse table + subscriber state to the log
(unadvertised debug aid).

### Adding a new view

1. Create `src/view/<name>/` with `mod.rs`, `state.rs`, and rendering
   (single `render.rs` like Events, or a `render/` submodule like
   Browser). Add `worker.rs` only if on-demand detail fetches needed.
2. Add a `ViewId::<Name>` variant; update `next()` / `prev()`.
3. Create `src/app/state/<name>.rs` with a `<Name>Handler` ZST
   implementing `ViewKeyHandler`.
4. Add one arm to `dispatch_handler!` in `src/app/state/mod.rs`.
5. For live cluster data, subscribe/unsubscribe `ClusterEndpoint`s in
   `WorkerRegistry::ensure`. For on-demand fetches, spawn a view-local
   worker.
6. Add a render arm to `src/app/ui.rs`.
7. Add a top-bar label (`src/widget/top_bar.rs`).

For a tab that needs a *sub-mode* rather than a new view, see the
Events watch sub-mode (mode discriminator on existing state, alternate
worker, mode-aware `WorkerRegistry` teardown). No new `ViewId`
required.

### Visual language

A project-wide bordered-box visual language goes through
`widget::panel::Panel`. Focused panels flip to `BorderType::Thick` +
accent colour; unfocused use plain borders + `theme::border_dim()`.
New interactive sub-panels use `↑`/`↓` for row nav — `j`/`k` aliases
are not used app-wide. Severity rendering (labels, colours, icons) is
consolidated in `widget::severity` and `widget::run_icon`; call those
helpers rather than inline `Color::*`/`Modifier::*`.

Shared helpers worth knowing about before reinventing them:

- `src/widget/modal.rs` — `MIN_WIDTH` / `MIN_HEIGHT`,
  `render_too_small()`, `render_verb_hint_strip<V: Verb>()`.
- `src/widget/scroll.rs` — `VerticalScrollState` /
  `BidirectionalScrollState`.
- `src/widget/filter_bar.rs` — `FilterChip` + `build_chip_line`.
- `src/widget/search.rs` — `SearchState` + `compute_matches` +
  `render_search_input` (`/ {query}_` footer prompt).
- `src/layout.rs` — `split_header_body_footer` / `split_two_rows` /
  `split_two_cols` / `center_percent` / `center_absolute`.
- `src/bytes.rs` — `KIB` / `MIB` / `GIB` + `format_bytes` /
  `format_bytes_int`. Prefer over raw `N * 1024 * 1024`.
- `src/client/status.rs` — `ProcessorStatus` /
  `ControllerServiceState` / `PortStatus` / `TransmissionStatus`
  typed enums.
- `src/client/mod.rs` — `ROOT_GROUP_ID` (NiFi's documented `"root"`
  alias for the root process group). Use this rather than the bare
  `"root"` literal.
- `src/timestamp.rs` — `format_age` / `format_age_secs`.
- `src/test_support.rs` — `fresh_state`, `tiny_config`,
  `default_fetch_duration`, `test_backend(height)` and
  `TEST_BACKEND_*`.

### Per-feature wiring notes

User-facing behaviour and keybindings live in `README.md`. The
following are mechanics that aren't derivable from reading the code or
already covered there.

- **Standalone-409 detection**
  (`cluster/fetcher_tasks.rs::error_is_standalone_409`) —
  `/controller/cluster` returns 409 on standalone NiFi. The matcher
  inspects the error's debug repr for `"409"`, the `NotClustered`
  variant, **and** NiFi's "Only a node connected to a cluster"
  message; the last is needed because some `nifi-rust-client` versions
  map the 409 to `NotFound` without the status code in the repr.
- **TLS cert fetcher** force-wakes on roster change via
  `publish_node_addresses`. HTTP-only contexts skip with a one-time
  info log. Severity thresholds (`<7d` red, `7..30d` yellow, `≥30d`
  muted) are hardcoded.
- **Drift index reuse** — `FlowIndex` ProcessGroup entries carry a
  `VersionControlInformationDtoState` re-stamped on every
  `ClusterChanged(VersionControl)`, so fuzzy-find drift filters work
  without a separate index.
- **Parameter ref annotation** — `#{name}` in property values gains a
  trailing `→`; `##{literal}` is the documented escape and is *not*
  annotated. Multi-ref values annotate but open without preselect.
- **Connection endpoint backfill** — NiFi leaves `source_id` /
  `destination_id` null on `ConnectionStatusSnapshotDto`;
  `connections_by_pg` backfills via parallel per-PG
  `/process-groups/{id}/connections` calls.
  `REMOTE_INPUT_PORT` / `REMOTE_OUTPUT_PORT` connectables write the
  parent RPG's `group_id` (not the port UUID) so cross-links land on
  the RPG arena entry.
- **Folders are reducer-only** — the client walker emits flat nodes;
  `apply_tree_snapshot` synthesises `Folder(Queues)` /
  `Folder(ControllerServices)` rows and re-parents the leaves. Folders
  never cross-link, never fetch, and are excluded from fuzzy-find.
- **Browser cross-link gating** — `BrowserState::resolve_id` checks
  canonical-UUID shape *before* scanning `state.nodes` (linear scan,
  once per annotatable row). Selected-relationships on connections
  intentionally aren't surfaced in the processor Connections section
  (data lives on `ConnectionDTO`, not the status snapshot).
- **Bulletins dedup** — beyond the monotonic-`id` ring, the reducer
  dedupes by `(source_id, message_stem)`: strips
  `ComponentName[id=<uuid>]` and normalises dynamic `[...]` regions,
  so repeating errors collapse into one row with `×N` count. Detail
  modal snapshots `GroupKey` + `GroupDetails` on open so subsequent
  ring mutations don't disturb it; `Enter` is intentionally a no-op
  inside it.
- **Access fixture (managed-authorizer)** — the integration test fixture
  runs NiFi's `managed-authorizer` with `FileUserGroupProvider` +
  `FileAccessPolicyProvider`. Two pre-baked XMLs in
  `integration-tests/scripts/conf/` configure it; `users.xml` and
  `authorizations.xml` are NOT pre-baked — NiFi auto-creates both on
  first start using `Initial User Identity 1-5`
  (admin/alice/bob/carol plus the cluster-node DN `CN=localhost`) and
  `Initial Admin Identity = admin`
  in `userGroupProvider`, plus `Node Identity 1 = CN=localhost` in
  `accessPolicyProvider` to seed the `/proxy` write policy. The
  cluster-node DN must be listed as `Initial User Identity X` in
  `userGroupProvider` (not `Node Identity X`, which is silently ignored
  there) for FileAccessPolicyProvider to find the user when seeding the
  proxy policy. The bcrypt admin password hash uses `$2b$` prefix —
  NiFi's `BCryptPasswordEncoder.matches` calls `verifyStrict` against
  `VERSION_2B` and rejects `$2a$` hashes. The seeder's
  `bootstrap_admin_policies` (in
  `integration-tests/seeder/src/access_fixture.rs`) runs BEFORE
  nuke-and-repave and grants admin per-root-PG policies that NiFi's
  Initial Admin doesn't auto-create in clustered mode (the root PG
  UUID isn't known when the FileAccessPolicyProvider initializes).
  CN=localhost is added alongside admin on every fixture-side `/data`
  policy because cluster federation walks the proxy chain and requires
  *each* user in the chain to have access. `/data/process-groups`
  inheritance only walks one level (component → parent PG), so
  `grant_data_recursively` + `grant_component_data_recursively` walk
  the full marker hierarchy and add explicit policies on every PG,
  processor, and connection. Public input ports also need
  `/data-transfer/input-ports/{id}` write per-port for the RPG → S2S
  handshake to enumerate them (NiFi rejects a wildcard
  `/data-transfer/input-ports` policy). The `seed_access_fixture` pass
  creates the `ops-team` group via REST and attaches realistic
  component-level policies on top. Cleanup intentionally does NOT
  delete users or groups — they're persistent across seeder runs;
  only PG-scoped policies get auto-deleted by NiFi when their parent
  PGs are nuked.
- **Access modal — inheritance & auth-disabled** —
  `BrowserVerb::OpenAccess` (`u`) opens a 5-axis matrix modal whose
  worker fans out
  `client.policies().get_access_policy_for_resource(action, resource)`
  via `buffer_unordered(5)`. **Inheritance is detected by comparing
  `response.component.resource` to the requested resource** — when they
  differ, the cell renders `↑` and the source is annotated.
  **Single-user-authorizer is not Unsupported**: it returns 200 with
  the implicit admin policy and the matrix renders correctly with a
  single identity. `AccessAuditState::Unsupported` only fires on 409
  ("no authorizer configured") or a blanket 403 ("Access is denied.
  Contact the system administrator") from an unsecured NiFi — same
  general detection lineage as
  `cluster/fetcher_tasks.rs::error_is_standalone_409`. Drill-in uses
  inline `accessPolicies` on `UserDto` / `UserGroupDto` so a single
  `tenants/{...}/{id}` call is sufficient. Two `ModalGate` impls chain
  in `KeyMap::translate` with `IdentityModalGate` ahead of
  `AccessModalGate` (deeper modal wins).
- **Action history paginator** — worker eagerly fetches page one then
  sleeps on a `tokio::sync::Notify` until the reducer wakes it. 100
  rows/page; auto-load when viewport bottom is within 10 of the loaded
  tail. Modal `Esc` cascades through search → expanded → close.
- **Reporting tasks** piggyback on Overview's subscription rather than
  adding a separate gate; "recent bulletins" filters the existing ring
  by `source_id`, no extra fetch.
- **Sparkline reducer fallback** — `reduce_status_history` reads
  `aggregateSnapshots` first, then sums `nodeSnapshots[*].statusSnapshots`
  per timestamp (NiFi clustered mode often returns empty aggregate).
  Worker is re-created on every selection change to a supported kind;
  reducer arms gate by `(kind, id)` to drop stale emits between abort
  and exit. 404 → `SparklineEndpointMissing` (sticky per selection).
  Strip is suppressed when remaining width < 12 cells.
- **Tracer streaming** — 512 KiB chunks via `provenance_content_range`;
  next chunk auto-fires when viewport bottom comes within 100 lines of
  decoded tail. Per-chunk classification uses
  `classify_text_or_hex_no_pretty` so chunk arrivals never block the
  UI thread.
- **Tracer JSON pretty-print** runs **once** off-thread when the side
  is fully loaded, via `serde_transcode` (`Deserializer` →
  `Serializer::pretty`) — this avoids the `serde_json::Value`
  round-trip and **preserves object key order**.
- **Tracer tabular** — `classify_content` catches Parquet/Avro decode
  errors, logs `warn!`, and surfaces them as `Hex` (the classifier
  signature stays infallible). Per-side ceiling resolves *after* the
  first chunk arrives. Parquet's footer lives at EOF, so a ceiling-hit
  fetch falls back to `Hex` with a chip; Avro is streamable and
  degrades via `truncated = true`. Tabular sides diff iff their
  `format` tags match.
- **Events watch sub-mode** — `EventsState.mode` is `OneShot` /
  `Watch(WatchSession)`; mode is implicit (empty predicate = one-shot,
  populated = watch). Predicate grammar: `attr op literal (AND …)*`
  with `= / != / =~ / !~` and `/.../` regex literals. Missing-attribute
  semantics: `=`/`=~` → false, `!=`/`!~` → true. **Cost guard:**
  `WatchSession::can_start` requires at least one of `component_id`,
  `flow_file_uuid`, `event_types`, or non-blank `start_time_iso`
  before the worker spawns. Tab switch is paused-and-resumed
  (buffer, predicate, and cursor survive). RAII guard fires
  `DELETE /provenance/{id}` on every exit path; submit/poll backoff
  is 5/10/30/60s capped by `[events] watch_retry_max`. A confirm modal
  protects users from discarding a non-empty buffer.
- **Queue listing** wraps NiFi's two-phase listing-request flow
  (`POST listing-requests` → poll `GET listing-requests/{id}` until
  `finished` → `DELETE`). `QueueListingHandle::drop` fires-and-forgets
  the `DELETE`; NiFi's listing-request TTL is the safety net if Drop
  misses. Polling cadence is 500 ms (not user-configurable). Server
  caps at 100 rows; `total > 100` → `[100 / N]` chip.
- **Fuzzy Find filter** (`Shift+F`) — a leading `:`-token narrows the
  corpus before fuzzy scoring via a `QueryFilter` enum (`:proc`,
  `:pg`, `:cs`, `:conn`, `:in`, `:out`, `:rpg`, `:drift`, `:stale`,
  `:modified`, `:syncerr`). Unknown or non-leading `:tokens` are plain
  query text. The chip row above the query line is read-only — the
  query string is the single source of truth.

## Dependency on `nifi-rust-client`

Depended on with the `dynamic` feature. The bottom of `Cargo.toml`
carries a **commented-out** `[patch.crates-io]` block pointing at a
sibling `../nifi-rust-client/crates/nifi-rust-client` worktree.
Uncomment locally to iterate against an unreleased change; recomment
before pushing. A forgotten uncomment will break CI on the first cargo
job (the sibling path doesn't exist on runners) — that is the intended
guardrail; do not try to teach CI to tolerate it.

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

**MSRV is `1.88`** (CI enforces via `RUSTUP_TOOLCHAIN`); `rust-toolchain.toml`
pins `1.93.0` for development. MSRV was raised from 1.85 to pull in
`time >= 0.3.47`, which fixes
[RUSTSEC-2026-0009](https://rustsec.org/advisories/RUSTSEC-2026-0009).

Rustdoc discipline: doc comments on `pub` items must not `[`link`]` to
private items — CI fails on private-link warnings. Use plain backticks.

### Integration test fixture

`./integration-tests/run.sh` brings up the floor (NiFi 2.6.0
standalone) and ceiling (2-node 2.9.0 cluster + ZooKeeper) versions,
seeds both, and runs `cargo test --test 'integration_*' -- --ignored`.
README "Development" covers the live-dev workflow and headline failure
narrative.

`run.sh` invokes `scripts/download-nars.sh` first to fetch
`nifi-parquet-nar` (and transitive `nifi-hadoop-libraries-nar`) from
Maven Central into a gitignored cache. The `apache/nifi` images don't
bundle the standalone Parquet writer, so the per-version mount into
`/opt/nifi/nifi-current/nar_extensions/` is required for
`diff-parquet-writer`.

**Fixture shape** — top-level marker PG `nifilens-fixture-v8` holds 6
child PGs: `orders-pipeline/` centerpiece, `remote-targets/` sibling,
and four standalones for hard-to-reach states (`invalid-pipeline`,
`backpressure-pipeline`, `versioned-clean`, `versioned-modified`); 5
parameter contexts in a 3-tier inheritance chain
(`fixture-pc-platform` → `fixture-pc-orders` →
`fixture-pc-region-{eu,us,apac}`), and 4 root-level CSes. See
`integration-tests/seeder/src/fixture/` for the authoritative layout.

**Diff coverage** — JSON↔JSON, JSON↔CSV grayed-out, Parquet↔Parquet,
Avro↔Avro. NiFi often reports
`inputContentClaim == outputContentClaim` on `CONTENT_MODIFIED` even
when bytes differ — always fetch both sides.

NiFi processor *property keys* drift between minor versions even when
display names are stable; setting a property by display name when the
real key differs silently turns it into a dynamic attribute. The
seeder handles known cases via
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

The **floor version 2.6.0 never drops** — it stays pinned forever so
the dynamic client is always tested against the oldest supported NiFi.

## Release

Releases are driven by `cargo-release` via `release/release.sh`, a
thin passthrough wrapper. **`cargo-release` is dry-run by default**;
pass `--execute` to actually release. The release commit updates
`Cargo.toml` `version`, `Cargo.lock`, and `CHANGELOG.md`
(`## [Unreleased]` → `## [X.Y.Z] — YYYY-MM-DD`, fresh `## [Unreleased]`
inserted, compare link rewritten).

Two workflows fire on every `v*.*.*` tag:

1. `publish-crate.yml` — verifies tag matches `Cargo.toml`, runs the
   full check suite, `cargo publish`es to crates.io.
2. `release.yml` — autogenerated by cargo-dist. Builds per-target
   archives (Linux x86_64/aarch64 gnu+musl, macOS x86_64/aarch64,
   Windows x86_64); uploads them plus shell / PowerShell installers
   and a Homebrew formula to a GitHub Release; writes notes from the
   `## [X.Y.Z]` CHANGELOG section.

The local machine never publishes; `cargo-release` is configured with
`publish = false` so `CARGO_REGISTRY_TOKEN` lives only in GitHub.

**Never hand-edit `release.yml`** — it is regenerated. To change
targets / installers / cargo-dist version: edit `dist-workspace.toml`,
run `dist generate`, commit the regenerated workflow alongside the
config. Install once: `cargo install cargo-release --locked && cargo
install cargo-dist --locked` (the cargo-dist binary is named `dist`).

Homebrew tap (not yet configured): create `maltesander/homebrew-tap`,
add `tap` / `formula` keys to `dist-workspace.toml`'s `[dist]` table,
regenerate, and add a `HOMEBREW_TAP_TOKEN` repo secret with
`contents: write` on the tap repo.

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
- When architecture / patterns change, update `AGENTS.md` in the same
  commit.
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
