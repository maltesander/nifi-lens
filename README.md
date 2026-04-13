# nifi-lens

> A keyboard-driven TUI lens into Apache NiFi 2.x. Browse flows,
> trace flowfiles, tail bulletins, and debug across clusters and versions.

[![CI](https://github.com/maltesander/nifi-lens/actions/workflows/ci.yml/badge.svg)](https://github.com/maltesander/nifi-lens/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/nifi-lens.svg)](https://crates.io/crates/nifi-lens)
[![Docs.rs](https://docs.rs/nifi-lens/badge.svg)](https://docs.rs/nifi-lens)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](LICENSE)
![MSRV: 1.88](https://img.shields.io/badge/MSRV-1.88-blue.svg)

## Status

Pre-release. The tool is being built in phases; see the roadmap in
[`AGENTS.md`](AGENTS.md#phase-roadmap).

## Screencasts

*Coming in v0.1.0.* This section is intentionally reserved.

## Features

- **Forensic flowfile tracing** — paste a UUID, get the full provenance
  lineage with attribute diffs and on-demand content previews.
- **Cluster-wide bulletin tail** — live, filterable, with auto-scroll pause
  and severity / component / free-text filters.
- **Cluster health dashboard** *(shipped)* — two-pane ops view with queue
  backpressure leaderboard, repository fill bars, per-node heap/GC/load
  strips, and processor thread leaderboard. Dual-cadence refresh (10 s
  PG status, 30 s system diagnostics). `Enter` jumps to Browser.
- **Flow browser** *(shipped)* — two-pane PG tree + per-node detail
  (Processor / Connection / ProcessGroup / Controller Service);
  `f` fuzzy find across all known components; `e` for a full
  properties modal; `c` to copy a node id to the clipboard.
- **Multi-cluster, multi-version** — kubeconfig-style contexts; one binary
  works against every supported NiFi 2.x version via
  [`nifi-rust-client`](https://docs.rs/nifi-rust-client)'s `dynamic` feature.
- **Read-only and safe by construction** — v1 never mutates cluster state.

## Install

Once published to crates.io:

```bash
cargo install nifi-lens
```

From source:

```bash
git clone https://github.com/maltesander/nifi-lens
cd nifi-lens
cargo install --path .
```

## Quick Start

Create `~/.config/nifilens/config.toml`:

```toml
current_context = "dev"

[[contexts]]
name = "dev"
url = "https://nifi-dev.internal:8443"
version_strategy = "closest"   # strict | closest | latest
insecure_tls = false

[contexts.auth]
type = "password"
username = "admin"
password_env = "NIFILENS_DEV_PASSWORD"
```

Then:

```bash
export NIFILENS_DEV_PASSWORD=...
nifilens
```

Press `?` inside the tool for a context-aware help modal. A context-sensitive
hint line at the bottom of the screen shows relevant keybindings for the
current view and mode.

## Core Components

`nifilens` has five top-level tabs, each optimized for a specific
operational question.

**Overview** — "Is this cluster OK right now?" Cluster identity, component
counts, bulletin-rate sparkline, queue backpressure metrics, repository fill
status, per-node health strips, and noisiest components. Dual-cadence refresh
(PG status every 10 s, system diagnostics every 30 s).

**Bulletins** *(shipped)* — cluster-wide bulletin tail with severity,
component-type, and free-text filters; auto-scroll pause with a new-
bulletin badge; `Enter` on a row jumps directly to the component in
the Browser tab.

**Browser** *(shipped)* — "Where does X live and what is it doing?"
Two-pane PG tree with drill-in, per-node detail pane, and global
`f` fuzzy find across all known components via
[`nucleo`](https://crates.io/crates/nucleo). Press `e` for a full
properties modal on Processor / Controller Service nodes; `c` to copy
the selected node's id to the clipboard.

**Tracer** *(shipped)* — "Why did this flowfile fail?" Paste a flowfile UUID
to trace its full lineage as a chronological event timeline. Expand any event
to see the attribute diff (All / Changed toggle) and fetch input or output
content on demand (text, JSON prettyprint, or hex dump for binary). Press `s`
to save the raw content bytes to a file. Cross-links from the Bulletins tab
(`t`) and Browser tab (`t`) land on a latest-provenance-events mini list for
the selected component.

### Browser tab

Two-pane view: PG tree on the left, per-node detail on the right.
Selection fires an on-demand detail fetch (15 s cadence for the tree,
on-select for detail). Press `e` on a processor or controller service
to pop the full properties list in a modal. Press `c` to copy the
selected node's id to the clipboard. Press `t` on a processor to jump
to the Tracer tab and see its latest provenance events.

### Tracer tab

Forensic flowfile investigation in four modes:

- **Entry** — type or paste a flowfile UUID into the input bar and press
  `Enter` to start a lineage query.
- **Lineage running** — a progress bar shows the NiFi server's completion
  percentage while the query is in flight.
- **Lineage** — chronological event timeline. Navigate with `↑`/`↓` / `j`/`k`.
  Press `Enter` or `Space` to expand an event into the detail pane.
  - **Detail pane**: attribute diff table with `d` to toggle All / Changed
    view. Press `i` or `o` to fetch input or output content respectively.
  - **Content pane**: text rendered as-is, JSON pretty-printed
    automatically, binary shown as a hex dump. Press `s` to save the raw
    bytes to a file; press `Esc` to dismiss.
- **Latest events** — when arriving via a cross-link (`t` in Bulletins or
  Browser), the Tracer shows the 20 most recent provenance events for the
  linked component. Press `Enter` on any event row to open it as a lineage
  query.

## Keybindings

Short global reference; full per-view help is available with `?` inside the
tool.

| Key | Action |
|---|---|
| `Tab` / `Shift+Tab` | Cycle tabs |
| `F1` | Jump to Overview |
| `F2` | Jump to Bulletins |
| `F3` | Jump to Browser |
| `F4` | Jump to Events |
| `F5` | Jump to Tracer |
| `K` | Switch cluster context |
| `f` | Global component fuzzy find (available once the Browser tab has loaded at least once to seed the index) |
| `[` / `]` | Cross-link back / forward |
| `?` | Context-aware help modal |
| `q` / `Ctrl+Q` | Quit |
| `b` (Browser) | Enter breadcrumb navigation |

## Configuration

Config file lives at `~/.config/nifilens/config.toml` and is kubeconfig-style:

```toml
current_context = "dev"

# Optional: Bulletins tab ring buffer size. Default 5000; valid range
# 100..=100000. Larger values keep more history at the cost of memory.
[bulletins]
ring_size = 5000

# Optional: UI rendering options. All fields are optional; the defaults
# below match what the tool uses if you omit the section.
[ui]
# Timestamp display format in Bulletins and Tracer:
#   "short"  — HH:MM:SS for today, "MMM DD HH:MM:SS" for older events
#   "iso"    — 2026-04-12T14:32:18Z (or ...+02:00 with local tz)
#   "human"  — Apr 12 14:32:18
timestamp_format = "short"

# "utc" or "local". "local" uses the host machine's time zone.
timestamp_tz = "utc"

[[contexts]]
name = "dev"
url = "https://nifi-dev.internal:8443"
version_strategy = "closest"   # strict | closest | latest
insecure_tls = false

[contexts.auth]
type = "password"              # password | token | mtls
username = "admin"
password_env = "NIFILENS_DEV_PASSWORD"

[[contexts]]
name = "prod"
url = "https://nifi-prod.internal:8443"
version_strategy = "strict"

[contexts.auth]
type = "password"
username = "operator"
password_env = "NIFILENS_PROD_PASSWORD"
```

- **Credentials** are configured in the `[contexts.auth]` sub-table. Three
  types are supported:

  | Type | Fields | Notes |
  |------|--------|-------|
  | `password` | `username`, `password_env` or `password` | `password_env` preferred; `password` emits a warning |
  | `token` | `token_env` or `token` | Pre-obtained JWT; `token_env` preferred |
  | `mtls` | `client_identity_path` | PEM containing private key + cert chain |

  Any context can optionally include `proxied_entities_chain = "<user1><user2>"`
  for NiFi proxy deployments.
- **File permissions** must be `0600`; `nifilens` refuses to start if the
  config is world-readable.
- **CLI overrides:** `nifilens --context stage`, `nifilens --config ./local.toml`.
- **Version strategy** maps to `nifi-rust-client`'s `VersionResolutionStrategy`.

## Development

See [`AGENTS.md`](AGENTS.md) for architecture, build / test / release
procedures, and contributor conventions.

### Running the integration fixture locally

`nifi-lens` ships with a Docker-based integration fixture that brings up
two NiFi versions simultaneously and pre-seeds them with a realistic flow
— running pipelines, a back-pressured queue, multi-severity bulletins,
nested process groups, and a handful of controller services. Use it to
test `nifi-lens` against live clusters without touching production.

```bash
./integration-tests/run.sh
```

This boots `apache/nifi:2.6.0` (standalone, port 8443) and a 2-node
`apache/nifi:2.9.0` cluster (ports 8444-8445) with ZooKeeper, seeds both
via the `nifilens-fixture-seeder` workspace binary, runs the
`#[ignore]`-gated integration suite, then tears the containers down.

For long-running live testing, skip the test step and leave the fixture
up:

```bash
docker compose -f integration-tests/docker-compose.yml up -d
export NIFILENS_IT_PASSWORD=adminpassword123
cargo run -p nifilens-fixture-seeder -- \
    --config integration-tests/nifilens-config.toml \
    --context dev-nifi-2-9-0
cargo run -- --config integration-tests/nifilens-config.toml \
    --context dev-nifi-2-9-0
```

The seeder supports `--skip-if-seeded` for idempotent re-runs during
iteration.

## License

Apache-2.0. See [`LICENSE`](LICENSE).
