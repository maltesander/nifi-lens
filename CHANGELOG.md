# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **First-run config bootstrap.** Launching `nifilens` without a config
  file now writes the commented template to the platform default path
  and prints guidance ("edit it with your cluster URL and credentials,
  then re-run nifilens"), instead of erroring out. `--config <path>`
  still surfaces the original error so an explicit override isn't
  silently bootstrapped at a different location.
- **Help modal grows and scrolls.** `?` now opens a help modal that
  sizes to fit the terminal (up to 80 cols × area-4 rows), and scrolls
  via `↑/↓/PgUp/PgDn/Home/End` when the active tab's verb set
  overflows. A footer chip shows the current page when scrolling is
  active.
- Browser: new **Access** modal (`u`) showing who can view / modify /
  view-data / operate / manage-policies on the selected UUID-bearing
  component (process group, processor, controller service, ports, RPG,
  connection). Pressing `Enter` on a user or group drills into a
  per-identity view of every (action, resource) grant cluster-wide, with
  cross-links back to Browser arena entries for resolvable resources.
  Read-only. Gracefully disables on clusters with no authorizer configured.
- **Search inside Access and Identity modals.** Both modals exposed
  `OpenSearch / SearchNext / SearchPrev` in their verb tables (so the
  hint bar promised `/ search`), but the dispatcher silently no-op'd
  every search verb — pressing `/` did nothing. With realistic
  clusters easily holding 100+ identities, that gap mattered. Search
  is now wired end-to-end, mirroring the version-control modal:
  `/ {query}_` live filter against identity strings (Access) or
  resource paths (Identity), `Enter` to commit, `n` / `N` to cycle
  matches with the row cursor following, matched substrings rendered
  in `theme::search_match`. The Access matrix appends `[group]` to
  group rows so users can narrow by category. The Identity drill-in
  intentionally keeps the prefix-free `resource` body so match
  offsets index the rendered path directly. `Esc` cascades through
  search before closing the modal — first press cancels search,
  second press closes — matching the established cascade in the
  ActionHistory and VersionControl modals.

### Changed

- **Version-control modal label clarity.** The `[e]` chord now reads
  `[e] bundle diffs` instead of the cryptic `[e] env`, and the body
  status chip reads `bundle diffs shown` / `bundle diffs hidden`
  instead of `env shown` / `env hidden`. Both refer to the same
  thing — NAR / bundle-version differences — but the original
  abbreviation gave a first-time user no clue what was being
  toggled. Also dropped `[n] next` from the VC modal hint bar to
  match every other searchable modal (Access / Identity / Reporting
  Tasks / VC now all show `/` only; `n` / `N` are discovered after
  pressing `/`).
- **Events watch predicate parse error stays visible until the next
  successful commit.** `push_predicate_char` and `pop_predicate_char`
  used to clear `WatchSession.last_parse_error` on every keystroke,
  so the user couldn't tell whether their in-progress edit fixed
  the previously rejected predicate — they only learned by pressing
  Enter again. The chip now persists across edits and clears in
  `commit_predicate` only on a successful parse, so disappearance of
  the chip is itself the success signal.
- **Bulletins text filter shows the committed value while the user
  is editing.** Previously the chip row collapsed to `text: foo_`
  during input, hiding the previously committed `bar` filter that
  Esc would revert to. Editing now reads `text: foo_  (was: bar)`
  when a prior committed filter exists, so the user knows whether
  cancelling unwinds to a filtered or unfiltered state.
- **Queue listing preserves selection by FlowFile UUID** across the
  next bulk fetch. Previously `apply_complete` replaced `rows` and
  re-clamped `selected` to the visible window, so the cursor stayed
  numerically at index 50 even though row 50 was now a different
  FlowFile. The reducer now captures the selected UUID before the
  swap and re-locates it in the new visible set; if it's gone (or
  filtered out) selection falls back to the prior clamp behavior.
- **Bulletins paused mode preserves the selected group across ring
  eviction.** While paused, when a new bulletin arrives and pushes
  an old one off the front of the cluster ring, every grouped
  position shifts up by one — the cursor used to silently land on a
  different group. `redraw_bulletins` now captures the selected
  group key before the mirror swap and re-resolves the cursor to
  the same logical group afterward; auto-scroll is unchanged
  (still snaps to newest).
- **Hint bar pins `?` help in the right cluster** alongside the
  version. Previously it lived at the tail of the dynamic hint list
  and was the first thing to fall off when truncating on narrow
  terminals — exactly the wrong outcome for the discoverability
  anchor. The pinned slot now reads `? help · nifi-lens vX.Y.Z` and
  survives any width below which the dynamic hints collapse to `…`.
- **Status bar refresh-age glyph turns yellow at 60 s and red at 5 min**
  since any successful fetch. `state.last_refresh` is bumped on every
  `ClusterUpdate`, so a climbing counter signals systemic failure
  across all 12 endpoints; the colour shift gives the user that
  signal at a glance instead of leaving them to mentally track an
  ever-incrementing `⟳ Nm ago`.
- **Fuzzy-find empty state names the active filter** instead of a
  generic "no matches". `:drift` with no hits now reads
  `no PGs with drift`, `:stale` reads `no stale PGs`, `:proc` reads
  `no processors in this cluster`, etc. — so the user can tell
  whether their filter typo'd or the cluster genuinely has none.
- **Bulletins hint row** now advertises the `1·2·3 sev` chord that
  toggles ERROR / WARN / INFO severity filters — the chips themselves
  showed counts but never the chord that flips them.
