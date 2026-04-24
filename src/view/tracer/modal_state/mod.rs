//! Content viewer modal types, constants, and reducers.
//!
//! Split from `state.rs` to keep that file under a manageable size.
//! All items here are re-exported from `state.rs` via `pub use
//! modal_state::*;` so existing import paths continue to compile
//! unchanged.

use super::state::TracerState;
use crate::widget::scroll::BidirectionalScrollState;

// ── Content viewer modal ──────────────────────────────────────────────────────

/// Immutable snapshot of modal header facts, captured once at open.
#[derive(Debug, Clone)]
pub struct ContentModalHeader {
    pub event_type: String,
    pub event_timestamp_iso: String,
    pub component_name: String,
    pub pg_path: String,
    pub input_size: Option<u64>,
    pub output_size: Option<u64>,
    pub input_mime: Option<String>,
    pub output_mime: Option<String>,
    pub input_available: bool,
    pub output_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentModalTab {
    Input,
    Output,
    Diff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Diffable {
    /// First chunks not yet landed on both sides. Diff tab gray, no
    /// reason chip.
    Pending,
    /// Eligible.
    Ok,
    /// Ineligible with a specific reason.
    NotAvailable(NotDiffableReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotDiffableReason {
    InputUnavailable,
    OutputUnavailable,
    MimeMismatch,
    SizeExceedsDiffCap,
    NoDifferences,
}

#[derive(Debug, Clone, Default)]
pub struct SideBuffer {
    pub loaded: Vec<u8>,
    pub decoded: crate::client::tracer::ContentRender,
    pub fully_loaded: bool,
    pub ceiling_hit: bool,
    pub in_flight: bool,
    pub last_error: Option<String>,
    /// `None` until the first chunk lands and the magic-byte sniff
    /// resolves which `[tracer.ceiling]` knob applies. After
    /// resolution, the inner `Option<usize>` follows the existing
    /// "Some = cap, None = unbounded" convention.
    pub effective_ceiling: Option<Option<usize>>,
    /// Set to `true` when a `spawn_blocking` tabular decode is in
    /// flight. Cleared when the `ContentDecoded` event lands.
    /// Guards against double-spawning if a second completion event
    /// races in (rare; only possible if the modal is reopened mid-
    /// stream).
    pub in_flight_decode: bool,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub tag: similar::ChangeTag,
    pub text: String,
    pub input_line: Option<u32>,
    pub output_line: Option<u32>,
    /// Byte-range segments marking which parts of `text` differ from
    /// the paired counterpart line. Populated for `Delete`/`Insert`
    /// rows produced from a Replace op where N:M pairing matched
    /// them 1:1 — lets the renderer dim the shared prefix/suffix
    /// and emphasize only the actually-changed bytes (e.g. the
    /// `OK → ok` substring inside a 70-byte CSV row). `None` for
    /// Equal rows, hunk headers, and unpaired Delete/Insert rows.
    pub inline_diff: Option<Vec<InlineSegment>>,
}

/// Inline-diff byte range within a paired `DiffLine.text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineSegment {
    /// Byte range into the paired `DiffLine.text`.
    pub range: std::ops::Range<usize>,
    /// True iff this segment's bytes differ from the paired counterpart.
    pub differs: bool,
}

#[derive(Debug, Clone)]
pub struct HunkAnchor {
    /// Index into `DiffRender::lines`.
    pub line_idx: u32,
    pub input_line: u32,
    pub output_line: u32,
}

#[derive(Debug, Clone, Default)]
pub struct DiffRender {
    pub lines: Vec<DiffLine>,
    pub hunks: Vec<HunkAnchor>,
    /// Line indices at which Ctrl+↓ / Ctrl+↑ ("next change") should
    /// land. A stop is the first line of every contiguous run of
    /// non-Equal lines — plus the start of each new Delete in a
    /// Replace block's interleaved pairs, so a CSV body with N
    /// changed rows yields N stops instead of a single hunk anchor.
    pub change_stops: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct ContentModalState {
    pub event_id: i64,
    pub header: ContentModalHeader,
    pub active_tab: ContentModalTab,
    /// Last non-Diff tab the user was on. Used by Save on Diff tab to
    /// decide which side to save.
    pub last_nondiff_tab: ContentModalTab,
    pub diffable: Diffable,
    pub input: SideBuffer,
    pub output: SideBuffer,
    pub diff_cache: Option<DiffRender>,
    /// Combined vertical + horizontal scroll state. `scroll.vertical.offset`
    /// is the scroll row (in lines); `scroll.horizontal_offset` is the
    /// column shift right of the fixed line-number gutter (used by the
    /// renderer to shift wide rows sideways). `last_viewport_rows` and
    /// `last_viewport_body_cols` are written by the renderer each frame;
    /// reducers use them to size page-sized scrolls.
    pub scroll: BidirectionalScrollState,
    pub search: Option<crate::widget::search::SearchState>,
}

// ── Content viewer modal reducers ─────────────────────────────────────────────

/// Byte size of a single streaming fetch chunk.
pub const MODAL_CHUNK_BYTES: usize = 512 * 1024;

/// One pending fetch request — the reducer returns these instead of
/// spawning directly so `AppState` code can route through the
/// `tokio::spawn_local` wiring.
#[derive(Debug, Clone, Copy)]
pub struct ModalFetchRequest {
    pub event_id: i64,
    pub side: crate::client::ContentSide,
    pub offset: usize,
    pub len: usize,
}

/// Open the content viewer modal for `event_id` seeded from `detail`.
/// Lands on `active_tab`; fires the first chunk on the corresponding
/// side (Diff fires both). Returns the list of fetch requests the
/// caller must spawn.
pub fn open_content_modal(
    state: &mut TracerState,
    detail: &crate::client::tracer::ProvenanceEventDetail,
    active_tab: ContentModalTab,
    cfg: &crate::config::TracerCeilingConfig,
) -> Vec<ModalFetchRequest> {
    let (input_mime, output_mime) = mime_pair_from_attributes(&detail.attributes);
    let header = ContentModalHeader {
        event_type: detail.summary.event_type.clone(),
        event_timestamp_iso: detail.summary.event_time_iso.clone(),
        component_name: detail.summary.component_name.clone(),
        pg_path: detail.summary.group_id.clone(),
        input_size: detail.input_size,
        output_size: detail.output_size,
        input_mime,
        output_mime,
        input_available: detail.input_available,
        output_available: detail.output_available,
    };

    // last_nondiff_tab: the side used when Save is pressed on the Diff
    // tab, since Diff itself doesn't correspond to a single fetchable
    // side. Also used by Tab cycling to pick a sensible "previous"
    // non-Diff tab.
    let last_nondiff_tab = match active_tab {
        ContentModalTab::Diff => {
            if header.input_available {
                ContentModalTab::Input
            } else {
                ContentModalTab::Output
            }
        }
        other => other,
    };

    let mut modal = ContentModalState {
        event_id: detail.summary.event_id,
        header,
        active_tab,
        last_nondiff_tab,
        diffable: Diffable::Pending,
        input: SideBuffer::default(),
        output: SideBuffer::default(),
        diff_cache: None,
        scroll: BidirectionalScrollState::default(),
        search: None,
    };

    let mut fired: Vec<ModalFetchRequest> = Vec::new();
    let event_id = modal.event_id;
    let initial_len = match provisional_ceiling(cfg) {
        Some(cap) => MODAL_CHUNK_BYTES.min(cap),
        None => MODAL_CHUNK_BYTES,
    };

    let mut fire = |side: crate::client::ContentSide, buf: &mut SideBuffer| {
        buf.in_flight = true;
        fired.push(ModalFetchRequest {
            event_id,
            side,
            offset: 0,
            len: initial_len,
        });
    };

    match active_tab {
        ContentModalTab::Input if modal.header.input_available => {
            fire(crate::client::ContentSide::Input, &mut modal.input);
        }
        ContentModalTab::Output if modal.header.output_available => {
            fire(crate::client::ContentSide::Output, &mut modal.output);
        }
        ContentModalTab::Diff => {
            if modal.header.input_available {
                fire(crate::client::ContentSide::Input, &mut modal.input);
            }
            if modal.header.output_available {
                fire(crate::client::ContentSide::Output, &mut modal.output);
            }
        }
        _ => {}
    }

    state.content_modal = Some(modal);
    fired
}

fn mime_pair_from_attributes(
    attrs: &[crate::client::tracer::AttributeTriple],
) -> (Option<String>, Option<String>) {
    let row = attrs.iter().find(|a| a.key == "mime.type");
    match row {
        Some(a) => (a.previous.clone(), a.current.clone()),
        None => (None, None),
    }
}

/// Apply a successfully-fetched chunk to the modal buffer.
///
/// Drops chunks whose `event_id` doesn't match the currently-open
/// modal (stale delivery after modal close or event change).
///
/// Uses `TracerCeilingConfig::default()` — `text` 4 MiB, `tabular`
/// 64 MiB, `diff` 16 MiB. Test-only convenience wrapper; runtime
/// callers must use `apply_modal_chunk_with_ceiling` so the user-
/// configured ceilings are honored.
pub fn apply_modal_chunk(
    state: &mut TracerState,
    event_id: i64,
    side: crate::client::ContentSide,
    offset: usize,
    bytes: Vec<u8>,
    eof: bool,
    requested_len: usize,
) {
    apply_modal_chunk_with_ceiling(
        state,
        event_id,
        side,
        offset,
        bytes,
        eof,
        requested_len,
        &crate::config::TracerCeilingConfig::default(),
    );
}

#[allow(clippy::too_many_arguments)]
pub fn apply_modal_chunk_with_ceiling(
    state: &mut TracerState,
    event_id: i64,
    side: crate::client::ContentSide,
    offset: usize,
    bytes: Vec<u8>,
    eof: bool,
    _requested_len: usize,
    cfg: &crate::config::TracerCeilingConfig,
) {
    let Some(modal) = state.content_modal.as_mut() else {
        return;
    };
    if modal.event_id != event_id {
        return;
    }
    let buf = match side {
        crate::client::ContentSide::Input => &mut modal.input,
        crate::client::ContentSide::Output => &mut modal.output,
    };
    if offset != buf.loaded.len() {
        return;
    }
    buf.loaded.extend_from_slice(&bytes);
    buf.in_flight = false;
    buf.last_error = None;
    // Resolve the per-side ceiling on the first chunk (idempotent).
    apply_first_chunk_and_resolve_ceiling(buf, &bytes, cfg);
    // Enforce the now-resolved ceiling: truncate if we've exceeded it.
    let cap = match buf.effective_ceiling {
        Some(resolved) => resolved,
        None => provisional_ceiling(cfg),
    };
    if let Some(c) = cap {
        if buf.loaded.len() > c {
            buf.loaded.truncate(c);
            buf.ceiling_hit = true;
            buf.fully_loaded = true;
        } else if buf.loaded.len() == c {
            buf.ceiling_hit = true;
            buf.fully_loaded = true;
        }
    }
    // For tabular content (detected by magic bytes) we defer decode to
    // fetch completion and run it off-thread via `spawn_blocking`.
    // Text / Hex content stays synchronous — it is cheap and enables
    // incremental rendering.
    let is_tabular = crate::client::tracer::detect_tabular_format(&buf.loaded).is_some();
    if is_tabular {
        // Leave `decoded` as-is (Empty until the off-thread decode lands).
        // The caller (state/mod.rs) will read `in_flight_decode` after this
        // call and spawn the decode when `fully_loaded || ceiling_hit`.
    } else {
        buf.decoded = crate::client::tracer::classify_content(buf.loaded.clone());
    }
    if eof && !buf.fully_loaded {
        buf.fully_loaded = true;
    }
    modal.diff_cache = None;
}

/// Check whether the given side now needs an off-thread tabular decode.
///
/// Returns `Some((event_id, side, bytes))` when ALL of:
/// - the modal is open with a matching `event_id`
/// - the side buffer has tabular magic bytes
/// - the side is now fully loaded (EOF or ceiling hit)
/// - no decode is already in flight for this side
///
/// Sets `in_flight_decode = true` on the buffer before returning so
/// the caller can unconditionally spawn without a second check.
pub fn take_pending_tabular_decode(
    state: &mut TracerState,
    event_id: i64,
    side: crate::client::ContentSide,
) -> Option<(i64, crate::client::ContentSide, Vec<u8>)> {
    let modal = state.content_modal.as_mut()?;
    if modal.event_id != event_id {
        return None;
    }
    let buf = match side {
        crate::client::ContentSide::Input => &mut modal.input,
        crate::client::ContentSide::Output => &mut modal.output,
    };
    if !buf.fully_loaded && !buf.ceiling_hit {
        return None;
    }
    if buf.in_flight_decode {
        return None;
    }
    crate::client::tracer::detect_tabular_format(&buf.loaded)?;
    buf.in_flight_decode = true;
    Some((event_id, side, buf.loaded.clone()))
}

/// Apply the result of an off-thread tabular decode back onto the modal.
///
/// Drops the result when:
/// - the modal is closed or belongs to a different `event_id` (stale)
/// - `in_flight_decode` is false (e.g., the modal was reset mid-decode)
pub fn apply_tabular_decode_result(
    state: &mut TracerState,
    event_id: i64,
    side: crate::client::ContentSide,
    render: crate::client::tracer::ContentRender,
) {
    let Some(modal) = state.content_modal.as_mut() else {
        return;
    };
    if modal.event_id != event_id {
        return;
    }
    let buf = match side {
        crate::client::ContentSide::Input => &mut modal.input,
        crate::client::ContentSide::Output => &mut modal.output,
    };
    if !buf.in_flight_decode {
        return;
    }
    buf.in_flight_decode = false;
    buf.decoded = render;
    modal.diff_cache = None;
}

pub fn apply_modal_chunk_failed(
    state: &mut TracerState,
    event_id: i64,
    side: crate::client::ContentSide,
    _offset: usize,
    error: String,
) {
    let Some(modal) = state.content_modal.as_mut() else {
        return;
    };
    if modal.event_id != event_id {
        return;
    }
    let buf = match side {
        crate::client::ContentSide::Input => &mut modal.input,
        crate::client::ContentSide::Output => &mut modal.output,
    };
    buf.in_flight = false;
    buf.last_error = Some(error);
}

/// Distance in rendered lines from the viewport bottom to the tail at
/// which we fire the next fetch.
const STREAM_LOOKAHEAD_LINES: usize = 100;

/// Scroll to `new_offset` and return any fetch requests the scroll
/// implies (auto-stream).
pub fn content_modal_scroll_to(
    state: &mut TracerState,
    new_offset: usize,
    cfg: &crate::config::TracerCeilingConfig,
) -> Vec<ModalFetchRequest> {
    let Some(modal) = state.content_modal.as_mut() else {
        return Vec::new();
    };
    modal.scroll.vertical.offset = new_offset;

    let side = match modal.active_tab {
        ContentModalTab::Input => crate::client::ContentSide::Input,
        ContentModalTab::Output => crate::client::ContentSide::Output,
        // Diff is capped per `[tracer.ceiling] diff` (default 16 MiB);
        // both sides are loaded eagerly on modal open when Diff is the
        // landing tab. No auto-stream here — ever.
        ContentModalTab::Diff => return Vec::new(),
    };
    let buf = match side {
        crate::client::ContentSide::Input => &modal.input,
        crate::client::ContentSide::Output => &modal.output,
    };
    if buf.fully_loaded || buf.in_flight {
        return Vec::new();
    }

    // Resolve the per-side cap once. Used for both the room-remaining
    // gate (replaces should_fire_next_chunk) and the chunk-length
    // calculation below.
    let cap = match buf.effective_ceiling {
        Some(resolved) => resolved,
        None => provisional_ceiling(cfg),
    };

    let line_count = decoded_line_count(&buf.decoded);
    let viewport_bottom = modal
        .scroll
        .vertical
        .offset
        .saturating_add(modal.scroll.vertical.last_viewport_rows);
    let distance_to_tail = line_count.saturating_sub(viewport_bottom);

    if distance_to_tail > STREAM_LOOKAHEAD_LINES {
        return Vec::new();
    }

    let remaining = match cap {
        Some(c) => c.saturating_sub(buf.loaded.len()),
        None => MODAL_CHUNK_BYTES,
    };
    if remaining == 0 {
        return Vec::new();
    }
    let len = MODAL_CHUNK_BYTES.min(remaining);

    let buf = match side {
        crate::client::ContentSide::Input => &mut modal.input,
        crate::client::ContentSide::Output => &mut modal.output,
    };
    buf.in_flight = true;

    vec![ModalFetchRequest {
        event_id: modal.event_id,
        side,
        offset: buf.loaded.len(),
        len,
    }]
}

fn decoded_line_count(render: &crate::client::tracer::ContentRender) -> usize {
    use crate::client::tracer::ContentRender;
    match render {
        ContentRender::Text { text, .. } => text.lines().count().max(1),
        ContentRender::Hex { first_4k } => first_4k.lines().count().max(1),
        ContentRender::Empty => 1,
        ContentRender::Tabular {
            schema_summary,
            body,
            ..
        } => schema_summary.lines().count() + 1 + body.lines().count(),
    }
}

/// Total scrollable line count for the currently active tab of the modal.
/// For Diff, counts `diff_cache.lines`; for Input/Output, counts `decoded_line_count`.
/// Returns 0 when the modal is not open.
pub fn content_modal_line_count(state: &TracerState) -> usize {
    let Some(modal) = state.content_modal.as_ref() else {
        return 0;
    };
    match modal.active_tab {
        ContentModalTab::Diff => modal
            .diff_cache
            .as_ref()
            .map(|c| c.lines.len())
            .unwrap_or(0),
        ContentModalTab::Input => decoded_line_count(&modal.input.decoded),
        ContentModalTab::Output => decoded_line_count(&modal.output.decoded),
    }
}

/// Scroll the modal by `delta` lines (positive = down, negative = up) and
/// return any auto-stream fetch requests the new position implies.
/// Clamps offset to `[0, line_count.saturating_sub(1)]`.
pub fn content_modal_scroll_by(
    state: &mut TracerState,
    delta: isize,
    cfg: &crate::config::TracerCeilingConfig,
) -> Vec<ModalFetchRequest> {
    let line_count = content_modal_line_count(state);
    let current = state
        .content_modal
        .as_ref()
        .map(|m| m.scroll.vertical.offset)
        .unwrap_or(0);
    let new_offset = if delta >= 0 {
        current
            .saturating_add(delta as usize)
            .min(line_count.saturating_sub(1))
    } else {
        current.saturating_sub((-delta) as usize)
    };
    content_modal_scroll_to(state, new_offset, cfg)
}

/// Shift the horizontal-scroll offset by `delta` columns (positive =
/// right, negative = left). Clamps at 0 on the left; no hard upper
/// bound — the render layer clips whatever the terminal can show.
pub fn content_modal_scroll_horizontal_by(state: &mut TracerState, delta: isize) {
    let Some(modal) = state.content_modal.as_mut() else {
        return;
    };
    modal.scroll.horizontal_offset = if delta >= 0 {
        modal
            .scroll
            .horizontal_offset
            .saturating_add(delta as usize)
    } else {
        modal
            .scroll
            .horizontal_offset
            .saturating_sub((-delta) as usize)
    };
}

/// Set the horizontal-scroll offset to 0 (leftmost column). Used by
/// `Home` and on tab switch.
pub fn content_modal_scroll_horizontal_home(state: &mut TracerState) {
    if let Some(modal) = state.content_modal.as_mut() {
        modal.scroll.horizontal_offset = 0;
    }
}

/// Scroll the modal so that the current search match (if any) is visible.
/// Sets `scroll_offset = match.line_idx` when the match is outside the
/// viewport. Leaves the offset unchanged when the match is already visible
/// to avoid jitter. Returns any auto-stream requests the new position implies.
pub fn content_modal_scroll_to_match(
    state: &mut TracerState,
    cfg: &crate::config::TracerCeilingConfig,
) -> Vec<ModalFetchRequest> {
    let Some(modal) = state.content_modal.as_ref() else {
        return Vec::new();
    };
    let Some(search) = modal.search.as_ref() else {
        return Vec::new();
    };
    let Some(idx) = search.current else {
        return Vec::new();
    };
    let Some(span) = search.matches.get(idx) else {
        return Vec::new();
    };
    let line = span.line_idx;
    let offset = modal.scroll.vertical.offset;
    let rows = modal.scroll.vertical.last_viewport_rows.max(1);
    if line < offset || line >= offset + rows {
        content_modal_scroll_to(state, line, cfg)
    } else {
        Vec::new()
    }
}

// ── Diff eligibility ──────────────────────────────────────────────────────────

/// MIME allowlist helper. Returns true for a single MIME string that
/// belongs to the allowlist (literal or wildcard).
fn mime_is_diffable_single(mime: &str) -> bool {
    const LITERAL: &[&str] = &[
        "application/json",
        "application/xml",
        "application/x-ndjson",
        "application/yaml",
        "text/yaml",
        "text/csv",
        "text/tab-separated-values",
        "text/plain",
    ];
    if LITERAL.contains(&mime) {
        return true;
    }
    if mime.starts_with("text/") {
        return true;
    }
    if mime.starts_with("application/") && (mime.ends_with("+json") || mime.ends_with("+xml")) {
        return true;
    }
    false
}

/// Resolve [`Diffable`] given the modal header and both side buffers.
///
/// Evaluation order: availability → mime pair → size cap → byte-equal.
/// Returns `Pending` when MIME-less UTF-8 fallback cannot decide yet
/// (either side still loading).
pub fn resolve_diffable(
    header: &ContentModalHeader,
    input: &SideBuffer,
    output: &SideBuffer,
    cfg: &crate::config::TracerCeilingConfig,
) -> Diffable {
    use crate::client::tracer::ContentRender;

    if !header.input_available {
        return Diffable::NotAvailable(NotDiffableReason::InputUnavailable);
    }
    if !header.output_available {
        return Diffable::NotAvailable(NotDiffableReason::OutputUnavailable);
    }

    // Tabular sides bypass the mime allowlist — the format tag is authoritative.
    match (&input.decoded, &output.decoded) {
        (
            ContentRender::Tabular {
                format: a,
                decoded_bytes: a_bytes,
                ..
            },
            ContentRender::Tabular {
                format: b,
                decoded_bytes: b_bytes,
                ..
            },
        ) => {
            if a != b {
                return Diffable::NotAvailable(NotDiffableReason::MimeMismatch);
            }
            // Same format: gate on the diff cap (decoded_bytes is the JSON-Lines
            // byte length, the same quantity the existing size check uses).
            if let Some(cap) = cfg.diff
                && (*a_bytes > cap || *b_bytes > cap)
            {
                return Diffable::NotAvailable(NotDiffableReason::SizeExceedsDiffCap);
            }
            return Diffable::Ok;
        }
        (ContentRender::Tabular { .. }, _) | (_, ContentRender::Tabular { .. }) => {
            // One side tabular, the other not — mixed binary/text isn't diffable.
            return Diffable::NotAvailable(NotDiffableReason::MimeMismatch);
        }
        _ => {} // Fall through to existing mime/size checks for non-Tabular pairs.
    }

    let mime_ok = match (header.input_mime.as_deref(), header.output_mime.as_deref()) {
        (Some(i), Some(o)) if i == o && mime_is_diffable_single(i) => true,
        (Some(_), Some(_)) => false,
        (None, None) => match (&input.decoded, &output.decoded) {
            (ContentRender::Text { .. }, ContentRender::Text { .. }) => true,
            (ContentRender::Empty, _) | (_, ContentRender::Empty)
                if input.loaded.is_empty() || output.loaded.is_empty() =>
            {
                if !input.fully_loaded || !output.fully_loaded {
                    // At least one side is empty but not yet fully loaded —
                    // wait for the fetch to complete before deciding.
                    return Diffable::Pending;
                }
                // Both sides fully loaded and at least one is empty.
                // Fall through as diffable so the byte-equality check below
                // resolves to NoDifferences for two empty buffers.
                true
            }
            _ => false,
        },
        _ => false,
    };
    if !mime_ok {
        return Diffable::NotAvailable(NotDiffableReason::MimeMismatch);
    }

    let isize = header.input_size.unwrap_or(input.loaded.len() as u64);
    let osize = header.output_size.unwrap_or(output.loaded.len() as u64);
    let exceeds = match cfg.diff {
        Some(cap) => isize > cap as u64 || osize > cap as u64,
        None => false,
    };
    if exceeds {
        return Diffable::NotAvailable(NotDiffableReason::SizeExceedsDiffCap);
    }

    if input.fully_loaded && output.fully_loaded {
        if input.loaded == output.loaded {
            return Diffable::NotAvailable(NotDiffableReason::NoDifferences);
        }
        return Diffable::Ok;
    }
    Diffable::Pending
}

/// Number of context lines to surround each change group with.
const DIFF_CONTEXT_LINES: usize = 3;

/// Compute the unified-diff cache for `(input, output)`. Line-based.
///
/// For `Replace` ops (block of N old lines becoming M new lines), the
/// `-` and `+` lines are **interleaved pairwise** so each changed
/// position reads as `-old / +new` adjacent — much easier to scan than
/// "all deletes followed by all inserts" when many or every line of a
/// CSV / table-like body has changed. When `N != M`, leftover lines
/// from the longer side are appended after the paired block.
pub fn compute_diff_cache(
    input: &str,
    output: &str,
    cfg: &crate::config::TracerCeilingConfig,
) -> DiffRender {
    // Defensive: honour the configured cap even if resolve_diffable was
    // bypassed or stale. Return an empty render for oversized input.
    if let Some(cap) = cfg.diff
        && (input.len() > cap || output.len() > cap)
    {
        return DiffRender::default();
    }
    let diff = similar::TextDiff::from_lines(input, output);
    // Pre-split both bodies so we can index by line number for the
    // interleave path. `split_inclusive('\n')` keeps the trailing
    // newline, matching `iter_changes` text content; we strip it
    // when building each `DiffLine` so the renderer doesn't draw a
    // dangling empty line per row.
    let old_lines: Vec<&str> = input.split_inclusive('\n').collect();
    let new_lines: Vec<&str> = output.split_inclusive('\n').collect();

    let mut lines: Vec<DiffLine> = Vec::new();
    let mut hunks: Vec<HunkAnchor> = Vec::new();

    for group in diff.grouped_ops(DIFF_CONTEXT_LINES) {
        let hunk_start = lines.len();
        let (input_line, output_line) = group
            .first()
            .map(|op| {
                (
                    op.old_range().start as u32 + 1,
                    op.new_range().start as u32 + 1,
                )
            })
            .unwrap_or((0, 0));
        hunks.push(HunkAnchor {
            line_idx: hunk_start as u32,
            input_line,
            output_line,
        });

        for op in group {
            match op.tag() {
                similar::DiffTag::Equal => {
                    let old_range = op.old_range();
                    let new_range = op.new_range();
                    for (oi, ni) in old_range.zip(new_range) {
                        if let Some(text) = old_lines.get(oi).copied() {
                            lines.push(make_diff_line(
                                similar::ChangeTag::Equal,
                                text,
                                Some((oi + 1) as u32),
                                Some((ni + 1) as u32),
                            ));
                        }
                    }
                }
                similar::DiffTag::Delete => {
                    for oi in op.old_range() {
                        if let Some(text) = old_lines.get(oi).copied() {
                            lines.push(make_diff_line(
                                similar::ChangeTag::Delete,
                                text,
                                Some((oi + 1) as u32),
                                None,
                            ));
                        }
                    }
                }
                similar::DiffTag::Insert => {
                    for ni in op.new_range() {
                        if let Some(text) = new_lines.get(ni).copied() {
                            lines.push(make_diff_line(
                                similar::ChangeTag::Insert,
                                text,
                                None,
                                Some((ni + 1) as u32),
                            ));
                        }
                    }
                }
                similar::DiffTag::Replace => {
                    let old_range = op.old_range();
                    let new_range = op.new_range();
                    let pair_count = old_range.len().min(new_range.len());

                    // Interleaved pairs: -old[k] +new[k] with a
                    // char-level inline diff between each pair so the
                    // renderer can dim unchanged prefix/suffix bytes
                    // and emphasize only the actually-differing
                    // substrings.
                    for k in 0..pair_count {
                        let oi = old_range.start + k;
                        let ni = new_range.start + k;
                        let old_text = old_lines.get(oi).copied();
                        let new_text = new_lines.get(ni).copied();
                        let (old_segments, new_segments) = match (old_text, new_text) {
                            (Some(o), Some(n)) => {
                                let trimmed_old = strip_trailing_newline(o);
                                let trimmed_new = strip_trailing_newline(n);
                                compute_inline_segments(trimmed_old, trimmed_new)
                            }
                            _ => (None, None),
                        };
                        if let Some(text) = old_text {
                            let mut line = make_diff_line(
                                similar::ChangeTag::Delete,
                                text,
                                Some((oi + 1) as u32),
                                None,
                            );
                            line.inline_diff = old_segments;
                            lines.push(line);
                        }
                        if let Some(text) = new_text {
                            let mut line = make_diff_line(
                                similar::ChangeTag::Insert,
                                text,
                                None,
                                Some((ni + 1) as u32),
                            );
                            line.inline_diff = new_segments;
                            lines.push(line);
                        }
                    }
                    // Trailing unmatched old lines (deletions only).
                    for k in pair_count..old_range.len() {
                        let oi = old_range.start + k;
                        if let Some(text) = old_lines.get(oi).copied() {
                            lines.push(make_diff_line(
                                similar::ChangeTag::Delete,
                                text,
                                Some((oi + 1) as u32),
                                None,
                            ));
                        }
                    }
                    // Trailing unmatched new lines (insertions only).
                    for k in pair_count..new_range.len() {
                        let ni = new_range.start + k;
                        if let Some(text) = new_lines.get(ni).copied() {
                            lines.push(make_diff_line(
                                similar::ChangeTag::Insert,
                                text,
                                None,
                                Some((ni + 1) as u32),
                            ));
                        }
                    }
                }
            }
        }
    }

    let change_stops = compute_change_stops(&lines);
    DiffRender {
        lines,
        hunks,
        change_stops,
    }
}

/// Walk `lines` once and collect the line indices where the user
/// expects Ctrl+↓ to land. See [`DiffRender::change_stops`] for the
/// rule. Run alone because `compute_diff_cache` emits lines in multiple
/// branches and a post-pass keeps the rule in one place.
fn compute_change_stops(lines: &[DiffLine]) -> Vec<u32> {
    let mut stops = Vec::new();
    let mut prev: Option<similar::ChangeTag> = None;
    for (idx, line) in lines.iter().enumerate() {
        let stop = match (prev, line.tag) {
            (_, similar::ChangeTag::Equal) => false,
            (None | Some(similar::ChangeTag::Equal), _) => true,
            (Some(similar::ChangeTag::Insert), similar::ChangeTag::Delete) => true,
            _ => false,
        };
        if stop {
            stops.push(idx as u32);
        }
        prev = Some(line.tag);
    }
    stops
}

/// Trim a single trailing `\n` — the counterpart to
/// `split_inclusive('\n')` for char-level inline diffs that should
/// not treat the newline itself as a differing byte.
fn strip_trailing_newline(s: &str) -> &str {
    s.strip_suffix('\n').unwrap_or(s)
}

/// Compute char-level inline-diff segments between two paired lines.
/// Returns `(old_segments, new_segments)` where each segment marks a
/// byte range into its side's text and whether those bytes differ
/// from the paired counterpart. Segments cover the full text (no
/// gaps) so the renderer can walk them without the original string.
///
/// Returns `(None, None)` when the two lines are byte-identical — the
/// caller has no reason to apply inline highlighting in that case
/// (and won't, since a Replace op with identical lines shouldn't
/// arise from a line-diff in the first place).
fn compute_inline_segments(
    old: &str,
    new: &str,
) -> (Option<Vec<InlineSegment>>, Option<Vec<InlineSegment>>) {
    if old == new {
        return (None, None);
    }
    // Char-level diff. Byte offsets are derived via `char_indices()`
    // so the returned segments index into the original &str correctly
    // even for multi-byte UTF-8.
    let diff = similar::TextDiff::from_chars(old, new);
    let old_char_boundaries = char_byte_boundaries(old);
    let new_char_boundaries = char_byte_boundaries(new);

    let mut old_segments: Vec<InlineSegment> = Vec::new();
    let mut new_segments: Vec<InlineSegment> = Vec::new();

    for op in diff.ops() {
        match op.tag() {
            similar::DiffTag::Equal => {
                let old_r = op.old_range();
                let new_r = op.new_range();
                push_segment(
                    &mut old_segments,
                    old_char_boundaries[old_r.start],
                    old_char_boundaries[old_r.end],
                    false,
                );
                push_segment(
                    &mut new_segments,
                    new_char_boundaries[new_r.start],
                    new_char_boundaries[new_r.end],
                    false,
                );
            }
            similar::DiffTag::Delete => {
                let old_r = op.old_range();
                push_segment(
                    &mut old_segments,
                    old_char_boundaries[old_r.start],
                    old_char_boundaries[old_r.end],
                    true,
                );
            }
            similar::DiffTag::Insert => {
                let new_r = op.new_range();
                push_segment(
                    &mut new_segments,
                    new_char_boundaries[new_r.start],
                    new_char_boundaries[new_r.end],
                    true,
                );
            }
            similar::DiffTag::Replace => {
                let old_r = op.old_range();
                let new_r = op.new_range();
                push_segment(
                    &mut old_segments,
                    old_char_boundaries[old_r.start],
                    old_char_boundaries[old_r.end],
                    true,
                );
                push_segment(
                    &mut new_segments,
                    new_char_boundaries[new_r.start],
                    new_char_boundaries[new_r.end],
                    true,
                );
            }
        }
    }

    (Some(old_segments), Some(new_segments))
}

/// Byte offsets for each char boundary in `s`, with a trailing
/// sentinel = `s.len()` so `boundaries[char_idx..char_idx+count]`
/// resolves to the correct byte range for any char slice.
fn char_byte_boundaries(s: &str) -> Vec<usize> {
    let mut v: Vec<usize> = s.char_indices().map(|(b, _)| b).collect();
    v.push(s.len());
    v
}

fn push_segment(into: &mut Vec<InlineSegment>, start: usize, end: usize, differs: bool) {
    if start == end {
        return;
    }
    if let Some(last) = into.last_mut()
        && last.differs == differs
        && last.range.end == start
    {
        // Merge with previous when adjacent + same flag.
        last.range.end = end;
        return;
    }
    into.push(InlineSegment {
        range: start..end,
        differs,
    });
}

/// Strip the trailing newline retained by `split_inclusive` and wrap
/// the line into a `DiffLine` with no inline-diff segments. Callers
/// that want inline highlighting populate `inline_diff` afterwards
/// via [`compute_inline_segments`].
fn make_diff_line(
    tag: similar::ChangeTag,
    raw: &str,
    input_line: Option<u32>,
    output_line: Option<u32>,
) -> DiffLine {
    let mut text = raw.to_string();
    if text.ends_with('\n') {
        text.pop();
    }
    DiffLine {
        tag,
        text,
        input_line,
        output_line,
        inline_diff: None,
    }
}

/// Resolve diffability for the modal and (lazily) populate
/// `diff_cache` when both sides are loaded and `Diffable::Ok`.
/// Idempotent — a cached diff stays cached.
///
/// Designed to be called from any reducer that may have just mutated
/// `modal.input` or `modal.output` (chunk arrival, tab switch).
/// Independent of `modal.active_tab` — users typically open on
/// Input/Output and switch to Diff *after* both chunks have already
/// arrived, by which point no further chunks fire and a tab-gated
/// compute would never trigger.
pub fn resolve_and_cache_diff(
    modal: &mut ContentModalState,
    cfg: &crate::config::TracerCeilingConfig,
) {
    modal.diffable = resolve_diffable(&modal.header, &modal.input, &modal.output, cfg);
    if modal.diffable == Diffable::Ok && modal.diff_cache.is_none() {
        let input_text = side_diff_text(&modal.input);
        let output_text = side_diff_text(&modal.output);
        modal.diff_cache = Some(compute_diff_cache(&input_text, &output_text, cfg));
    }
}

/// Pick the text to feed into the line-based diff for a side. Prefers
/// the already-classified `ContentRender` variant over the raw bytes —
/// `Text` carries pretty-printed JSON (single-line diff would be
/// unreadable), and `Tabular` carries JSON-Lines rows (one record per
/// line, ideal for line-based diffing). `Hex` and `Empty` are never
/// reached in practice because diff eligibility is gated upstream on
/// text-typed or tabular-typed MIME pairs.
fn side_diff_text(buf: &SideBuffer) -> String {
    match &buf.decoded {
        crate::client::tracer::ContentRender::Text { text, .. } => text.clone(),
        crate::client::tracer::ContentRender::Tabular { body, .. } => body.clone(),
        crate::client::tracer::ContentRender::Hex { first_4k } => first_4k.clone(),
        crate::client::tracer::ContentRender::Empty => String::new(),
    }
}

pub fn close_content_modal(state: &mut TracerState) {
    state.content_modal = None;
}

pub fn switch_content_modal_tab(
    state: &mut TracerState,
    new_tab: ContentModalTab,
    cfg: &crate::config::TracerCeilingConfig,
) -> Vec<ModalFetchRequest> {
    let Some(modal) = state.content_modal.as_mut() else {
        return Vec::new();
    };
    if modal.active_tab == new_tab {
        return Vec::new();
    }

    modal.active_tab = new_tab;
    modal.scroll.reset();
    modal.search = None;
    if !matches!(new_tab, ContentModalTab::Diff) {
        modal.last_nondiff_tab = new_tab;
    }

    let event_id = modal.event_id;
    let initial_len = match provisional_ceiling(cfg) {
        Some(cap) => MODAL_CHUNK_BYTES.min(cap),
        None => MODAL_CHUNK_BYTES,
    };
    let mut fired: Vec<ModalFetchRequest> = Vec::new();
    let mut fire = |side: crate::client::ContentSide, buf: &mut SideBuffer| {
        if buf.in_flight || !buf.loaded.is_empty() || buf.fully_loaded {
            return;
        }
        buf.in_flight = true;
        fired.push(ModalFetchRequest {
            event_id,
            side,
            offset: 0,
            len: initial_len,
        });
    };
    match new_tab {
        ContentModalTab::Input if modal.header.input_available => {
            fire(crate::client::ContentSide::Input, &mut modal.input);
        }
        ContentModalTab::Output if modal.header.output_available => {
            fire(crate::client::ContentSide::Output, &mut modal.output);
        }
        ContentModalTab::Diff => {
            if modal.header.input_available {
                fire(crate::client::ContentSide::Input, &mut modal.input);
            }
            if modal.header.output_available {
                fire(crate::client::ContentSide::Output, &mut modal.output);
            }
        }
        _ => {}
    }
    fired
}

pub fn hunk_next(state: &mut TracerState) {
    let Some(modal) = state.content_modal.as_mut() else {
        return;
    };
    let Some(cache) = modal.diff_cache.as_ref() else {
        return;
    };
    let current = modal.scroll.vertical.offset as u32;
    if let Some(&next) = cache.change_stops.iter().find(|&&i| i > current) {
        modal.scroll.vertical.offset = next as usize;
    }
}

pub fn hunk_prev(state: &mut TracerState) {
    let Some(modal) = state.content_modal.as_mut() else {
        return;
    };
    let Some(cache) = modal.diff_cache.as_ref() else {
        return;
    };
    let current = modal.scroll.vertical.offset as u32;
    if let Some(&prev) = cache.change_stops.iter().rev().find(|&&i| i < current) {
        modal.scroll.vertical.offset = prev as usize;
    }
}

/// Render the currently visible tab's contents as plain text for the
/// clipboard. Returns `None` when there is nothing to copy.
pub fn content_modal_copy_text(state: &TracerState) -> Option<String> {
    let modal = state.content_modal.as_ref()?;
    match modal.active_tab {
        ContentModalTab::Input => {
            if modal.input.loaded.is_empty() {
                return None;
            }
            Some(String::from_utf8_lossy(&modal.input.loaded).into_owned())
        }
        ContentModalTab::Output => {
            if modal.output.loaded.is_empty() {
                return None;
            }
            Some(String::from_utf8_lossy(&modal.output.loaded).into_owned())
        }
        ContentModalTab::Diff => {
            let cache = modal.diff_cache.as_ref()?;
            let mut buf = String::new();
            for line in &cache.lines {
                let prefix = match line.tag {
                    similar::ChangeTag::Insert => "+ ",
                    similar::ChangeTag::Delete => "- ",
                    similar::ChangeTag::Equal => "  ",
                };
                buf.push_str(prefix);
                buf.push_str(&line.text);
                buf.push('\n');
            }
            Some(buf)
        }
    }
}

// ── Content modal search helpers ──────────────────────────────────────────────

/// Return the plain-text body for the active tab, used as the search corpus.
/// Returns an empty string when the modal is not open or the tab has no text.
fn content_modal_search_body(modal: &ContentModalState) -> String {
    use crate::client::tracer::ContentRender;
    match modal.active_tab {
        ContentModalTab::Input => match &modal.input.decoded {
            ContentRender::Text { text, .. } => text.clone(),
            ContentRender::Hex { first_4k } => first_4k.clone(),
            ContentRender::Empty => String::new(),
            ContentRender::Tabular {
                schema_summary,
                body,
                ..
            } => format!("{}\n-- schema --\n{}", schema_summary, body),
        },
        ContentModalTab::Output => match &modal.output.decoded {
            ContentRender::Text { text, .. } => text.clone(),
            ContentRender::Hex { first_4k } => first_4k.clone(),
            ContentRender::Empty => String::new(),
            ContentRender::Tabular {
                schema_summary,
                body,
                ..
            } => format!("{}\n-- schema --\n{}", schema_summary, body),
        },
        ContentModalTab::Diff => {
            if let Some(cache) = &modal.diff_cache {
                cache
                    .lines
                    .iter()
                    .map(|l| l.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                String::new()
            }
        }
    }
}

/// Append a character to the live modal search query and recompute matches.
/// No-op if the modal is not open or search input is not active.
pub fn content_modal_search_push(state: &mut TracerState, ch: char) {
    // Check preconditions before taking any borrow.
    {
        let Some(modal) = state.content_modal.as_ref() else {
            return;
        };
        let Some(search) = modal.search.as_ref() else {
            return;
        };
        if !search.input_active {
            return;
        }
    }
    // Compute the body while we only have an immutable borrow.
    let body = content_modal_search_body(state.content_modal.as_ref().unwrap());
    // Now mutate.
    let modal = state.content_modal.as_mut().unwrap();
    let search = modal.search.as_mut().unwrap();
    search.query.push(ch);
    search.matches = crate::widget::search::compute_matches(&body, &search.query);
    search.current = if search.matches.is_empty() {
        None
    } else {
        Some(0)
    };
}

/// Remove the last character from the live modal search query and recompute matches.
/// No-op if the modal is not open or search input is not active.
pub fn content_modal_search_pop(state: &mut TracerState) {
    // Check preconditions before taking any borrow.
    {
        let Some(modal) = state.content_modal.as_ref() else {
            return;
        };
        let Some(search) = modal.search.as_ref() else {
            return;
        };
        if !search.input_active {
            return;
        }
    }
    // Compute body with immutable borrow, then mutate.
    let body = content_modal_search_body(state.content_modal.as_ref().unwrap());
    let modal = state.content_modal.as_mut().unwrap();
    let search = modal.search.as_mut().unwrap();
    search.query.pop();
    search.matches = crate::widget::search::compute_matches(&body, &search.query);
    search.current = if search.matches.is_empty() {
        None
    } else {
        Some(0)
    };
}

/// Commit the current query. If the query is empty, closes search. Otherwise
/// flips `input_active` to false and `committed` to true.
pub fn content_modal_search_commit(state: &mut TracerState) {
    let Some(modal) = state.content_modal.as_mut() else {
        return;
    };
    let Some(search) = modal.search.as_mut() else {
        return;
    };
    if search.query.is_empty() {
        modal.search = None;
        return;
    }
    search.input_active = false;
    search.committed = true;
    if search.current.is_none() && !search.matches.is_empty() {
        search.current = Some(0);
    }
}

/// Cancel search and clear all search state from the modal.
pub fn content_modal_search_cancel(state: &mut TracerState) {
    if let Some(modal) = state.content_modal.as_mut() {
        modal.search = None;
    }
}

// ---------------------------------------------------------------------------
// Ceiling resolution helpers
// ---------------------------------------------------------------------------

/// Resolve the per-side ceiling once enough bytes have landed to
/// sniff the format. Idempotent — returns early if already resolved.
///
/// Called by the modal reducer after each chunk append; the first
/// call decides whether `cfg.text` or `cfg.tabular` applies, based
/// on the buffer's leading magic bytes.
pub fn apply_first_chunk_and_resolve_ceiling(
    side: &mut SideBuffer,
    chunk: &[u8],
    cfg: &crate::config::TracerCeilingConfig,
) {
    if side.effective_ceiling.is_some() {
        return;
    }
    let resolved = match crate::client::tracer::detect_tabular_format(chunk) {
        Some(_) => cfg.tabular,
        None => cfg.text,
    };
    side.effective_ceiling = Some(resolved);
}

/// Provisional ceiling = `max(text, tabular)`. Used for the first
/// chunk fetch before the format is known. If either knob is
/// unbounded (`None`), the provisional ceiling is also unbounded.
pub fn provisional_ceiling(cfg: &crate::config::TracerCeilingConfig) -> Option<usize> {
    match (cfg.text, cfg.tabular) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (None, _) | (_, None) => None,
    }
}

/// Returns `true` iff there is still budget for fetching another chunk
/// on this side, given the per-side resolved ceiling (or the provisional
/// `max(text, tabular)` ceiling if not yet resolved).
///
/// `effective_ceiling = Some(None)` means "decided: unbounded" and
/// `effective_ceiling = None` means "not decided yet". Both cases use
/// the appropriate path via the `match` before falling back to
/// `provisional_ceiling` — `Option::flatten` would conflate the two.
pub fn should_fire_next_chunk(side: &SideBuffer, cfg: &crate::config::TracerCeilingConfig) -> bool {
    let cap = match side.effective_ceiling {
        Some(resolved) => resolved,       // decided (Some=cap, None=unbounded)
        None => provisional_ceiling(cfg), // not decided yet → provisional max
    };
    match cap {
        None => true,                     // unbounded
        Some(c) => side.loaded.len() < c, // room remains
    }
}

#[cfg(test)]
mod tests;
