# nifi-lens

> A keyboard-driven TUI lens into Apache NiFi 2.x. Browse flows,
> trace flowfiles, tail bulletins, and debug across clusters and versions.

[![CI](https://github.com/maltesander/nifi-lens/actions/workflows/ci.yml/badge.svg)](https://github.com/maltesander/nifi-lens/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/nifi-lens.svg)](https://crates.io/crates/nifi-lens)
[![Docs.rs](https://docs.rs/nifi-lens/badge.svg)](https://docs.rs/nifi-lens)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](LICENSE)
![MSRV: 1.88](https://img.shields.io/badge/MSRV-1.88-blue.svg)

## Screenshots

**Overview** — Cluster health at a glance: "Is this cluster OK right now?"

![Overview tab](assets/screenshots/overview.png)

**Bulletins** — Live cluster-wide bulletin tail: "What is the cluster complaining about?"

![Bulletins tab](assets/screenshots/bulletins.png)

**Browser** — Flow tree with per-node detail: "Where does X live and what is it doing?"

![Browser tab](assets/screenshots/browser.png)

**Events** — Provenance search and detail: "What just happened across the cluster?"

![Events tab](assets/screenshots/events.png)

**Tracer** — Flowfile lineage with attribute diff: "Why did this flowfile fail?"

![Tracer — attribute diff](assets/screenshots/tracer_attributes.png)

**Tracer** — Content preview (Input / Output tabs): "Why did this flowfile fail?"

![Tracer — content preview](assets/screenshots/tracer_content.png)

## Features

- **Cluster overview** — health dashboard with bulletin-rate sparkline, queue backpressure, per-node heap/GC, and noisiest components.
- **Bulletin tail** — live cluster-wide log with severity filters, source deduplication, and per-source mute.
- **Flow browser** — component tree with per-node detail; `Shift+F` fuzzy search across all known components.
- **Provenance events** — filterable cluster-wide event search cross-linked from Bulletins and Browser.
- **Flowfile tracer** — paste a UUID to trace its full lineage with attribute diffs and content previews (text, JSON, hex).
- **Multi-cluster** — kubeconfig-style contexts; `Shift+K` to switch clusters; one binary for every NiFi 2.x version.
- **Read-only** — v0.1 never mutates cluster state.

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

Press `?` inside the tool for a context-aware help modal. A hint line at
the bottom shows relevant keybindings for the current view.

## Core Components

Five top-level tabs, each targeting a specific operational question.

**Overview** — Cluster health at a glance: component counts, bulletin-rate
sparkline, queue backpressure, repository fill, per-node health strips, and
noisiest components.

**Bulletins** — Live cluster-wide bulletin tail. Severity, component-type,
and free-text filters; deduplication collapses repeating errors into a single
row with an `×N` count. `Enter` on a row jumps to the component in Browser.

**Browser** — Two-pane PG tree with per-node detail. `p` opens a properties
modal; `c` copies the node id; `g` opens the cross-tab jump menu.

**Events** — Provenance search with a filter bar (time / type / source /
flowfile UUID / attribute). Results are colored by event type and
cross-linked from Bulletins and Browser.

**Tracer** — Paste a flowfile UUID to trace its full lineage. Expand any
event for a tabbed detail pane (Attributes | Input | Output); `d` toggles
the attribute All / Changed diff; `s` saves raw content to a file.

## Keybindings

The tool is largely self-explanatory — `?` opens context-aware help and the
hint bar at the bottom always shows what's available. A few highlights:

**Navigation** — `↑`/`↓` rows, `Tab`/`Shift+Tab` between panes,
`F1`–`F5` jump to tabs, `g` opens the cross-tab goto menu, `Shift+K`
switches the active cluster context, `Shift+F` opens global fuzzy search,
`q`/`Ctrl+C` to quit.

**Bulletins** — `1`/`2`/`3` toggle severity filters; `Shift+G` cycles
group-by modes; `Shift+P` pauses auto-scroll; `Shift+M` mutes a source.

**Browser** — `p` for properties modal; `c` to copy; `Shift+F` for
global fuzzy search.

**Events** — `Shift+D`/`T`/`S`/`U`/`A` to edit filters; `Enter` in
the filter bar to submit.

**Tracer** — `←`/`→` cycle detail tabs; `d` toggles attribute diff;
`s` saves content.

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
# ca_cert_path = "/etc/nifi-lens/certs/dev-ca.crt"   # optional extra CA cert (PEM)
# proxy_url       = "http://proxy.internal:3128"      # all traffic through this proxy
# http_proxy_url  = "http://proxy.internal:3128"      # HTTP traffic only
# https_proxy_url = "http://proxy.internal:3128"      # HTTPS traffic only

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