- **Visual language unified.** A single `LOADING_LABEL` (`"loading…"`)
  now renders everywhere a fetch is in-flight (Browser tree, Bulletins
  list age chip, Tracer event detail / latest-events / content sides);
  three previous variants (`"initial fetch…"`, `"connecting…"`,
  `"Loading event detail…"`) are gone. A new `theme::PLACEHOLDER_DASH`
  constant (em-dash) replaces the previous mix of `(none)` / `none` /
  `-` / blank for absent values in Browser connection panes, Events
  detail relationship, Tracer attribute diff cells and lineage rows,
  and Overview node-detail TLS chain. The `Panel` widget gained
  `border_style(Style)` and `rounded()` builder methods; ten previous
  raw `Block::default().borders(...)` / `Block::bordered()` sites
  (Browser tree outer block, Events confirm modal, Tracer content
  modal, Bulletins detail modal, properties / version-control /
  identity / save / help / error-detail modals, goto menu, watch
  strip) now route through `Panel`. Severity-coloured frames (the
  Bulletins detail modal) keep their hue via `border_style`.
- **Scrollbar indicators.** A new `widget::scroll::render_vertical_scrollbar`
  helper renders a position indicator on the right edge of any
  scrollable pane. Wired into the Bulletins detail modal and Browser
  Action history modal as the first integration sites — the bar
  auto-suppresses when content fits the viewport. Same helper is
  available for any further pane that already tracks a
  `VerticalScrollState`.
- **Modal framework hardening.** The Bulletins detail modal and Tracer
  content viewer modal are now consistent with the rest of the
  framework. Both now short-circuit at `widget::modal::MIN_WIDTH ×
  MIN_HEIGHT` via `render_too_small`, render the search prompt via
  the shared `widget::search::render_search_{input,strip}` helpers,
  and render the footer hint strip via the shared
  `widget::modal::render_verb_hint_strip{,_with}` helpers. Bulletins
  detail gained a proper `BulletinsDetailModalVerb` enum + a
  `BulletinsDetailModalGate` chained in `KeyMap::translate`, so its
  modal-only chords (`Esc` / `/` / `n` / `N` / `c`) shadow the outer
  Bulletins-tab keybindings while open instead of running through
  the parent verb dispatch. The Queue-listing peek modal's hand-rolled
  `/`-prompt was migrated to `render_search_input`; cursor glyph and
  spacing now match the rest of the app.
- Integration test fixture switched from NiFi `single-user-authorizer` to
  `managed-authorizer` with file-based providers. Now seeded with `admin`
  / `alice` / `bob` / `carol` users and an `ops-team` group, plus
  realistic component-level policies on the `orders-pipeline` and
  `versioned-clean` PGs. Live integration coverage now exercises the
  access-policies-audit feature end-to-end
  (`tests/integration_browser_access.rs`).
- **Reporting tasks visibility on Overview**: a Components-panel row showing
  `running / stopped / invalid` counts plus a `t`-launched master-detail
  modal with properties, validation errors, parameter-ref annotations, and
  filtered bulletin history. Read-only.
- Events watch sub-mode: live tail of provenance events with a
  client-side AND-of-clauses attribute predicate. Lowercase `w` on
  Browser / Tracer rows opens Events pre-narrowed to a component.
- Config: `[events] watch_buffer_size`, `[events] watch_retry_max`,
  `[polling.cluster] events_tail`.
- Internal: removed the main-thread `LocalSet` workaround. All polling
  fetchers and view workers now run on a single multi-thread tokio
  runtime via `tokio::spawn`. Drop-side cleanup HTTP DELETEs use a new
  `app::cleanup::spawn_cleanup` helper that silently no-ops outside an
  active runtime. No user-visible behaviour change; the parquet/avro
  classification and per-PG fan-out fetchers gain real parallelism.

### Fixed

