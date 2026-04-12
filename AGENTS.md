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
    ├── app/                # mod (run loop), state (reducer), ui (frame render)
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
- **PG-scoped recent bulletins show 0.** The PG detail pane renders
  a `"Recent bulletins (0 in this PG)"` header but the actual ring
  filter is not threaded yet (requires access to `AppState.bulletins`
  from the render path). Phase 5 polish item.
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

- **Node diagnostics show aggregate only when nodewise fetch fails.**
  If NiFi returns an error for the per-node system-diagnostics call,
  the Nodes pane falls back to showing no rows. A graceful aggregate
  fallback is a Phase 6 polish item.
- **Time-to-full estimate is zero when throughput is zero.** Connections
  with a back-pressured queue but zero recent throughput show
  `TimeToFull::Unknown` rather than infinity. The distinction is not
  surfaced to the user. Phase 6 polish item.
- **Health tab `Enter` cross-link is a stub for non-queue/processor rows.**
  Pressing `Enter` on a repository or node row emits no cross-link intent;
  the keybind is silently ignored. Phase 6 polish item.

## Dependency on `nifi-rust-client`

`nifi-lens` depends on `nifi-rust-client = "0.7.0"` with the `dynamic`
feature, declared in `Cargo.toml`:

```toml
nifi-rust-client = { version = "0.7.0", features = ["dynamic"] }
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

The integration harness at `integration-tests/` brings up two simultaneous
NiFi containers (2.6.0 floor + 2.8.0 ceiling), seeds each with a rich
fixture via `nifilens-fixture-seeder`, and runs `cargo test --test
'integration_*' -- --ignored` against both. One command does it all:

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
    --context dev-nifi-2-8-0 --skip-if-seeded
cargo run -- --config integration-tests/nifilens-config.toml \
    --context dev-nifi-2-8-0
```

`--skip-if-seeded` makes re-runs of the seeder a no-op when the fixture
marker PG (`nifilens-fixture-v1`) is already present, so iterating on
`nifi-lens` itself doesn't reset the fixture state.

The fixture is four process groups (`healthy-pipeline` with nested
`ingest` / `enrich` children, `noisy-pipeline`, `backpressure-pipeline`,
`invalid-pipeline`) plus three controller services, all under a top-level
marker PG named `nifilens-fixture-v1`. Bumping the marker name
invalidates stale fixtures automatically on the next seed pass.

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
7. **Phase 6 — Polish and first release.** Help modal filled out, error
   surfacing reviewed, screencasts, `v0.1.0` to crates.io.
8. **Phase 7 — Write-path scaffolding.** Dry-run mode, confirmation modal
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
