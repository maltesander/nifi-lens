# AGENTS

## Project Overview

`nifi-lens` is a keyboard-driven terminal UI for observing and debugging
Apache NiFi 2.x clusters. It is powered by
[`nifi-rust-client`](https://docs.rs/nifi-rust-client) used exclusively via
the `dynamic` feature, so one binary works against every supported NiFi
version in the fleet. v1 is read-only, multi-cluster (kubeconfig-style
context switching), and forensics-focused — it is explicitly a *lens*, not
a canvas replacement.

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
└── src/
    ├── lib.rs              # public entry: pub fn run() -> ExitCode
    ├── main.rs             # thin wrapper: std::process::exit(nifi_lens::run())
    ├── cli.rs              # clap derive: Args, Command, ConfigAction, LogLevel
    ├── error.rs            # NifiLensError (snafu, full variant set)
    ├── logging.rs          # tracing-subscriber + rotating file + StderrToggle
    ├── theme.rs            # color / style constants
    ├── event.rs            # AppEvent, IntentOutcome, ViewPayload
    ├── config/             # mod, loader, init — schema, load, config init
    ├── client/             # mod, build — NifiClient wrapper (Deref) + TLS
    ├── app/                # mod (run loop), state/ (per-view reducers + ViewKeyHandler), ui (frame render), navigation, worker
    ├── intent/             # Intent enum + IntentDispatcher
    ├── view/               # per-tab views; overview/ shipped Phase 1 with state/render/worker
    └── widget/             # status_bar, help_modal, context_switcher
```

Phase 1+ will grow the existing modules with real per-view data workers
and renderers, and add `fuzzy/` (nucleo-backed find) and `util/` when the
first callers land. See [Phase Roadmap](#phase-roadmap) for the shipping
order.

## Architecture

`nifi-lens` follows a standard "ratatui + tokio" split:

- **Single `tokio` multi-thread runtime with a main-thread `LocalSet`** owns everything.
- **UI loop** runs on the main task. It drains an internal `AppEvent`
  channel, mutates state, and redraws (60 fps cap, only when state changed).
- **Terminal event task** converts `crossterm::Event` → `AppEvent::Input`.
- **Per-view data worker tasks** poll the relevant `NifiClient` endpoints
  and push `AppEvent::Data(view, payload)` into the channel. Workers are
  spawned on tab activation and cancelled on tab switch, so API load is
  proportional to what the user is actually looking at. The run loop owns
  a `WorkerRegistry` (`src/app/worker.rs`) holding at most one
  `JoinHandle<()>`: on every tab change it aborts the previous worker and
  spawns the new view's worker (no-op when the view already matches).
  Phase 2 added the Bulletins worker on a 5-second cadence; Browser /
  Tracer workers land in their respective phases. Each view's worker
  resumes from its own cursor on tab re-entry — for Bulletins that is
  `AppState.bulletins.last_id`, passed into `spawn` by the
  `WorkerRegistry`. Workers run via
  `tokio::task::spawn_local` on the main-thread `LocalSet` (wired in
  `src/lib.rs`) because `nifi-rust-client` dynamic traits return `!Send`
  futures.
- **Intent dispatcher** handles one-shot actions (trace a UUID, drill into
  a process group, fetch content for an event). It runs tasks off the
  runtime and pushes results back via the same channel.

State is mutated **only on the UI task**. No locks, no races.

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
Write variants exist in the enum from day one so v2 does not require
restructuring, but no key binding constructs them in v1 and the dispatcher
refuses to execute them without an `--allow-writes` CLI flag (which v1
does not expose).

### Error handling

- **Library-style modules** (`config`, `client`, `intent`): `snafu`, to
  match `nifi-rust-client`.
- **Application edge**: `color-eyre` for pretty crash reports.
- **In-TUI errors**: surfaced as a transient status-line banner with an
  optional detail modal (`e` to expand). Never printed to stdout while the
  TUI is active — it corrupts the terminal.
- **No panics in data paths.** Panics are only acceptable on config parse
  failures at startup.

### Logging

`tracing` + `tracing-subscriber` + `tracing-appender` write to
`~/.local/state/nifilens/nifilens.log` (rotating, 5 files × 10 MB).
Default level `info`; `--debug` raises to `debug`. **Never** writes to
stdout or stderr while the TUI is active.

### Bulletins ring buffer

The Bulletins tab holds a rolling in-memory window of recently-seen
bulletins. The cap is controlled by `[bulletins] ring_size` in
`config.toml` (default 5000, valid range 100..=100_000). Memory budget
at the default is ~1–2 MB. The worker polls
`flow_api().get_bulletin_board(after, limit=1000)` every 5 seconds,
dedups via the monotonic `id` cursor, and drops from the front when the
ring exceeds its capacity.

**Accepted edge cases:**

- **Cluster restart cursor drift.** If NiFi restarts and its in-memory
  bulletin IDs reset to a lower number, the Bulletins worker will see
  empty batches until the server's ID stream overtakes `last_id`. Users
  relaunch the tool in practice. A cursor-reset heuristic is a Phase 5
  polish item.
- **`e` key collision.** Phase 0's global `e` expands the error-banner
  detail. When the Bulletins tab is active, the view-local `e` wins
  (toggles the Error chip). A cleaner collision rule is a Phase 5
  polish item.

**Accepted Phase 3 edge cases:**

- **Cross-link race on first Browser entry.** `Enter` on a Bulletins
  row before the Browser tab has ever been visited yields a
  `"component ... not found in current flow tree"` warning banner.
  The user retries after the next 15 s tick. Queue-and-retry fix is
  a Phase 5 polish item.
- **Detail fetch failures leave `pending_detail` set.** The loading
  skeleton persists until selection moves or a retry succeeds. The
  error banner is the user-visible feedback. Phase 5 polish item.
- **Second `e` collision.** Phase 2's Bulletins `e` already overrode
  the Phase 0 banner-expand. Phase 3 adds a second override for the
  Browser properties modal. Phase 5 tracks a cleaner collision rule.
- **Ports render in the tree but have a minimal detail pane.**
  Richer port detail is deferred to Phase 5.
- **Clipboard failures surface as warning banners.** Not errors;
  clipboard is a nice-to-have.
- **PG-scoped recent bulletins show 0.** *(resolved in UI Reorg
  Phase 5)* The PG detail pane renders a
  `"Recent bulletins (N in this PG)"` header and up to 3 recent
  matching rows. Phase 5 widened `browser::render::render` to
  accept `&VecDeque<BulletinSnapshot>` and added the
  `view::bulletins::state::recent_for_group_id` helper.
- **PG tree row counts show 0.** `ProcessGroupStatusSnapshotDto`
  does not expose per-PG `running`/`stopped`/`invalid`/`disabled`
  counts; the recursive fetch populates those fields with 0. The PG
  detail pane shows the real counts via the separate `get_process_group`
  call. Per-row tree counts will be wired in a follow-up when an
  endpoint or library change exposes them.

**Accepted Phase 4 edge cases:**

- **LatestEvents cross-link uses a fixed limit of 20.** When the
  Bulletins `t` or Browser `t` cross-link fires, the Tracer fetches the
  20 most recent provenance events for the component. Events older than
  the 20-event window are not shown. A configurable limit is a Phase 5
  polish item.
- **Full provenance search deferred.** The `POST /provenance` endpoint
  (time-range / relationship / attribute filters) is not wired in Phase
  4. The Tracer entry form only accepts a flowfile UUID. Full search mode
  is a future phase item.
- **Save-to-file overwrites without confirmation.** Pressing `s` on the
  content pane writes the raw bytes to the default path immediately;
  there is no overwrite prompt. A confirmation modal is a Phase 5 polish
  item.
- **Lineage poll timeout is not user-configurable.** The in-TUI lineage
  poller retries up to a hard-coded limit before surfacing a timeout
  banner. A `[tracer] lineage_timeout_secs` config knob is deferred to
  Phase 5.
- **`CrossLink::TraceComponent` carries no `since` field.** The field
  was dropped during Phase 4 implementation; Bulletins `t` and Browser
  `t` always land on the 20 most recent events rather than events since
  the bulletin timestamp. Phase 5 polish item.
- **Third `e` collision.** Phase 4 introduces a third override of the
  `e` key for the Tracer event-detail expand. Phase 6 tracks a cleaner
  per-mode collision rule.

**Accepted Phase 5 edge cases:**

- **Node diagnostics aggregate fallback.** *(resolved in Phase 6)* When
  the nodewise system-diagnostics call fails, the worker retries with
  aggregate-only and shows a single "Cluster (aggregate)" row with a
  warning banner.
- **Stalled queue display.** *(resolved in Phase 6)* Connections with
  queued items but zero throughput now show `∞ (stalled)` in red instead
  of the misleading `stable` label.
- **Health tab `Enter` hint for non-crosslink rows.** *(resolved in
  Phase 6)* Pressing `Enter` on a repository or node row now shows an
  info-severity banner instead of silently doing nothing.
- **Repository fill bars now show per-node breakdown.** *(resolved in
  Phase 6)* Selecting a repository row with `j`/`k` in the Repositories
  category shows per-node fill bars in a detail pane on the right.

**Accepted UI Readability edge cases:**

- **Icon coloring vs selected row.** On a selected (highlighted) row,
  the run-state icon's foreground color fights the `REVERSED` modifier.
  REVERSED wins; the icon is still recognizable by glyph shape alone.
- **Spark-bar resolution.** A 4-cell spark-bar has 32 discrete fill
  states. Sub-cell load changes will not re-render visibly. This is a
  trend indicator paired with the numeric label, not a precision readout.
- **Grouping with auto-scroll pause.** When auto-scroll is active and
  `Shift+B` collapses incoming bursts, the visible row count shrinks.
  Auto-scroll stays on row 0; the user sees fewer rows, each
  representing more bulletins.
- **`num_cpus` missing on a node.** When `available_processors` is
  absent from the DTO, the Load column falls back to the plain numeric
  rendering with no spark-bar. Defensive default; the field is
  populated on every supported NiFi version in practice.
- **`time` crate, not `chrono`.** The spec said `chrono`; the
  implementation uses `time` 0.3 because it is already a dependency and
  covers every requirement (parse + format + local-offset).

**Accepted UI Reorg Phase 2 edge cases:**

- **Text-input modes swallow bare-letter global chords.** After the
  keybinding sweep, printable characters `f` / `K` / `[` / `]` / `B`
  are captured by any text-input mode they reach (Bulletins `/` filter,
  Tracer Entry UUID prompt, Browser breadcrumb-focus mode, Fuzzy find
  modal query buffer). Users must press `Esc` to exit the text-input
  mode before the bare-letter chord reaches the global handler. This
  is consistent with vim-modal convention and is the same trade-off
  that the old `Shift+B` already had.

**Accepted UI Reorg Phase 3 edge cases:**

- **Processor thread leaderboard dropped.** The old Health "Processors"
  category showed a top-N processor-by-active-threads leaderboard.
  Layout 3's Overview has no equivalent panel — the processor info
  line shows aggregate counts only. Per-processor thread
  investigation is still possible via the Browser detail pane.
  Re-introducing a leaderboard would require a future detail-pane
  drill-in.
- **Per-node repository breakdown dropped.** The old Health
  "Repositories" detail pane showed per-node fill bars when a row
  was selected. Overview shows only the cluster-aggregate row
  (averaged via the server's `SystemDiagAggregate`). Per-node
  breakdown would land via a future `Enter`-on-row drill-in detail
  pane.
- **`connected_nodes` always equals `total_nodes`.** The current
  `NodeDiagnostics` shape on `nifi-rust-client` 0.9 does not carry a
  per-node connected/disconnected status field, so the cluster
  summary reports `connected = total = diag.nodes.len()`
  unconditionally. The top-bar identity strip therefore always
  renders the node count in muted style. A real connected-vs-total
  distinction needs an upstream library change.
- **Aggregate-fallback warning persists.** When the nodewise sysdiag
  call fails and the worker falls back to aggregate-only, the
  reducer surfaces a WARN banner each cycle. The banner has no
  auto-clear (it persists until dismissed), so under sustained
  failure the user sees the same warning until they press Esc.
  Auto-suppress on repeat is a polish item.
- **Queue time-to-full predictions dropped from the UI.** The old Health
  "Queues" category showed server-predicted `~30s` / `~2m` / `stable`
  hints in a `time-to-full` column, and a red `∞ (stalled)` badge for
  connections with queued items but zero throughput (Phase 6 polish).
  Layout 3's unhealthy-queues table shows fill / queue / src→dst /
  ffile count only. The prediction data is still available via
  `connections[i].predicted_millis_until_backpressure`; restoring the
  column is tracked as a future polish item.
- **Queue Enter-cross-link dropped.** The old Health queue rows
  supported `Enter` → Browser jump for the selected connection.
  Layout 3's unhealthy-queues table is non-interactive — `Enter` on
  a queue row is a no-op. Restoring the cross-link is a future
  polish item.
- **`spawn_polling_worker` retained as orphan helper.** The
  `app::worker::spawn_polling_worker` helper is still used by the
  Bulletins worker. The Overview worker no longer uses it after the
  Phase 3 dual-cadence rewrite. Keeping the helper is acceptable for
  now (one consumer earns it); deletion would land in a polish pass
  if Bulletins ever moves to a dual-cadence pattern.

**Accepted UI Reorg Phase 4 edge cases:**

- **Stem extraction is name-agnostic.** `strip_component_prefix`
  accepts any text before `[id=<anything>]` and requires exactly
  one trailing ASCII whitespace. Messages whose prefix is malformed
  (missing `]`, no trailing space, embedded newlines before the
  bracket) fall through unchanged and dedup by the full raw
  message — they will not collapse with correctly-prefixed
  variants. Acceptable because NiFi's own emission is consistent
  in practice.
- **Dedup key is stem-only, not attribute-aware.** Two bulletins
  from the same processor with the same stem but different bound
  attribute values in the stem (e.g. `… uuid=A` vs `… uuid=B`)
  are distinct rows because the attribute values are part of the
  stem. Collapse-across-attrs is a future polish item.
- **PG path fallback to UUID tail.** When the Browser tree has not
  yet been populated (user lands on Bulletins first), the PG path
  column shows `…d59706` (last 8 chars of the group UUID) in
  muted style. No warning banner. Opening Browser once seeds the
  arena and subsequent Bulletins refreshes render the real path.
- **Mute is session-scoped.** Mutes live in `BulletinsState.mutes`
  and reset on every TUI restart. No config persistence. Pressing
  `m` on an already-muted source unmutes it via the toggle path —
  but since muted rows are hidden, the user can't reach them
  through normal navigation; the primary unmute path is TUI
  restart. A "muted sources" modal is a future polish item.
- **Severity chip counts ignore other filters.** `[E 87]` means
  "87 ERROR rows exist in the ring right now", not "87 ERROR rows
  pass the current component-type / text / mute filters". The chip
  tells the user what hiding that severity would hide from the
  total ring, not the currently-filtered view. Re-reading as
  current-filter-scoped is a future polish item.
- **`t` still lands on Tracer, not Events.** The spec calls for
  `t` → Events (with component + last-15m prefilter). Phase 6 adds
  the Events tab and retargets `t` then. Phase 4 deliberately did
  not regress the existing Tracer cross-link.
- **Old Phase 2 `B` consecutive-group toggle is gone.** Users who
  had muscle memory for `Shift+B` → "bundle consecutive" must now
  cycle `g` through `source+msg` (equivalent and stronger) or
  `off` (raw timeline).
- **`g` no longer jumps to oldest.** The vim-style `g`/`G` pair is
  now asymmetric — `g` cycles group modes while `G` still jumps to
  newest. Use `Home` to jump to oldest. The asymmetry is
  acceptable because `g` delivers more value as a mode cycler than
  as a vim jump.

**Accepted UI Reorg Phase 5 edge cases:**

- **Lost at-a-glance PG counts in the tree.** Dropping
  `● 5 ○ 2 ⚠ 0 ⌀ 1` trailing summaries means scanning for
  "which PG has the most invalids" requires drilling in. The
  tree marker color rolls up to a ternary red/yellow/green
  signal, not a precise count. User accepted this during
  brainstorming.
- **Invalid-only tree badge declined.** A compromise showing
  `⚠2` inline for PGs with any invalid descendants was offered
  and declined — clean tree wins.
- **Connection time-to-full prediction not wired.** The spec
  calls for a time-to-full badge on Connection detail using the
  server-predicted `predicted_millis_until_backpressure` field.
  That field is available on `QueueSnapshot` (Overview) but not
  yet on `ConnectionDetail` — populating it requires an upstream
  `nifi-rust-client` change. Polish item.
- **CS referencing-components list not wired.** The spec calls
  for a "referencing components" count + short list on
  Controller Service detail. That requires a second API call to
  `GET /controller-services/{id}/references`, which is not
  currently plumbed. Polish item.
- **PG health rollup ignores bulletin severity.** The rollup
  only considers descendant processor `run_status`, not recent
  bulletins. A processor can be RUNNING while emitting ERROR
  bulletins and the PG marker stays green. Bulletin-aware
  rollup is a future polish item.
- **PG health rollup excludes Controller Services.** CS state
  (`DISABLED`, `INVALID`) does not contribute to the rollup.
  Intentional — the rollup models "is the flow running" rather
  than "is every component healthy".
- **Connection detail has no Recent bulletins section.**
  Connections rarely emit bulletins directly in practice;
  adding the plumbing for negligible value is intentional scope
  tightening. The existing PG-scoped section on the parent PG
  detail covers the cluster-level view.
- **Duplicate `format_severity_label` / `severity_style` across
  render leaves.** `pg.rs`, `processor.rs`, and
  `bulletins/render.rs` each define their own copies. The
  helpers are 10 lines each and identical; DRY-ing them into a
  shared module would widen the Phase 5 scope. A polish task
  can consolidate when the next render file lands.
- **PG detail "Recent bulletins" walks the ring twice.** Once
  for `recent_for_group_id` (up to 3 most recent) and once for
  the total count in the header. The ring is capped at 5000 by
  default, so the O(2n) walk is cheap. A single pass returning
  `(Vec<&Bulletin>, usize)` would be slightly more efficient;
  deferred as polish.
- **Processor detail hints line scrolls off the 24-line test
  terminal.** The `processor_detail_with_many_properties`
  snapshot test uses `TestBackend::new(100, 24)` and the
  fixture is dense enough that the `Recent bulletins` header
  and action hints row fall below the 24-line viewport. The
  code renders both lines correctly; the snapshot captures
  only what a 24-row terminal would show. Bumping the test
  terminal height is a polish item.

**Accepted UI Reorg Phase 6 edge cases:**

- **No tab completion for `T` type field.** The spec calls for Tab
  to complete known event types (DROP, EXPIRE, ROUTE, etc.). v1
  supports free-text inline edit only. A polish task can wire
  completion against the static event-type list.
- **No parent/child uuid display in detail pane.** The
  `ProvenanceEventSummary` DTO does not carry parent/child flowfile
  uuids. Populating those rows requires a second
  `GET /provenance-events/{id}` per row or upstream library work.
  Deferred; the detail pane shows relationship, component, group id,
  and flowfile uuid only.
- **Bulletins/Browser `t` clobbers `source` + `time` filters.**
  Cross-linking from these tabs overwrites the user's current
  `source` and `time` filter values. The `types`, `uuid`, and
  `attr` fields are preserved. This is the user-friendlier behavior
  when iterating — if it proves confusing, a polish item can
  clear all other filters on cross-link.
- **`truncated` false positive at exactly the cap.** The reducer
  marks `truncated = true` when `total_count > max_results` (strict
  greater-than). A query that returns exactly 500 events is NOT
  marked truncated — this is the conservative read: at exactly the
  cap the server either clipped at our request or coincidentally
  matched. In practice the server returns fewer than cap unless it
  is genuinely truncating.
- **`CrossLink::TraceComponent` variant retained.** The old Tracer
  latest-events cross-link variant stays in `src/intent/mod.rs` with
  its handle_pure / dispatch arms intact. Phase 6 just removes the
  emission sites in Bulletins and Browser `t` handlers. Pruning the
  variant is deferred until no emission site remains.
- **Filter state survives tab switches.** When the user leaves the
  Events tab and returns, the filter bar's edit buffer is committed
  and the filters re-display on return. Query state (`Running`,
  `Done`) also persists. Consistent with Bulletins tab's filter
  preservation.
- **Worker poll timeout is a hard 60 s.** Queries that do not
  complete within 60 s emit `QueryFailed`. Longer windows would
  need the timeout raised via config — deferred.
- **`t` key collision resolved per-mode.** On the filter bar
  (Mode A), `t` opens the time-field editor. On a selected results
  row (Mode B), `t` emits the `TraceByUuid` cross-link. Mode A and
  Mode B are distinguished by whether `selected_row.is_some()`.
- **Events row `g` uses `OpenInBrowser`, not a Events-specific
  variant.** The spec says "g jump to Browser (ControlRate)". Since
  both `component_id` and `group_id` are available on
  `ProvenanceEventSummary`, the existing `CrossLink::OpenInBrowser`
  variant is a clean reuse.
- **Events row `t` switches tab via `TracerLineageStarted` reducer.**
  The `CrossLink::TraceByUuid` dispatcher arm spawns `spawn_lineage`
  and returns `IntentOutcome::TracerLineageStarted { uuid, abort }`.
  Its reducer arm sets `state.current_tab = ViewId::Tracer` and
  populates `state.tracer` via `start_lineage` — same path used by
  the existing Tracer entry-form submission, where the tab switch
  is a no-op because the user is already on Tracer.

## Dependency on `nifi-rust-client`

`nifi-lens` depends on `nifi-rust-client = "0.8.0"` with the `dynamic`
feature, declared in `Cargo.toml`:

```toml
nifi-rust-client = { version = "0.8.0", features = ["dynamic"] }
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