- **Tracer diff modal no longer freezes the UI for ~30s after JSON
  pretty-print.** Pretty-printing a 2 MB compact JSON FlowFile
  explodes its line count from a handful to tens of thousands;
  `compute_diff_cache` then ran synchronously on the UI thread when
  the pretty-print result invalidated the cache (and again when the
  second side's pretty arrived), running `similar::TextDiff::from_lines`
  plus per-pair char-level inline diffs. The diff cache build now
  runs off-thread via `tokio::task::spawn_blocking` (mirrors the
  existing pretty-print and tabular-decode plumbing) with a
  generation token so a stale result whose inputs changed
  mid-compute is dropped on arrival rather than installed. The Diff
  tab shows "computing diff…" while the off-thread compute runs.
- Status bar `init: X/N endpoints ready` chip now counts the
  `reporting_tasks` fetcher (previously hard-coded `/11` even though the
  store owns 12 endpoints, so the chip never reached `12/12`).
- Logging directory is created `0o700` in a single syscall on unix,
  closing a brief race window between `mkdir` and `chmod`. Daily log
  files are pruned at startup to keep the most recent 14, capping
  on-disk growth.

### Internal

- Centralised `TransmissionStatus` typed enum (`client::status`) for
  RPG transmission state; replaced ad-hoc string compares.
- Production-side `client::ROOT_GROUP_ID` constant for NiFi's documented
  root-PG alias; replaced literal `"root"` strings across reducers,
  fixtures, and tests.
- Extracted shared modal helpers: `layout::center_percent` /
  `layout::center_absolute` (was duplicated four times) and
  `widget::search::render_search_input` (footer search-bar prompt was
  duplicated across the version-control, parameter-context, and
  action-history modals).
- Tracer hex-fallback cap is now a named `HEX_PREVIEW_BYTES` constant.
- Node-detail render fixtures use `bytes::GIB` instead of inline
  `1024_u64.pow(3)` literals.

## [0.9.0] — 2026-04-30

### Added

- **Remote Process Group support** across the Browser tree (transmission
  badge, target URI chip), Identity pane (target URI, transport
  protocol, validation status, remote port descriptions), 3-row
  sparkline (received flowfiles / sent flowfiles / total bytes/s),
  Overview Components panel (`Remote PGs` row with TRANSMIT / NOT-TX
  counts), fuzzy-find (`:rpg` filter alias), Bulletins → Browser jump,
  action-history modal (`a`), and version-control diff modal.
- Browser queue listing panel — selecting a queued connection in the
  Browser tab now lists up to 100 flowfiles inline, with per-row
  chords: `i` peek attributes, `t` trace lineage in Tracer, `c` copy
  UUID, `/` filter by filename, `r` refresh.
- **Action history modal** opens with `a` on Browser detail rows
  (processor, PG, connection, controller service, port) to view NiFi
  flow-configuration audit events filtered by `sourceId`. Paginated
  with auto-load on scroll (100 rows per page; loads next when the
  selection comes within 10 rows of the loaded tail), substring search
  (`/`, `n`/`Shift+N` cycle), copy-as-TSV (`c`), inline expansion of a
  selected row (`Enter`), and refresh from offset 0 (`r`). Read-only.
- **Inline sparkline strip** in Browser detail identity panels for
  processor / PG / connection rows. Three rows per pane (in flowfiles,
  out flowfiles, task time or queued count) backed by NiFi's
  `/flow/{type}/{id}/status/history`. Selection-scoped periodic
  worker; default 30 s cadence configurable via
  `[polling.cluster] status_history`. Responsive — suppressed below
  24 cells of identity-inner width so narrow terminals keep the
  identity panel readable.

### Changed

- Tracer content modal chunk size raised from 512 KiB to 8 MiB
  (`MODAL_CHUNK_BYTES`). Typical record-format flowfiles (text / JSON
  / Parquet / Avro at the few-MiB scale) now load in a single
  round-trip, so the diff modal can compute its diff without
  depending on scroll-driven follow-up fetches that don't fire on
  an empty body.
- Tracer content JSON pretty-print no longer blocks the UI thread.
  Per-chunk classification renders plain UTF-8 incrementally; the
  pretty-print pass runs once off-thread (`spawn_blocking`) when the
  side is fully loaded. The reformatter switches from a
  `serde_json::Value` round-trip to a streaming `serde_transcode`
  pipeline, eliminating the per-object allocation pass — roughly 3×
  faster in release, much wider in debug. **Object key order is now
  preserved** (the `Value`-based path alphabetised through `BTreeMap`).
- Integration test fixture reworked into a single `orders-pipeline`
  centerpiece (ingest → transform → regional sinks + deadletter)
  with a 5-context parameter hierarchy and a `--break-after` seeder
  flag that controls when the demo's headline parameter mutation
  lands. Replaces the previous focused-pipeline set
  (healthy/noisy/bulky/diff/parameterized/remote); standalone
  fixtures retained where state encoding requires it
  (invalid, backpressure, versioned-clean, versioned-modified).

## [0.8.1] — 2026-04-27

### Fixed

- Windows binary build (cargo-dist) failed to compile `nifi-rust-client`
  because the generated `#[path = "..."]` attributes embedded the OUT_DIR
  with backslashes, which rustc parsed as invalid escape sequences
  (`\a`, `\x86_64`, …). Fixed upstream in `nifi-rust-client` 0.11.1; this
  release bumps the dependency to pick it up.

## [0.8.0] — 2026-04-27

### Added

- **`[polling.cluster] batch_concurrency` knob** (default `16`)
  bounds the maximum number of concurrent in-flight HTTP requests
  the per-PG fan-out fetchers (`version_control`,
  `parameter_context_bindings`, `connections_by_pg`) issue per tick.
  On a 500-PG cluster this caps in-flight from ~500 to 16 by
  default, dramatically reducing pressure on NiFi's HTTP thread
  pool. See [README §Configuration](README.md#configuration). Raise
  on fast clusters; lower on overloaded ones.
- **`init: X/11 endpoints ready` status-bar chip** displayed during
  boot until every cluster fetcher has produced its first snapshot.
  Replaces the "screen looks broken for 10 seconds" first-run UX
  on slow clusters.
- **`NO_COLOR` env var** honored alongside the existing `--no-color`
  CLI flag, per [no-color.org](https://no-color.org/). Useful for
  CI logs and screen readers.
- **`Flags`** section in the README documenting `--config`,
  `--context`, `--debug`, `--log-level`, `--no-color`, plus the
  `NIFILENS_LOG` and `RUST_LOG` env-var fallbacks for log filtering.
- **Logs** section in the README explaining the daily-rotated
  `nifilens.log.YYYY-MM-DD` filename pattern and a
  `tail-the-latest` recipe that handles midnight rollover.

- **Fuzzy Find kind filter**: type `:proc`, `:pg`, `:cs`, `:conn`,
  `:in`, or `:out` at the start of the query to narrow the corpus to
  a single component kind. A chip row above the query shows the
  active filter. Clear by backspacing through the prefix.

- **Distribution**: prebuilt binaries for Linux (x86_64 / aarch64,
  gnu + musl), macOS (x86_64 / aarch64), and Windows (x86_64) are now
  attached to each GitHub Release, along with shell and PowerShell
  one-line installers and a Homebrew formula. The binary pipeline is
  owned by `cargo-dist` (`dist-workspace.toml` + autogenerated
  `.github/workflows/release.yml`); `cargo publish` continues to run
  from `publish-crate.yml`.

- Browser tree-row drift chips (`[STALE]` / `[MODIFIED]` /
  `[STALE+MOD]` / `[SYNC-ERR]`) on versioned process groups whose
  registry state ≠ `UP_TO_DATE`.
- Version-control modal opened with `m` on any versioned PG: registry /
  bucket / branch / flow / version identity plus per-component,
  per-property diff from `/process-groups/{id}/local-modifications`.
  Search (`/` `n` `N`), copy (`c`), refresh (`r`), and an
  environmental-toggle (`e`, hidden by default).
- Fuzzy Find prefix tokens for drift filtering: `:drift`, `:stale`,
  `:modified`, `:syncerr` (PG-only).
- New `[polling.cluster] version_control` configuration key (default
  `30s`); subscriber-gated to Browser.
- Browser: Parameter Context modal — open via `p` on any PG (incl.
  root). Walks the inheritance chain, renders resolved parameters with
  override / sensitive / provided / unresolved flags, surfaces
  reverse-lookup ("Used by N PGs"), and adds `→` cross-links from
  `#{name}` parameter references in processor / controller-service
  property values. The PG detail pane gains a `Parameter context:` row
  cross-linking into the same modal. Read-only.

### Changed

- **`--allow-writes` CLI flag is hidden from `--help`.** The flag is
  still parsed (and still rejected at startup with a "writes not
  implemented" error), but no longer surfaces in `--help` output —
  removes a frequent footgun for users who think writes are
  configurable in v0.1.
- **AppEvent channel saturation watchdog**: a 1 Hz background task
  emits a `tracing::warn!` whenever fewer than 16 of the channel's
  256 slots remain free. Self-rate-limiting (silent when load
  recovers); makes slow renders and producer surges visible without
  any hot-path overhead.

- **Config / log paths** now resolve via `directories::ProjectDirs`
  and pick the right location per OS. Linux behavior is unchanged
  (`$XDG_CONFIG_HOME` / `$XDG_STATE_HOME`, fallback to
  `~/.config/nifilens/` and `~/.local/state/nifilens/`). On macOS,
  config lives in `~/Library/Application Support/nifilens/` and logs
  in `~/Library/Caches/nifilens/`. On Windows, config lives in
  `%APPDATA%\nifilens\config\` and logs in `%LOCALAPPDATA%\nifilens\cache\`.
  The binary previously refused to start on Windows because `HOME`
  is not set there.

### Fixed

- **Clipboard read/write timeout (2 s).** `arboard::Clipboard`
  operations could hang indefinitely on stalled X11/Wayland
  clipboard daemons, freezing the UI. Each call now runs on a
  worker thread behind a 2-second deadline; on timeout the user
  sees a `clipboard: write timed out after 2s` banner and the next
  call re-initializes a fresh handle.
- **Parquet/Avro decoder timeout (5 s).** Pathological inputs
  (broken offsets, huge schemas) could wedge a `spawn_blocking`
  worker. Each decoder now runs on a worker thread with a 5-second
  deadline; on timeout `classify_content` falls back to `Hex` via
  the existing error path.

- Tracer content viewer modal: `/`-search highlights now render on
  Input/Output rows for Parquet- and Avro-decoded tabular content. The
  Tabular renderer previously bypassed the search-overlay loop, so
  matches counted and `n`/`N` navigation worked but no row was
  visually marked.
- Browser detail-pane focus is no longer wiped on every periodic
  cluster refresh. Selecting a processor's Properties row, scrolling
  inside it, or stepping between detail sections used to reset back
  to the tree on each `RootPgStatus` / `ControllerServices` /
  `ConnectionsByPg` tick (default 10s). Focus now persists as long
  as the selected `(id, kind)` survives the arena rebuild.
- Status-line `info` and `warning` banners now auto-clear on the next
  input event so transient toasts (e.g. `copied: …`,
  `clipboard: no display`) do not linger after the user has moved on.
  `error` banners stay sticky — they may carry detail to expand and
  must be acknowledged with `Esc`.
- Pressing `m` on a non-versioned Browser selection is now a silent
  no-op. The verb is grayed out in the hint bar, but the keymap still
  dispatched it, surfacing a sticky `not under version control`
  warning banner that could only be cleared with `Esc`.

## [0.7.0] — 2026-04-24

### Added

- **Tracer**: full-screen content viewer modal (`i`) with Input /
  Output / Diff tabs, streaming fetch, colored unified diff,
  change navigation (`Ctrl+↓` / `Ctrl+↑`) that steps through every
  individual change — including dense bodies like CSV where every
  row changed — and a `· N changes` count on the header's sizes
  line. Bulletins-style search primitives are shared via
  `src/widget/search.rs`.
- **Config**: `[tracer] modal_streaming_ceiling` (default `4MiB`,
  `"0"` = unbounded) bounds the modal's per-side load.
- **Overview** Nodes panel: per-row role/status badge (`[PC]` / `[P·]` /
  `[·C]` / `[··]` / `[OFF]` / `[DIS]` / `[CON]`) and heartbeat-age
  column, joined from a new `ClusterEndpoint::ClusterNodes` fetcher
  polling `/controller/cluster` every 5 s by default.
- **Overview** node detail modal: redesigned as a four-quadrant dashboard
  (identity header, resources, repositories per-disk, events timeline,
  GC table). Standalone NiFi servers degrade cleanly to a reduced
  layout.
- `[polling.cluster] cluster_nodes` config key (default `5s`).
- **Tracer**: content viewer decodes Apache Parquet (`PAR1` magic) and
  Apache Avro Object Container Files (`Obj\x01` magic) into a schema
  header + JSON-Lines body. The Diff tab supports Parquet↔Parquet and
  Avro↔Avro comparisons; Parquet↔Avro shows a Mime mismatch.
- **Overview**: per-node TLS certificate expiry now surfaced on the
  Nodes list as a trailing chip (always shown: red/bold for
  expired or `<7d`, yellow for `7–30d`, muted grey when healthy at
  `≥30d`; quiet only when data is missing) and in the node detail
  modal as a full chain with per-entry `not_after`. Standalone
  NiFi falls back to probing the context URL's host+port; HTTP-
  only contexts skip probing. New `[polling.cluster] tls_certs`
  cadence knob (default `1h`).

### Changed

- **Config**: `[tracer] modal_streaming_ceiling` is replaced by the
  nested `[tracer.ceiling]` table with `text`, `tabular`, and `diff`
  keys (defaults `4 MiB` / `64 MiB` / `16 MiB`; `"0"` = unbounded).
  The legacy key is honored for one release with a deprecation
  warning, then removed. Diff size cap is now configurable via
  `[tracer.ceiling] diff` (was a fixed 512 KiB).
- **Tracer**: inline content preview cap lowered from 1 MiB to 8 KiB.
  Use `i` to open the new modal for full streamed content.
- **Bulletins detail modal**: `Enter` is now a no-op. Previously it
  jumped to the source in Browser, which caused an accidental
  navigation when committing a `/`-search with Enter and then pressing
  Enter a second time. To jump to the source, close the modal and use
  `g` on the Bulletins tab.
- **Fixture**: `diff-pipeline` adds `ConvertRecord-parquet` and
  `ConvertRecord-avro` sink chains plus the supporting
  `diff-parquet-writer` / `diff-avro-writer` controller services for
  live-cluster Tabular decode coverage. The fixture marker is
  bumped to `nifilens-fixture-v3`.
- **Build**: `integration-tests/scripts/download-nars.sh` fetches
  `nifi-parquet-nar` (and its transitive `nifi-hadoop-libraries-nar`
  dependency) from Maven Central into a gitignored cache; the NARs
  are mounted per-version into each NiFi container. Required because
  `apache/nifi` base images don't bundle the standalone Parquet
  writer.

### Fixed

- **Input**: `Shift+Tab` (previous pane focus) no longer falls through to
  `Unmapped` on crossterm ≥ 0.28, which delivers the key as
  `KeyCode::BackTab` with the redundant `SHIFT` modifier bit set. The
  bit is now stripped at the keymap boundary so the `BackTab` chord
  matches again across every view.
- **Bulletins**: time column no longer renders `--:--:--` on NiFi < 2.7.2.
  Those versions omit `timestampIso` on `BulletinDTO` and ship `timestamp`
  as wall-clock time only (`HH:MM:SS UTC`); the client now synthesizes
  an ISO-8601 value at fetch time by combining it with today's UTC
  date (with a one-minute grace window that backs off by a day for
  bulletins emitted just before midnight and polled just after). Same
  fix also unblocks the Overview histogram and the detail modal on
  NiFi 2.6.0.

### Security

- Bump `rustls-webpki` to 0.103.13, resolving
  [RUSTSEC-2026-0104][rustsec-0104] (reachable panic in CRL parsing).
  `nifi-lens` does not use CRLs, so it was not exposed, but the
  transitive lockfile entry is updated to keep `cargo deny` green.

[rustsec-0104]: https://rustsec.org/advisories/RUSTSEC-2026-0104

## [0.6.0] — 2026-04-21

### Added

- **Bulletins**: new detail modal (`i`) shows the full raw message with
  vertical scroll (`↑`/`↓`, `PgUp`/`PgDn`, `Home`/`End`),
  plain-substring `/`-search with `n`/`N` cycling, `c` to copy, and
  `Enter` to jump to the source in Browser. `Esc` closes.

### Changed

- **Tracer**: the Save-to-disk action now streams the flowfile body
  chunk-by-chunk to the target path via the new
  `provenance_content_stream` helper, replacing the previous
  fetch-into-`Vec<u8>`-then-write path. Large flowfiles no longer spike
  RAM during save.
- **Bulletins**: the `×N` repeat-count cell is now dim grey (bold)
  instead of yellow, so the severity column is the only color signal
  on each row.
- **Bulletins**: `1`/`2`/`3` severity hints no longer appear in the
  status-bar hint strip — the `[E n] [W n] [I n]` filter chips already
  surface both the shortcut and the filter state. Still documented in
  `?` help.

## [0.5.0] — 2026-04-18

### Changed

- **Architecture**: all periodic NiFi polling is now centralized in a
  single `ClusterStore` (seven per-endpoint fetchers), replacing the
  per-view worker polls that Overview, Browser, and Bulletins used to
  run. Overview and Browser now share a single `root_pg_status`,
  `controller_services`, and per-PG `connections_by_pg` fetch — load
  reduction is proportional to the number of tabs that previously
  duplicated these polls.
- Polling cadences adapt to measured latency (up to `max_interval`,
  default `60s`) and are jittered by ±`jitter_percent/100` (default
  20%) to avoid synchronized bursts across endpoints.
- Three expensive endpoints (`root_pg_status`, `controller_services`,
  `connections_by_pg`) park when no view subscribes to them — i.e.
  while neither Overview nor Browser is the active tab.
- **Overview**: top panel renamed from `Processors` to `Components` and
  expanded into a three-row table (process groups, processors,
  controller services). PG row shows version-sync drift counts (or
  `all in sync` when healthy) and input/output port counts. CS row
  shows per-state counts; degrades to a `cs list unavailable` chip
  when the new `/flow/process-groups/root/controller-services` fetch
  fails. Drops the always-near-zero `THREADS` field.

- **Overview**: nodes panel now lists cluster nodes sorted
  alphabetically by `host:port` (case-insensitive) instead of in the
  order returned by `/system-diagnostics`.

- **Browser**: the `p` properties popup is now a selectable two-column
  table. `↑`/`↓` move the selection, `c` copies the focused row's
  value to the clipboard, and `Enter` on a property whose value is a
  UUID pointing to a known arena node closes the modal and jumps to
  that node in the tree (same cross-link path used by the detail
  pane). A fixed detail strip below the table shows the selected
  row's full value so long values stay readable.

### Fixed

- **Bulletins**: `Shift+R` (clear filters) now also clears the
  session-scoped mute list. Previously muted sources could only be
  unmuted by restarting the binary, because muted rows are hidden and
  could not be reselected to toggle `Shift+M` off.

### Removed

- Per-view Overview, Browser, and Bulletins worker tasks — replaced by
  `ClusterStore` fetchers and `redraw_*` reducers driven off
  `AppEvent::ClusterChanged`.
- Sysdiag nodewise → aggregate fallback banner — the transition is now
  logged to `nifilens.log` rather than surfaced in the TUI. Monitor the
  log if you run mixed-version fleets.

### Security

- Bump `rand` to 0.10, resolving [RUSTSEC-2026-0097][rustsec-0097]
  (soundness advisory for `ThreadRng` under custom loggers).

[rustsec-0097]: https://rustsec.org/advisories/RUSTSEC-2026-0097

### Breaking (config)

- Per-view polling sections `[polling.overview]`, `[polling.browser]`,
  and `[polling.bulletins]` have been replaced by a single
  `[polling.cluster]` section. See `README.md` for the new shape. There
  is no back-compat shim: `config.toml` files that still use the old
  sections will fail to parse.

## [0.4.0] — 2026-04-17

### Added

- **Configurable poll intervals** for Overview, Browser, and Bulletins
  tabs via a new `[polling]` section in `config.toml`. Values use the
  humantime format (`"10s"`, `"750ms"`). Defaults match the previously-
  hardcoded cadences (10s/30s/15s/5s). Out-of-band values log a warning
  but are accepted as-is. Events in-flight query polling and Tracer
  content polling stay on their internal cadences.
- **Browser**: controller services appear as first-class tree nodes under
  their owning PG, bucketed under a collapsible `⚙ Controller services`
  folder.
- **Browser**: queues moved under a collapsible `→ Queues` folder per PG.
- **Browser**: controller-service detail pane shows comments,
  `restricted / deprecated / persistsState` flags, a `Referencing
  components` section (Enter to jump to the referencing component),
  and a `Recent bulletins` section.
- **Browser**: input / output ports have a working detail pane
  (identity + recent bulletins).
- **Browser**: cross-navigation across detail panes. Queue endpoints,
  processor CS-reference properties, and a new processor `Connections`
  section all jump to the referenced component on Enter (reuses the
  existing `OpenInBrowser` cross-link). Rows whose value resolves to a
  known arena node render a trailing `→` marker. Controller Service
  and Port Identity panels now show the parent process group's name
  instead of the raw UUID.
- **Tracer content-pane preview cap**: bodies over 1 MiB are fetched with
  a `Range: bytes=0-1048575` header and flagged as truncated in the panel
  title. Save re-fetches the full body on demand.
- **Integration fixture**: new `bulky-pipeline` producing ~1.5 MiB
  flowfiles, and an `UpdateAttribute-cleanup` processor in
  `healthy-pipeline/enrich` that exercises the attribute-removed rendering
  path.

### Changed

- Fixture now wires a `ConvertRecord` processor at the start of
  `healthy-pipeline/enrich` referencing `fixture-json-reader` and
  `fixture-json-writer` (both ENABLED at root), so the browser
  CS-referencing integration test has at least one referenced CS on
  NiFi 2.6.0 (previously only `stress-pipeline` on 2.9.0 created
  references, causing the test to fail on the floor version).
- Fixture marker bumped to `nifilens-fixture-v2`; existing clusters will
  be re-seeded automatically on next run.
- `ContentSnapshot` no longer carries raw bytes; `ContentPane::Shown`
  follows suit. The Save action now re-fetches via
  `provenance_content_raw`.
- `ContentRender::Text` now carries a single authoritative `String` plus a
  `pretty_printed: bool` flag, replacing the separate `pretty: String`
  field.

## [0.3.0] — 2026-04-15

### Added

- **Browser: horizontal scroll in detail panes.** When a detail
  sub-panel is focused (`Tab`/`Shift+Tab`), `←`/`→` now scroll the
  content column one character at a time. Applies to Properties (VALUE
  column), Validation errors, Recent bulletins (message column),
  Controller services (type column), and Child groups (name column).
  Each section remembers its own horizontal offset independently.
- **Browser: validation errors in bordered panel.** Processors and
  controller services that have active validation errors now display them
  in a focusable bordered panel instead of inline text, making the list
  navigable and visually distinct.

### Fixed

- **Tracer: save confirmation shown in status bar.** After saving content
  to a file (`s`), the "saved to \<path\>" message now appears in the
  global footer banner (Info severity) instead of being silently
  discarded. Save failures are likewise surfaced as Error banners.
- **Status bar: long messages truncated with `…`.** Banner text wider than
  the available column count is trimmed and suffixed with an ellipsis
  rather than being hard-clipped by the terminal.
- **Tracer: `Updated` attribute class shown in yellow.** Changed
  attributes in the lineage detail pane now render with the warning
  (yellow) colour to distinguish them from added (green) and deleted
  (red) entries.
- **Events: `timestamp_tz` config honoured in detail pane.** Event detail
  rows now respect the `[timestamps] timezone` setting instead of always
  showing UTC.

## [0.2.0] — 2026-04-15

### Changed

- **Keybinding consolidation — all views.** FuzzyFind moves from `f` to
  `Shift+F`. Bulletins: CycleGroupBy `g` → `Shift+G`, TogglePause `p` →
  `Shift+P`, MuteSource `m` → `Shift+M`, ClearFilters `Shift+C` →
  `Shift+R`. Events: all filter-field edit keys move to Shift variants
  (`Shift+D` Time / `Shift+T` Types / `Shift+S` Source / `Shift+U` UUID /
  `Shift+A` Attributes); new-query `n` → `Shift+N`; reset `r` → `Shift+R`;
  bare `r` is now Refresh. No view binds bare `j`, `k`, `[`, or `]`;
  regression tests enforce this.
- **Cross-tab jump redesigned.** The two-key `g <letter>` leader combo is
  replaced by a single `g` that dispatches `AppAction::Jump`. When exactly
  one destination is reachable the jump fires silently; when multiple are
  available the new JumpMenu modal opens (see Added).
- **Tab / Shift+Tab cycle panes within each view.** All tabs (Overview,
  Browser, Events, Tracer) now use Tab/Shift+Tab for intra-view pane focus
  cycling. Tab-bar switching stays on F1–F5 exclusively.

### Added

- **`v` = paste, `x` = cut** from the system clipboard in every text-input
  field (Bulletins `/` search, Events filter fields, Tracer UUID entry).
- **JumpMenu modal** — `g` opens a scrollable, keyboard-navigable list of
  context-sensitive cross-tab destinations. `↑`/`↓` move the selection,
  `Enter` confirms, `Esc` cancels. Fires immediately when only one
  destination is reachable.
- **Overview interactive panels** — Nodes, Queues, and Noisy panels are
  now focusable `Table` widgets with row selection, scroll-to-cursor, and
  thick-border focus indicators. Tab/Shift+Tab cycles focus between panels.
- **Overview node detail popup** — `Enter` on a selected node row opens a
  two-pane modal with heap/load/threads/uptime summary on the left and GC
  collector table + per-type repository utilization with fill bars on the
  right.
- **Tracer timeline enriched** — lineage rows now show component type and
  per-event detail hints alongside the existing timestamp and event-type
  columns.
- **Bulletins `c`** copies the selected row's raw message to the clipboard.

### Fixed

- Overview: Queues panel `g` → Browser cross-link now navigates to the
  connection's parent process group instead of the connection itself.
- Overview: aligned repository fill bars; replaced aggregate load
  spark-bar with per-CPU strip.
- Overview: fixed bulletins-per-minute rolling window accumulating
  duplicate counts.
- Events: widened type column; replaced the empty `rel` column with an
  event-details column.

### Security

- Updated `rustls-webpki` to 0.103.12 (fixes
  [RUSTSEC-2026-0098](https://rustsec.org/advisories/RUSTSEC-2026-0098)
  and
  [RUSTSEC-2026-0099](https://rustsec.org/advisories/RUSTSEC-2026-0099)).

## [0.1.0] — 2026-04-14

Initial public release. Condensed summary of the development phases
that landed in this tag.

### Added

- **Five tabs** — Overview (cluster health dashboard with sparkline,
  queue / repository fill, per-node heap/GC/load, noisiest components),
  Bulletins (live cluster-wide tail with severity / source dedup and
  per-source mute), Browser (PG tree with per-node detail panes for
  Processor / Connection / Process Group / Controller Service / Ports,
  cross-navigation `→` jumps, properties modal), Events (cluster-wide
  provenance search with filter bar, result detail pane), Tracer (paste
  a flowfile UUID → lineage timeline → tabbed Attributes / Input /
  Output detail with content preview and save).
- **Multi-cluster** — kubeconfig-style `~/.config/nifilens/config.toml`
  with `[[contexts]]`, `current_context`, `Shift+K` to switch at
  runtime. `0600` permission enforcement.
- **Auth variants** — `[contexts.auth]` sub-table with `type =
  password | token | mtls`; `password_env` / `token_env` env-var
  indirection; `proxied_entities_chain` for proxy deployments.
- **TLS** — system trust store by default; optional per-context
  `ca_cert_path` PEM; `insecure_tls = true` with a loud warning.
- **CLI** — `clap` derive with `--config`, `--context`, `--debug`,
  `--log-level`, `--no-color`, `--allow-writes` (reserved). Subcommands:
  `config init`, `config validate`, `version`.
- **Input layer** — typed action enums (`FocusAction`, `HistoryAction`,
  `TabAction`, `AppAction`, per-view `ViewVerb`) plus a shared `Verb`
  trait. Hint bar and help modal are both generated from `Verb`, so
  adding a keybinding updates both surfaces.
- **Cross-tab navigation** — `g` opens a context-sensitive jump menu;
  `Enter` on a Bulletins / Events row lands on the matching component;
  `t` on a row traces it; arena cross-links decorate detail rows with
  a trailing `→`. Tab history via `Shift+←` / `Shift+→` with selection
  restore.
- **Fuzzy find** — global `Shift+F` modal, nucleo-backed, kind · name ·
  path · state columns with highlighted matches.
- **Bulletins ring buffer** — configurable via `[bulletins] ring_size`
  (default 5000; 100..=100_000). Dedup by `(source_id, message_stem)`
  with dynamic `[...]` normalization collapses repeat errors into a
  single `×N` row.
- **Configurable poll cadences** — `[polling.cluster]` per-endpoint
  humantime values (`"10s"`, `"750ms"`). Adaptive scaling up to
  `max_interval` and ±`jitter_percent` jitter.
- **`[ui]` config** — `timestamp_format` (`short` / `iso` / `human`),
  `timestamp_tz` (`utc` / `local`).
- **Visual language** — project-wide bordered-box via
  `widget::panel::Panel`; focused panels flip to thick borders in the
  accent color. `widget::severity` / `widget::run_icon` / `widget::gauge`
  centralize severity labels, run-state glyphs, and fill / spark bars.
- **NifiClient wrapper** around `nifi_rust_client::DynamicClient`
  (`Deref` / `DerefMut`) with typed snapshot helpers for the seven
  endpoints the UI needs; clustered-NiFi `clusterNodeId` pinned at
  login.
- **Central `ClusterStore`** — owns all periodic fetchers (one task per
  endpoint), subscriber-gated for expensive endpoints, snapshot
  mutation only on the UI task. Views subscribe; no per-view pollers.
- **Per-tab `WorkerRegistry`** — on-demand detail fetches swap with
  tab activation / exit.
- **Rotating logging** via `tracing-subscriber` + `tracing-appender` to
  `$XDG_STATE_HOME/nifilens/nifilens.log`; env-filter priority chain
  (`--log-level` > `--debug` > `NIFILENS_LOG` > `RUST_LOG` > `info`).
  `StderrToggle` suppresses stderr output while raw mode is active.
- **TerminalGuard** RAII wrapper + panic hook so the terminal always
  restores cleanly before `color_eyre` prints.
- **Error banners** — transient status-line banners with expandable
  detail modal; never writes to stdout / stderr while the TUI is
  active.
- **`Intent` pipeline** — enum with read + write variants. Write
  intents unconditionally refuse without `--allow-writes` (reserved
  for v2).
- **Crate is lib + bin.** `src/lib.rs` holds every module; `src/main.rs`
  is a thin `nifi_lens::run()` wrapper. Integration tests link against
  the library.
- **Multi-version integration fixture** — `integration-tests/run.sh`
  boots NiFi 2.6.0 (floor) and 2.9.0 (2-node cluster), seeds both via
  `nifilens-fixture-seeder`, runs the `#[ignore]`-gated suite, tears
  down. Versions driven from a single `versions.toml` source of truth
  (compile-time `FIXTURE_VERSIONS` const via `build.rs`); CI drift
  check enforces consistency.
- **Wiremock client tests** for happy path and 401 / 500 surfaces.

### Changed

- **MSRV raised to 1.88** (from 1.85) for `time >= 0.3.47`
  ([RUSTSEC-2026-0009](https://rustsec.org/advisories/RUSTSEC-2026-0009)).
- **`nifi-rust-client`** tracked from 0.5 → 0.10.1 over the release —
  API surface flattened, `traits` module gone, typed provenance
  content bodies, NiFi 2.9.0 support, `clusterNodeId` handling.
- **Row navigation** standardized on `↑`/`↓` + `Home`/`End`. No view
  binds bare `j`, `k`, `[`, or `]`; regression tests enforce this.
- **Keybinding convention** — bare lowercase for view-local, bare
  capital for app-wide, `Ctrl` reserved for quit + text-input helpers.
- **`deny.toml`** allows `BSL-1.0` (arboard transitive) and ignores
  `RUSTSEC-2024-0436` (unmaintained `paste` transitive via ratatui —
  no safe upgrade available).

### Security

- `rustls-webpki` pinned via upstream to pick up fixes later released
  in 0.2.0.

### Notes

- **Read-only.** v0.1 ships no write paths; `--allow-writes` is
  reserved and unused.
- **Intentional omissions for later work.** Per-node repository
  drill-in, processor thread leaderboard, and queue time-to-full
  predictions are known unshipped polish items.

[Unreleased]: https://github.com/maltesander/nifi-lens/compare/v0.9.0...HEAD
[0.9.0]: https://github.com/maltesander/nifi-lens/releases/tag/v0.9.0
[0.8.1]: https://github.com/maltesander/nifi-lens/releases/tag/v0.8.1
[0.8.0]: https://github.com/maltesander/nifi-lens/releases/tag/v0.8.0
[0.7.0]: https://github.com/maltesander/nifi-lens/releases/tag/v0.7.0
[0.6.0]: https://github.com/maltesander/nifi-lens/releases/tag/v0.6.0
[0.5.0]: https://github.com/maltesander/nifi-lens/releases/tag/v0.5.0
[0.4.0]: https://github.com/maltesander/nifi-lens/releases/tag/v0.4.0
[0.3.0]: https://github.com/maltesander/nifi-lens/releases/tag/v0.3.0
[0.2.0]: https://github.com/maltesander/nifi-lens/releases/tag/v0.2.0
[0.1.0]: https://github.com/maltesander/nifi-lens/releases/tag/v0.1.0
