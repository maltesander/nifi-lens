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
- **Health overview** *(shipped)* — cluster identity, component counts,
  15-minute bulletin-rate sparkline, unhealthy-queue leaderboard, and
  top noisy components on one screen, refreshed every 10 seconds.
- **Flow browser** *(shipped)* — two-pane PG tree + per-node detail
  (Processor / Connection / ProcessGroup / Controller Service);
  `Ctrl+F` fuzzy find across all known components; `e` for a full
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
username = "admin"
password_env = "NIFILENS_DEV_PASSWORD"
version_strategy = "closest"
insecure_tls = false
```

Then:

```bash
export NIFILENS_DEV_PASSWORD=...
nifilens
```

Press `?` inside the tool for a context-aware help modal.

## Core Components

`nifilens` has four top-level tabs, each optimized for a specific
operational question.

**Overview** — "Is this cluster OK right now?" Cluster identity, component
counts, bulletin-rate sparkline, top-10 unhealthy queues, noisiest components.
Refreshes every 10 seconds.

**Bulletins** *(shipped)* — cluster-wide bulletin tail with severity,
component-type, and free-text filters; auto-scroll pause with a new-
bulletin badge; `Enter` on a row jumps directly to the component in
the Browser tab.

**Browser** *(shipped)* — "Where does X live and what is it doing?"
Two-pane PG tree with drill-in, per-node detail pane, and global
`Ctrl+F` fuzzy find across all known components via
[`nucleo`](https://crates.io/crates/nucleo). Press `e` for a full
properties modal on Processor / Controller Service nodes; `c` to copy
the selected node's id to the clipboard.

**Tracer** — "Why did this flowfile fail?" Forensic provenance view: paste a
UUID or component-scoped query, walk the event timeline, see attribute diffs
per event, preview input and output content on demand (text or hex).

### Browser tab

Two-pane view: PG tree on the left, per-node detail on the right.
Selection fires an on-demand detail fetch (15 s cadence for the tree,
on-select for detail). Press `e` on a processor or controller service
to pop the full properties list in a modal. Press `c` to copy the
selected node's id to the clipboard. Press `t` on a processor for a
Tracer cross-link (Phase 4 target — still a stub banner).

## Keybindings

Short global reference; full per-view help is available with `?` inside the
tool.

| Key | Action |
|---|---|
| `Tab` / `Shift+Tab` | Cycle tabs |
| `F1`–`F4` | Jump to tab directly |
| `Ctrl+K` | Switch cluster context |
| `Ctrl+F` | Global component fuzzy find (available once the Browser tab has loaded at least once to seed the index) |
| `?` | Context-aware help modal |
| `q` / `Ctrl+Q` | Quit |

## Configuration

Config file lives at `~/.config/nifilens/config.toml` and is kubeconfig-style:

```toml
current_context = "dev"

# Optional: Bulletins tab ring buffer size. Default 5000; valid range
# 100..=100000. Larger values keep more history at the cost of memory.
[bulletins]
ring_size = 5000

[[contexts]]
name = "dev"
url = "https://nifi-dev.internal:8443"
username = "admin"
password_env = "NIFILENS_DEV_PASSWORD"
version_strategy = "closest"   # strict | closest | latest
insecure_tls = false

[[contexts]]
name = "prod"
url = "https://nifi-prod.internal:8443"
username = "operator"
password_env = "NIFILENS_PROD_PASSWORD"
version_strategy = "strict"
```

- **Credentials** primarily via `password_env` → environment variable.
  Plaintext `password = "..."` is supported but emits a warning at load.
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

This boots `apache/nifi:2.6.0` (port 8443) and `apache/nifi:2.8.0` (port
8444), seeds both via the `nifilens-fixture-seeder` workspace binary,
runs the `#[ignore]`-gated integration suite, then tears the containers
down.

For long-running live testing, skip the test step and leave the fixture
up:

```bash
docker compose -f integration-tests/docker-compose.yml up -d
export NIFILENS_IT_PASSWORD=adminpassword123
cargo run -p nifilens-fixture-seeder -- \
    --config integration-tests/nifilens-config.toml \
    --context dev-nifi-2-8-0
cargo run -- --config integration-tests/nifilens-config.toml \
    --context dev-nifi-2-8-0
```

The seeder supports `--skip-if-seeded` for idempotent re-runs during
iteration.

## License

Apache-2.0. See [`LICENSE`](LICENSE).