**Dependencies are kept alphabetically sorted** in `Cargo.toml`. New deps
land in the correct position, never appended at the bottom.

Phase 3 activates `arboard` 3 (clipboard support for the `c` keybind)
and `nucleo` 0.5 (fuzzy matcher powering `Ctrl+F`). Both were added
earlier in commit `021eadc build: add runtime and dev dependencies`
and remained unused until Phase 3.

Phase 4 activates `uuid` 1 (flowfile UUID parsing and validation in the
Tracer entry form) and promotes `serde_json` 1 from a dev-dependency to
a runtime dependency (JSON pretty-printing in the content preview pane).

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

**MSRV is `1.88`.** The `rust-toolchain.toml` pins `1.93.0` for development;
CI enforces the `1.88` floor via `RUSTUP_TOOLCHAIN` override. MSRV was
raised from 1.85 to 1.88 to pull in `time >= 0.3.47`, which fixes
[RUSTSEC-2026-0009](https://rustsec.org/advisories/RUSTSEC-2026-0009).

### Pointing the binary at a local NiFi cluster

Create `~/.config/nifilens/config.toml` as shown in `README.md`, export the
referenced `password_env` variable, and run `cargo run -- --context dev`.

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
marker PG (`nifilens-fixture-v1`) is already present, so iterating on
`nifi-lens` itself doesn't reset the fixture state.

The fixture is four process groups (`healthy-pipeline` with nested
`ingest` / `enrich` children, `noisy-pipeline`, `backpressure-pipeline`,
`invalid-pipeline`) plus three controller services, all under a top-level
marker PG named `nifilens-fixture-v1`. When the detected NiFi version is
>= 2.9.0, the seeder also creates `stress-pipeline` — a longer branching
flow with ConvertRecord (JSON to CSV), UpdateRecord (add computed fields,
transform existing), RouteOnAttribute (hot/normal split), and dual
ControlRate bottlenecks for sustained queue backpressure. The stress
pipeline adds four additional controller services (`stress-json-reader`,
`stress-csv-writer`, `stress-csv-reader`, `stress-csv-writer-out`).
Bumping the marker name invalidates stale fixtures automatically on the
next seed pass.

### Bumping the NiFi ceiling version

When `nifi-rust-client` adds support for a new NiFi version (e.g. 2.9.0):

1. Update `nifi-rust-client` in the root `Cargo.toml`.
2. Edit `integration-tests/versions.toml` — replace or append the new
   version.
3. Edit `integration-tests/docker-compose.yml` — add/replace the service
   block.
4. Edit `integration-tests/nifilens-config.toml` — add/replace the context.
5. Edit `tests/common/versions.rs` — add/replace the `port_for` match arm.
6. Run `./integration-tests/run.sh` locally to verify.
7. Push. CI's drift check enforces steps 2–4 consistency.

The **floor version 2.6.0 never drops** — it stays pinned forever so the
dynamic client is always tested against the oldest supported NiFi.

## Release

Releases are driven by [`cargo-release`](https://crates.io/crates/cargo-release)
via `release/release.sh`, which is a thin passthrough wrapper.

**`cargo-release` is dry-run by default.** Adding `--execute` performs the
release.

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
3. `cargo publish`es to crates.io using the `CARGO_REGISTRY_TOKEN` secret.
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
| Design specs and implementation plans | `docs/superpowers/specs/` and `docs/superpowers/plans/` locally | Markdown (gitignored) |

**Rules:**

- `docs/` is gitignored. Do **not** hard-link into it from any committed
  file. Specs and plans are private to the working copy.
- When architecture or patterns change, update `AGENTS.md` in the same
  commit.
- When user-visible behavior changes, update `README.md` and `CHANGELOG.md`
  in the same commit.
- Every new dependency goes into `Cargo.toml` in its correct alphabetical
  position.

## Phase Roadmap

`nifi-lens` ships in discrete phases, each leaving the tool in a runnable,
usable state.

1. **Phase 0 — Foundations** *(shipped)*. Config loader, `NifiClient`
   wrapper with `Deref`, ratatui + crossterm render loop, four empty tab
   placeholders, `Ctrl+K` context switcher, intent dispatcher stub,
   rotating-file logging with stderr toggle, panic-safe terminal guard,
   wiremock client tests, Docker-backed integration test harness.
2. **Phase 1 — Overview tab.** *(shipped)* Health dashboard: identity strip,
   component counts, bulletin-rate sparkline, unhealthy-queue leaderboard,
   top noisy components.
3. **Phase 2 — Bulletins tab.** *(shipped)* Cluster-wide bulletin tail
   with severity / component-type / free-text filters, auto-scroll
   pause with `+N new` badge, and cross-link stubs for Browser / Tracer
   jumps.
4. **Phase 3 — Browser tab.** *(shipped)* Two-pane PG tree + per-node
   detail view (Processor / Connection / ProcessGroup / Controller
   Service). 15-second recursive tree poll + on-demand detail fetches.
   Global `Ctrl+F` fuzzy find (nucleo-backed, lazy-seeded on first
   Browser entry). `Enter` on a bulletin row now lands on the matching
   component instead of the Phase 3 stub banner. Properties modal (`e`)
   for Processor and Controller Service nodes. Clipboard copy (`c`) of
   the selected node id.
5. **Phase 4 — Tracer tab.** *(shipped)* Paste a flowfile UUID → see
   its full lineage as a chronological event timeline → expand any event
   for the attribute diff and input/output content (text / JSON
   prettyprint / hex) with save-to-file. Bulletins `t` and Browser `t`
   cross-links land on a latest-provenance-events mini list. Full
   provenance search mode (`POST /provenance` with time-range /
   relationship filters) deferred to a future phase.
6. **Phase 5 — Cluster Health tab.** *(shipped)* Two-pane ops dashboard
   with four categories: queue backpressure leaderboard with server-
   predicted time-to-full, repository fill bars, per-node heap/GC/load
   strips, and processor thread leaderboard. Dual-cadence worker (10s
   PG status, 30s system diagnostics). `Enter` on queue/processor rows
   jumps to Browser.
7. **Phase 6 — Polish.** *(shipped)* Structural cleanup (`app/state.rs`
   split into per-view modules behind `ViewKeyHandler` trait,
   `ListNavigation` trait, generic worker polling, theme consolidation),
   Health tab edge cases (aggregate fallback, stalled queue display,
   cross-link hints, per-node repository breakdown), help modal cleanup.
8. **UI Improvements — Navigation polish.** *(shipped)* Tab history
   (`Alt+Left`/`Alt+Right`) for cross-link back/forward with selection
   restore, interactive breadcrumb bar in Browser detail pane, context-
   sensitive sticky footer hint line.
9. **UI Readability Improvements — Information density.** *(shipped)*
   Browser per-processor run-state icons, Health Load spark-bar gauge,
   Bulletins consecutive-source grouping (`Shift+B`), configurable
   timestamp format via new `[ui]` config section
   (`timestamp_format` / `timestamp_tz`), theme audit pass replacing
   inline `Color::*`/`Modifier::*` constructors with `theme::*` helpers.
10. **UI Reorg Phase 1 — Chrome refactor.** *(shipped)* One-row top bar
    (tabs + right-aligned compact identity strip `[ctx] v2.9.0 · nodes N/M`)
    replacing the old bordered `" nifi-lens "` tab box. Rewritten
    `status_bar` widget renders a severity-colored banner on the left and
    a right-aligned refresh age (`⟳ Ns ago`) on the right. New
    `crate::test_support` helper module (`fresh_state`, `tiny_config`)
    for widget-level tests outside `app::state`. `AppState.cluster_summary`
    added as a `Option`-valued placeholder; the Overview worker populates
    it in UI Reorg Phase 3. Added the `Events` `ViewId` with a
    "coming in Phase 6" bordered placeholder so the tab bar has its final
    shape. Health tab still exists until UI Reorg Phase 3 removes it.
11. **UI Reorg Phase 2 — Keybinding rename sweep.** *(shipped)* Global
    `Ctrl+F` → `f` (fuzzy find), `Ctrl+K` → `K` (context switcher),
    `Alt+←` / `Alt+→` → `[` / `]` (tab history back/forward). Bulletins
    `Shift+B` relabeled `B` in user-facing text (the handler already
    matched `KeyCode::Char('B')`). `Ctrl+C` / `Ctrl+Q` quit stay
    (universal terminal convention). `Ctrl+U` clear-line in Tracer entry
    and `Ctrl+N` / `Ctrl+P` list-nav in the fuzzy-find modal stay
    (emacs text-input conventions). Established rule: bare lowercase
    for view-local, bare capital for app-wide, no Ctrl chords except
    `Ctrl+C` or text-input helpers.
12. **UI Reorg Phase 3 — Overview merge.** *(shipped)* Health tab
    deleted entirely; its node, repository, queue-pressure, and
    sysdiag data merged into Overview as the new Layout 3 dashboard
    (processor info line, nodes hero zone, bulletins+noisy split,
    unhealthy queues full-width). Overview worker rewritten for
    dual cadence (10s PG status, 30s system diagnostics with
    nodewise → aggregate fallback). `AppState.cluster_summary` now
    populated from the SystemDiag payload — top-bar identity strip
    shows real `nodes N/M` instead of the Phase 1 `?/?` placeholder.
    `ViewPayload::Health`, `HealthPayload`, `ViewId::Health`, the
    `view/health/` directory, `app/state/health.rs`, and the F3 →
    Health binding all removed. F-keys remap to F1..F5 =
    Overview/Bulletins/Browser/Events/Tracer.
13. **UI Reorg Phase 4 — Bulletins redesign.** *(shipped)* Rewrote
    the Bulletins tab as Layout L. The reducer now strips NiFi's
    `ComponentName[id=<uuid>]` prefix and dedups by
    `(source_id, message_stem)`; a new `GroupMode` enum replaces
    the old consecutive-group toggle and cycles
    `source+msg` / `source` / `off` via the `g` key. New `m`
    mutes the selected row's `source_id` session-scoped. Severity
    chips now carry ring counts (`[E 87] [W 32] [I 0]`). List
    columns are `time / sev / # / source / pg path / message`,
    with the PG path resolved via a new
    `BrowserState::pg_path` helper (falls back to `…tail8` when
    the Browser tree has not yet been indexed). Detail pane is
    multi-line full-width with source, pg path, count, first-seen,
    last-seen, raw message, source id, pg id, and per-row action
    hints. The old `B` consecutive-group toggle and `g` / `G`
    vim-jump bindings were removed (`Home` / `End` still work).
    Bulletins `t` keeps its current Tracer cross-link; retargeting
    to the Events tab is deferred to Phase 6.
14. **UI Reorg Phase 5 — Browser declutter & detail enrichment.**
    *(shipped)* Tree rows drop trailing status summaries
    (`● 5 ○ 2 ⚠ 0 ⌀ 1`, connection fill, CS state) in favor of
    richly labeled per-kind detail panes. PG tree markers gain a
    rolled-up health color (`BrowserState::pg_health_rollup`) —
    any descendant processor `INVALID` → red, `STOPPED` → yellow,
    else green. PG detail grows labeled sections for processors /
    threads / queued / controller services / child groups /
    recent bulletins / action hints. Connection detail gains a
    prominent fill gauge (via the existing `widget::gauge::fill_bar`
    helper) with color-by-percent. Controller service detail gains
    a state chip at the top. Processor detail adds a "Recent
    bulletins (N for this processor)" section. The Browser render
    signature is widened to accept `&VecDeque<BulletinSnapshot>`;
    the Phase 3 "PG-scoped recent bulletins show 0" edge case is
    resolved. Two new free helpers in
    `view::bulletins::state` (`recent_for_source_id` /
    `recent_for_group_id`) filter the ring without cloning.
    `BrowserState` gains `PgHealth`, `pg_health_rollup`, and
    `child_process_groups`.
15. **UI Reorg Phase 6 — Events tab.** *(shipped)* New cluster-wide
    provenance-search tab with a 2-row filter bar (`t time` / `T type`
    / `s source` / `u file uuid` / `a attr`), a results list colored
    by event type (DROP/EXPIRE red+bold, ROUTE accent, RECEIVE/SEND
    /FETCH/DOWNLOAD green, FORK/JOIN/CLONE muted), and a detail pane
    for the selected row. A new `src/client/events.rs` wraps NiFi's
    `POST /provenance` + `GET /provenance/{id}` + `DELETE
    /provenance/{id}` endpoints; the worker
    (`src/view/events/worker.rs`) submits, polls at 750 ms until the
    server reports `finished = true`, and best-effort-deletes the
    server-side query. The reducer walks
    `EventsQueryStatus { Idle / Running / Done / Failed }` with
    query-id matching to drop late payloads from cancelled queries.
    Mode A (filter-bar nav) vs Mode B (row nav) keep the letter-key
    bindings unambiguous — `t` on the filter bar opens the time-field
    editor; `t` on a selected row emits the `CrossLink::TraceByUuid`
    cross-link. Cross-links: Bulletins `t` and Browser `t` now land
    on Events pre-filled with `source = component` and
    `time = last 15m`, auto-running the query. Events row `t` →
    Tracer lineage via `CrossLink::TraceByUuid`. Events row `g` →
    Browser via the existing `OpenInBrowser` cross-link. Scope cuts:
    tab-completion on `T type`, attribute regex, realtime follow,
    saved queries, per-node scope, bulk export, parent/child uuid
    display — all tracked as edge cases.
16. **Phase 7 — Write-path scaffolding.** Dry-run mode, confirmation modal
    primitive, audit log, `--allow-writes` flag. No writes enabled yet —
    this just lays the rails for v2.

Each phase has its own design spec and implementation plan under
`docs/superpowers/`, local-only.

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
