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
- **Health overview** — unhealthy queue leaderboard, component counts,
  bulletin rate sparkline, top noisy components.
- **Flow browser** — walk the process-group tree, see every detail of a
  selected processor / connection / controller service on one screen,
  jump between related components.
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

**Bulletins** — "What's going wrong?" Live cluster-wide bulletin tail with
severity / component / free-text filters, auto-scroll with pause, and
cross-links to Browser and Tracer.

**Browser** — "Where does X live and what is it doing?" Process-group tree
with drill-in, per-node detail pane, and global fuzzy find across all known
components via [`nucleo`](https://crates.io/crates/nucleo).

**Tracer** — "Why did this flowfile fail?" Forensic provenance view: paste a
UUID or component-scoped query, walk the event timeline, see attribute diffs
per event, preview input and output content on demand (text or hex).

## Keybindings

Short global reference; full per-view help is available with `?` inside the
tool.

| Key | Action |
|---|---|
| `Tab` / `Shift+Tab` | Cycle tabs |
| `F1`–`F4` | Jump to tab directly |
| `Ctrl+K` | Switch cluster context |
| `Ctrl+F` | Global component fuzzy find |
| `?` | Context-aware help modal |
| `q` / `Ctrl+Q` | Quit |

## Configuration

Config file lives at `~/.config/nifilens/config.toml` and is kubeconfig-style:

```toml
current_context = "dev"

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

## License

Apache-2.0. See [`LICENSE`](LICENSE).
