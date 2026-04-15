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

- **Cluster overview dashboard** — cluster identity, component counts,
  bulletin-rate sparkline, per-node heap/GC/load strips, repository fill
  bars, unhealthy-queue leaderboard, and noisiest components. Dual-cadence
  refresh (10 s PG status, 30 s system diagnostics).
- **Cluster-wide bulletin tail** — live, filterable, with auto-scroll
  pause, severity / component / free-text filters, and source-based
  deduplication so repeating errors collapse into a single row with an
  `×N` count column.
- **Flow browser** — two-pane PG tree + per-node detail
  (Processor / Connection / ProcessGroup / Controller Service). Global
  `Shift+F` fuzzy find across all known components, `p` for a full properties
  modal, `c` to copy a node id to the clipboard.
- **Cluster-wide provenance events** — pre-filtered provenance search
  (time / type / source / uuid / attribute) with results colored by
  event type; cross-linked from Bulletins and Browser.
- **Forensic flowfile tracing** — paste a UUID, get the full provenance
  lineage with attribute diffs and on-demand content previews
  (text, JSON prettyprint, or hex dump).
- **Multi-cluster, multi-version** — kubeconfig-style contexts; one
  binary works against every supported NiFi 2.x version via
  [`nifi-rust-client`](https://docs.rs/nifi-rust-client)'s `dynamic`
  feature.
- **Read-only and safe by construction** — v0.1 never mutates cluster
  state.

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

**Bulletins** — "What is the cluster complaining about?" Cluster-wide
bulletin tail with severity, component-type, and free-text filters;
auto-scroll pause with a new-bulletin badge; `Enter` on a row jumps
directly to the component in the Browser tab (Rule 1a Enter-fallback).
Rows are deduplicated by `(source_id, message_stem)` — repeating
errors collapse into a single row with an `×N` count column. `1`/`2`/`3`
toggle error/warning/info severity chips, `Shift+G` cycles group-by modes
(`source+msg` / `source` / `off`), `Shift+P` pauses auto-scroll, `Shift+M` mutes
the selected row's source for the session, and severity chips carry
live ring counts (`[E 87] [W 32] [I 0]`).

**Browser** — "Where does X live and what is it doing?" Two-pane PG
tree with drill-in, per-node detail pane, and global `Shift+F` fuzzy find
across all known components via
[`nucleo`](https://crates.io/crates/nucleo). Press `p` for a full
properties modal on Processor / Controller Service nodes; `c` to copy
the selected node's id to the clipboard.

**Events** — "What just happened across the cluster?" Provenance
search with a 2-row filter bar (time / type / source / flowfile uuid /
attribute), results list colored by event type, and a detail pane for
the selected event. Cross-linked from Bulletins and Browser via the
`g` jump menu, pre-filtered to the source component and the last 15 minutes.

**Tracer** — "Why did this flowfile fail?" Paste a flowfile UUID to
trace its full lineage as a chronological event timeline. Expand any
event to see a tabbed detail pane (Attributes | Input | Output) with
`←`/`→` to cycle tabs, `d` to toggle the attribute All / Changed diff,
and on-demand content fetching for the Input / Output tabs (text, JSON
prettyprint, or hex dump for binary). Press `s` to save the raw
content bytes to a file.

### Browser tab

Two-pane view: PG tree on the left, per-node detail on the right.
Selection fires an on-demand detail fetch (15 s cadence for the tree,
on-select for detail). Press `p` on a processor or controller service
to pop the full properties list in a modal. Press `c` to copy the
selected node's id to the clipboard. Press `g` on a processor to
open the jump menu and navigate to its Events or Tracer view.

**Tree navigation:** `↑`/`↓` move the cursor; `Enter` drills into a
process group or focuses the Detail panel on a leaf; `Esc` collapses
the current node or pops focus up one level; `→` expands, `←` collapses.

**Detail focus (Processor / Controller Service only):** press `Enter`
to move from the tree into the detail panel.

| Key | Action |
|---|---|
| `→` / `←` | Cycle to the next / previous focusable section |
| `↑` / `↓` | Scroll rows within the focused section |
| `Enter` | Drill into a child group (when focused on the ChildGroups section) |
| `Esc` | Return focus to the tree |
| `c` | Copy focused row's property value or bulletin message |

### Tracer tab

Forensic flowfile investigation:

- **Entry** — type or paste a flowfile UUID into the input bar and
  press `Enter` to start a lineage query. Cross-links from the Events
  jump menu populate the UUID automatically.
- **Lineage running** — a progress bar shows the NiFi server's
  completion percentage while the query is in flight.
- **Lineage** — chronological event timeline. Navigate with `↑`/`↓`.
  Press `Enter` to load an event's detail into the tabbed detail pane.
  - **Detail pane** — three sibling tabs: `Attributes` | `Input` |
    `Output`. Cycle with `←` / `→` (disabled tabs without a content
    claim are skipped). Scroll rows within the active tab with
    `↑`/`↓`, page with `PgUp`/`PgDn`, jump with `Home`/`End`.
  - **Attributes tab**: `d` toggles the All / Changed diff view.
  - **Input / Output tabs**: text rendered as-is, JSON pretty-printed
    automatically, binary shown as a hex dump. Press `s` to save the
    raw bytes to a file.
  - Press `Esc` to return from the detail pane to the timeline.

## Keybindings

Short global reference; full per-view help is available with `?` inside the
tool.

### Global

| Key | Action |
|---|---|
| `↑` / `↓` / `←` / `→` | Navigate (up/down rows, left/right peers) |
| `PgUp` / `PgDn` | Page up / down |
| `Home` / `End` | Jump to first / last |
| `Enter` | Drill / activate / submit |
| `Esc` | Leave focused pane / cancel pending input |
| `Shift+←` / `Shift+→` | History back / forward |
| `Tab` / `Shift+Tab` | Focus next / prev pane |
| `F1`..`F5` | Jump to tab 1..5 (Overview / Bulletins / Browser / Events / Tracer) |
| `?` | Context-aware help modal |
| `K` | Switch cluster context |
| `Shift+F` | Global component fuzzy find (available once Browser has loaded once to seed the index) |
| `q` / `Ctrl+C` | Quit |
| `F12` | Dump the keymap reverse table to the log file (dev/support) |

### Cross-tab jumps (`g`)

Press `g` to open a context-sensitive jump menu. The menu shows only
the destinations that are reachable from the current selection. Select
a destination and press `Enter`, or press `Esc` to cancel.

Available destinations (context-dependent):

| Destination | Goes to |
|---|---|
| Browser | Show selection in the Browser tab |
| Events | Show provenance events for the selection |
| Tracer | Trace the selection's flowfile in Tracer |

### Bulletins

| Key | Action |
|---|---|
| `1` / `2` / `3` | Toggle error / warning / info severity filter |
| `Shift+T` | Cycle component-type filter |
| `Shift+G` | Cycle group-by mode (`source+msg` / `source` / `off`) |
| `Shift+P` | Pause / resume auto-scroll |
| `Shift+M` | Mute selected row's source for the session |
| `c` | Copy raw message to clipboard |
| `Shift+R` | Clear all filters |
| `/` | Open text search |
| `r` | Refresh |
| `Enter` | Jump to source component in Browser (Rule 1a fallback) |

### Browser

| Key | Action |
|---|---|
| `p` | Open Properties modal (Processor / Controller Service) |
| `c` | Copy id (tree) / row value (detail) |
| `r` | Refresh the tree |

### Events

| Key | Action |
|---|---|
| `Shift+D` / `Shift+T` / `Shift+S` / `Shift+U` / `Shift+A` | Edit Time / Types / Source / UUID / Attributes filter |
| `n` | Clear filters and submit a new query |
| `r` | Reset filters (no submit) |
| `Shift+L` | Raise result cap (500 → 5000) |
| `Enter` (filter bar) | Submit query |

### Tracer

| Key | Action |
|---|---|
| `Enter` (Entry) | Submit lineage query |
| `Enter` (Timeline) | Load event detail and focus the Detail pane |
| `←` / `→` (Detail) | Cycle Attributes / Input / Output tabs |
| `d` (Attributes tab) | Toggle attribute All / Changed diff |
| `s` (Input / Output tab) | Save raw content bytes to a file |
| `r` | Refresh lineage |
| `c` | Copy UUID / attribute value |
| `Esc` (Detail) | Return to the timeline |
| `Esc` (Timeline) | Return to the Entry screen |

No view binds `j` or `k`. No view binds bare `[` or `]`. Regression
tests enforce this.

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
