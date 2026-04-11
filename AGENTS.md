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
    ├── view/               # per-tab placeholder renderers
    └── widget/             # status_bar, help_modal, context_switcher
```

Phase 0 (see [Phase Roadmap](#phase-roadmap)) will grow the `src/` tree into
modules for `app`, `config`, `client`, `intent`, `event`, `view`, `widget`,
`fuzzy`, `theme`, and `util`.

## Architecture

`nifi-lens` follows a standard "ratatui + tokio" split:

- **Single `tokio` multi-thread runtime** owns everything.
- **UI loop** runs on the main task. It drains an internal `AppEvent`
  channel, mutates state, and redraws (60 fps cap, only when state changed).
- **Terminal event task** converts `crossterm::Event` → `AppEvent::Input`.
- **Per-view data worker tasks** poll the relevant `NifiClient` endpoints
  and push `AppEvent::Data(view, payload)` into the channel. Workers are
  spawned on tab activation and cancelled on tab switch, so API load is
  proportional to what the user is actually looking at.
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

## Dependency on `nifi-rust-client`

`nifi-lens` depends on `nifi-rust-client = "0.5.0"` with the `dynamic`
feature, declared in `Cargo.toml`:

```toml
nifi-rust-client = { version = "0.5.0", features = ["dynamic"] }
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
2. **Phase 1 — Overview tab.** Health dashboard: identity strip,
   component counts, bulletin-rate sparkline, unhealthy-queue leaderboard,
   top noisy components.
3. **Phase 2 — Bulletins tab.** Cluster-wide bulletin tail with
   severity / component / free-text filters and auto-scroll pause.
4. **Phase 3 — Browser tab.** Process-group tree, per-node detail, global
   fuzzy find, cross-links from bulletins.
5. **Phase 4 — Tracer tab.** Forensic flowfile investigation: provenance
   search, lineage timeline, attribute diffs, on-demand content preview.
6. **Phase 5 — Polish and first release.** Help modal filled out, error
   surfacing reviewed, screencasts, `v0.1.0` to crates.io.
7. **Phase 6 — Write-path scaffolding.** Dry-run mode, confirmation modal
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
