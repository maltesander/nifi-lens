//! Content viewer modal types, constants, and reducers.
//!
//! Split from `state.rs` to keep that file under a manageable size.
//! All items here are re-exported from `state.rs` via `pub use
//! modal_state::*;` so existing import paths continue to compile
//! unchanged.

use super::state::TracerState;

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
    pub scroll_offset: usize,
    /// Horizontal scroll offset in display columns into the body text
    /// (right of the fixed line-number gutter). Advanced by `←` / `→`;
    /// used by the renderer to shift wide rows sideways.
    pub horizontal_scroll_offset: usize,
    /// Last viewport row count (body area, excluding header/tab-strip/
    /// footer/hint). Written by the renderer each frame.
    pub last_viewport_rows: usize,
    /// Last viewport body-column count (body area minus the gutter).
    /// Written by the renderer each frame; reducers use it to size
    /// horizontal page-size scrolls.
    pub last_viewport_body_cols: usize,
    pub search: Option<crate::widget::search::SearchState>,
}

// ── Content viewer modal reducers ─────────────────────────────────────────────

/// Byte size of a single streaming fetch chunk.
pub const MODAL_CHUNK_BYTES: usize = 512 * 1024;

/// Maximum bytes loaded per side in Diff mode. Fixed; unlike the
/// single-side ceiling this is not configurable.
pub const DIFF_SIZE_CAP_BYTES: u64 = 512 * 1024;

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
    ceiling: Option<usize>,
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
        scroll_offset: 0,
        horizontal_scroll_offset: 0,
        last_viewport_rows: 0,
        last_viewport_body_cols: 0,
        search: None,
    };

    let mut fired: Vec<ModalFetchRequest> = Vec::new();
    let event_id = modal.event_id;
    let initial_len = match ceiling {
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
        None,
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
    ceiling: Option<usize>,
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
    buf.decoded = crate::client::tracer::classify_content(buf.loaded.clone());
    if eof {
        buf.fully_loaded = true;
    }
    if let Some(cap) = ceiling
        && buf.loaded.len() >= cap
    {
        buf.ceiling_hit = true;
        buf.fully_loaded = true;
    }
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
    ceiling: Option<usize>,
) -> Vec<ModalFetchRequest> {
    let Some(modal) = state.content_modal.as_mut() else {
        return Vec::new();
    };
    modal.scroll_offset = new_offset;

    let side = match modal.active_tab {
        ContentModalTab::Input => crate::client::ContentSide::Input,
        ContentModalTab::Output => crate::client::ContentSide::Output,
        // Diff is capped at 512 KiB per side (fixed via DIFF_SIZE_CAP_BYTES);
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

    let line_count = decoded_line_count(&buf.decoded);
    let viewport_bottom = modal.scroll_offset.saturating_add(modal.last_viewport_rows);
    let distance_to_tail = line_count.saturating_sub(viewport_bottom);

    if distance_to_tail > STREAM_LOOKAHEAD_LINES {
        return Vec::new();
    }

    let remaining = match ceiling {
        Some(cap) => cap.saturating_sub(buf.loaded.len()),
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
    ceiling: Option<usize>,
) -> Vec<ModalFetchRequest> {
    let line_count = content_modal_line_count(state);
    let current = state
        .content_modal
        .as_ref()
        .map(|m| m.scroll_offset)
        .unwrap_or(0);
    let new_offset = if delta >= 0 {
        current
            .saturating_add(delta as usize)
            .min(line_count.saturating_sub(1))
    } else {
        current.saturating_sub((-delta) as usize)
    };
    content_modal_scroll_to(state, new_offset, ceiling)
}

/// Shift the horizontal-scroll offset by `delta` columns (positive =
/// right, negative = left). Clamps at 0 on the left; no hard upper
/// bound — the render layer clips whatever the terminal can show.
pub fn content_modal_scroll_horizontal_by(state: &mut TracerState, delta: isize) {
    let Some(modal) = state.content_modal.as_mut() else {
        return;
    };
    modal.horizontal_scroll_offset = if delta >= 0 {
        modal
            .horizontal_scroll_offset
            .saturating_add(delta as usize)
    } else {
        modal
            .horizontal_scroll_offset
            .saturating_sub((-delta) as usize)
    };
}

/// Set the horizontal-scroll offset to 0 (leftmost column). Used by
/// `Home` and on tab switch.
pub fn content_modal_scroll_horizontal_home(state: &mut TracerState) {
    if let Some(modal) = state.content_modal.as_mut() {
        modal.horizontal_scroll_offset = 0;
    }
}

/// Scroll the modal so that the current search match (if any) is visible.
/// Sets `scroll_offset = match.line_idx` when the match is outside the
/// viewport. Leaves the offset unchanged when the match is already visible
/// to avoid jitter. Returns any auto-stream requests the new position implies.
pub fn content_modal_scroll_to_match(
    state: &mut TracerState,
    ceiling: Option<usize>,
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
    let offset = modal.scroll_offset;
    let rows = modal.last_viewport_rows.max(1);
    if line < offset || line >= offset + rows {
        content_modal_scroll_to(state, line, ceiling)
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
) -> Diffable {
    use crate::client::tracer::ContentRender;

    if !header.input_available {
        return Diffable::NotAvailable(NotDiffableReason::InputUnavailable);
    }
    if !header.output_available {
        return Diffable::NotAvailable(NotDiffableReason::OutputUnavailable);
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
    if isize > DIFF_SIZE_CAP_BYTES || osize > DIFF_SIZE_CAP_BYTES {
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
pub fn compute_diff_cache(input: &str, output: &str) -> DiffRender {
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
pub fn resolve_and_cache_diff(modal: &mut ContentModalState) {
    modal.diffable = resolve_diffable(&modal.header, &modal.input, &modal.output);
    if modal.diffable == Diffable::Ok && modal.diff_cache.is_none() {
        let input_text = side_diff_text(&modal.input);
        let output_text = side_diff_text(&modal.output);
        modal.diff_cache = Some(compute_diff_cache(&input_text, &output_text));
    }
}

/// Pick the text to feed into the line-based diff for a side. Prefers
/// the already-classified `ContentRender::Text` (which JSON has been
/// pretty-printed into by `classify_content`) over the raw bytes —
/// otherwise compact JSON would produce a single-line diff that's
/// effectively unreadable in the modal. Non-text content falls back
/// to lossy UTF-8 of the loaded bytes; in practice diff is gated to
/// text-typed sides upstream so the fallback rarely fires.
fn side_diff_text(buf: &SideBuffer) -> String {
    match &buf.decoded {
        crate::client::tracer::ContentRender::Text { text, .. } => text.clone(),
        _ => String::from_utf8_lossy(&buf.loaded).into_owned(),
    }
}

pub fn close_content_modal(state: &mut TracerState) {
    state.content_modal = None;
}

pub fn switch_content_modal_tab(
    state: &mut TracerState,
    new_tab: ContentModalTab,
    ceiling: Option<usize>,
) -> Vec<ModalFetchRequest> {
    let Some(modal) = state.content_modal.as_mut() else {
        return Vec::new();
    };
    if modal.active_tab == new_tab {
        return Vec::new();
    }

    modal.active_tab = new_tab;
    modal.scroll_offset = 0;
    modal.horizontal_scroll_offset = 0;
    modal.search = None;
    if !matches!(new_tab, ContentModalTab::Diff) {
        modal.last_nondiff_tab = new_tab;
    }

    let event_id = modal.event_id;
    let initial_len = match ceiling {
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
    let current = modal.scroll_offset as u32;
    if let Some(&next) = cache.change_stops.iter().find(|&&i| i > current) {
        modal.scroll_offset = next as usize;
    }
}

pub fn hunk_prev(state: &mut TracerState) {
    let Some(modal) = state.content_modal.as_mut() else {
        return;
    };
    let Some(cache) = modal.diff_cache.as_ref() else {
        return;
    };
    let current = modal.scroll_offset as u32;
    if let Some(&prev) = cache.change_stops.iter().rev().find(|&&i| i < current) {
        modal.scroll_offset = prev as usize;
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
        },
        ContentModalTab::Output => match &modal.output.decoded {
            ContentRender::Text { text, .. } => text.clone(),
            ContentRender::Hex { first_4k } => first_4k.clone(),
            ContentRender::Empty => String::new(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn stub_modal(event_id: i64, active: ContentModalTab) -> ContentModalState {
        ContentModalState {
            event_id,
            header: ContentModalHeader {
                event_type: "DROP".into(),
                event_timestamp_iso: "".into(),
                component_name: "x".into(),
                pg_path: "pg".into(),
                input_size: Some(1024),
                output_size: Some(1024),
                input_mime: Some("application/json".into()),
                output_mime: Some("application/json".into()),
                input_available: true,
                output_available: true,
            },
            active_tab: active,
            last_nondiff_tab: match active {
                ContentModalTab::Diff => ContentModalTab::Input,
                other => other,
            },
            diffable: Diffable::Pending,
            input: SideBuffer::default(),
            output: SideBuffer::default(),
            diff_cache: None,
            scroll_offset: 0,
            horizontal_scroll_offset: 0,
            last_viewport_rows: 0,
            last_viewport_body_cols: 0,
            search: None,
        }
    }

    #[test]
    fn apply_modal_chunk_extends_loaded_and_reclassifies() {
        use crate::client::ContentSide;
        use crate::client::tracer::ContentRender;

        let mut modal = stub_modal(42, ContentModalTab::Input);
        modal.input.in_flight = true;
        let mut state = TracerState {
            content_modal: Some(modal),
            ..TracerState::default()
        };

        apply_modal_chunk(
            &mut state,
            42,
            ContentSide::Input,
            0,
            b"hello".to_vec(),
            false,
            5,
        );

        let buf = &state.content_modal.as_ref().unwrap().input;
        assert_eq!(buf.loaded, b"hello");
        assert!(!buf.in_flight);
        assert!(!buf.fully_loaded);
        assert!(matches!(&buf.decoded, ContentRender::Text { text, .. } if text.contains("hello")));
    }

    #[test]
    fn apply_modal_chunk_for_different_event_is_dropped() {
        use crate::client::ContentSide;

        let mut modal = stub_modal(1, ContentModalTab::Input);
        modal.input.in_flight = true;
        let mut state = TracerState {
            content_modal: Some(modal),
            ..TracerState::default()
        };

        apply_modal_chunk(
            &mut state,
            999,
            ContentSide::Input,
            0,
            b"x".to_vec(),
            false,
            1,
        );

        let buf = &state.content_modal.as_ref().unwrap().input;
        assert!(
            buf.loaded.is_empty(),
            "chunk from wrong event must be dropped"
        );
    }

    #[test]
    fn apply_modal_chunk_hits_ceiling() {
        use crate::client::ContentSide;

        let mut modal = stub_modal(1, ContentModalTab::Input);
        modal.input.in_flight = true;
        let mut state = TracerState {
            content_modal: Some(modal),
            ..TracerState::default()
        };

        // Ceiling = 10, chunk = 20 bytes → ceiling_hit
        apply_modal_chunk_with_ceiling(
            &mut state,
            1,
            ContentSide::Input,
            0,
            vec![0u8; 20],
            false,
            20,
            Some(10),
        );

        let buf = &state.content_modal.as_ref().unwrap().input;
        assert!(buf.ceiling_hit);
        assert!(buf.fully_loaded);
    }

    #[test]
    fn apply_modal_chunk_unbounded_ceiling_keeps_streaming() {
        use crate::client::ContentSide;

        let mut modal = stub_modal(1, ContentModalTab::Input);
        modal.input.in_flight = true;
        let mut state = TracerState {
            content_modal: Some(modal),
            ..TracerState::default()
        };

        apply_modal_chunk_with_ceiling(
            &mut state,
            1,
            ContentSide::Input,
            0,
            vec![0u8; 20],
            false,
            20,
            None,
        );

        let buf = &state.content_modal.as_ref().unwrap().input;
        assert!(!buf.ceiling_hit);
        assert!(!buf.fully_loaded);
    }

    #[test]
    fn apply_modal_chunk_failed_sets_last_error() {
        use crate::client::ContentSide;

        let mut modal = stub_modal(1, ContentModalTab::Input);
        modal.input.in_flight = true;
        let mut state = TracerState {
            content_modal: Some(modal),
            ..TracerState::default()
        };

        apply_modal_chunk_failed(
            &mut state,
            1,
            ContentSide::Input,
            0,
            "500 internal".to_string(),
        );

        let buf = &state.content_modal.as_ref().unwrap().input;
        assert!(!buf.in_flight);
        assert_eq!(buf.last_error.as_deref(), Some("500 internal"));
        assert!(!buf.fully_loaded);
    }

    fn stub_event_detail() -> crate::client::tracer::ProvenanceEventDetail {
        use crate::client::tracer::{
            AttributeTriple, ProvenanceEventDetail, ProvenanceEventSummary,
        };
        ProvenanceEventDetail {
            summary: ProvenanceEventSummary {
                event_id: 42,
                event_time_iso: "2026-04-22T13:42:18.231Z".to_string(),
                event_type: "DROP".to_string(),
                component_id: "c-1".to_string(),
                component_name: "UpdateAttribute-enrich".to_string(),
                component_type: "UpdateAttribute".to_string(),
                group_id: "pg-1".to_string(),
                flow_file_uuid: "ff-uuid".to_string(),
                relationship: None,
                details: None,
            },
            attributes: vec![AttributeTriple {
                key: "mime.type".to_string(),
                previous: Some("application/json".to_string()),
                current: Some("application/json".to_string()),
            }],
            transit_uri: None,
            input_available: true,
            output_available: true,
            input_size: Some(2400),
            output_size: Some(2800),
        }
    }

    #[test]
    fn content_modal_opens_with_active_side_loading() {
        use crate::client::ContentSide;

        let mut state = TracerState::default();
        let detail = stub_event_detail();

        let fired = open_content_modal(
            &mut state,
            &detail,
            ContentModalTab::Input,
            Some(4 * 1024 * 1024),
        );
        let modal = state.content_modal.as_ref().expect("modal open");
        assert_eq!(modal.event_id, 42);
        assert_eq!(modal.active_tab, ContentModalTab::Input);
        assert_eq!(modal.diffable, Diffable::Pending);
        assert!(modal.input.in_flight);
        assert!(!modal.output.in_flight, "output lazy-fetched on switch");
        assert_eq!(modal.header.input_mime.as_deref(), Some("application/json"));
        assert_eq!(
            modal.header.output_mime.as_deref(),
            Some("application/json")
        );
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].side, ContentSide::Input);
        assert_eq!(fired[0].offset, 0);
        assert_eq!(fired[0].len, 524_288);
    }

    #[test]
    fn content_modal_open_with_tiny_ceiling_clamps_first_chunk() {
        use crate::client::ContentSide;

        let detail = stub_event_detail();
        let mut state = TracerState::default();
        let fired = open_content_modal(&mut state, &detail, ContentModalTab::Input, Some(100_000));
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].side, ContentSide::Input);
        assert_eq!(fired[0].len, 100_000);
    }

    #[test]
    fn content_modal_scroll_triggers_fetch_near_tail() {
        use crate::client::ContentSide;
        use crate::client::tracer::ContentRender;

        let text = "a\n".repeat(1000);
        let loaded_bytes = text.clone().into_bytes();
        let len = loaded_bytes.len();
        let mut modal = stub_modal(1, ContentModalTab::Input);
        modal.input.loaded = loaded_bytes;
        modal.input.decoded = ContentRender::Text {
            text,
            pretty_printed: false,
        };
        modal.last_viewport_rows = 30;
        let state = TracerState {
            content_modal: Some(modal),
            ..TracerState::default()
        };
        let mut state = state;

        // Viewport bottom = 965 + 30 = 995 → distance to tail (1000) = 5 < 100.
        let fired = content_modal_scroll_to(&mut state, 965, Some(4 * 1024 * 1024));
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].side, ContentSide::Input);
        assert_eq!(fired[0].offset, len);
    }

    #[test]
    fn content_modal_scroll_does_not_fetch_when_fully_loaded() {
        use crate::client::tracer::ContentRender;

        let text = "a\n".repeat(1000);
        let mut modal = stub_modal(1, ContentModalTab::Input);
        modal.input.loaded = text.as_bytes().to_vec();
        modal.input.decoded = ContentRender::Text {
            text,
            pretty_printed: false,
        };
        modal.input.fully_loaded = true;
        modal.last_viewport_rows = 30;
        let mut state = TracerState {
            content_modal: Some(modal),
            ..TracerState::default()
        };
        let fired = content_modal_scroll_to(&mut state, 965, Some(4 * 1024 * 1024));
        assert!(fired.is_empty());
    }

    #[test]
    fn content_modal_scroll_does_not_fetch_when_in_flight() {
        use crate::client::tracer::ContentRender;

        let text = "a\n".repeat(1000);
        let mut modal = stub_modal(1, ContentModalTab::Input);
        modal.input.loaded = text.as_bytes().to_vec();
        modal.input.decoded = ContentRender::Text {
            text,
            pretty_printed: false,
        };
        modal.input.in_flight = true;
        modal.last_viewport_rows = 30;
        let mut state = TracerState {
            content_modal: Some(modal),
            ..TracerState::default()
        };
        let fired = content_modal_scroll_to(&mut state, 965, Some(4 * 1024 * 1024));
        assert!(fired.is_empty());
    }

    #[test]
    fn content_modal_scroll_respects_ceiling() {
        use crate::client::tracer::ContentRender;

        let text = "a\n".repeat(1000);
        let loaded_bytes = text.as_bytes().to_vec();
        let loaded_len = loaded_bytes.len();
        let mut modal = stub_modal(1, ContentModalTab::Input);
        modal.input.loaded = loaded_bytes;
        modal.input.decoded = ContentRender::Text {
            text,
            pretty_printed: false,
        };
        modal.last_viewport_rows = 30;
        let mut state = TracerState {
            content_modal: Some(modal),
            ..TracerState::default()
        };
        // Ceiling = loaded_len + 48576 bytes remaining, chunk = min(512K, 48576)
        let fired = content_modal_scroll_to(&mut state, 965, Some(loaded_len + 48_576));
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].len, 48_576);
    }

    // ── resolve_diffable helpers ──────────────────────────────────────────────

    fn header_with_mime(i: &str, o: &str, isize: u64, osize: u64) -> ContentModalHeader {
        ContentModalHeader {
            event_type: "DROP".into(),
            event_timestamp_iso: "".into(),
            component_name: "x".into(),
            pg_path: "pg".into(),
            input_size: Some(isize),
            output_size: Some(osize),
            input_mime: Some(i.into()),
            output_mime: Some(o.into()),
            input_available: true,
            output_available: true,
        }
    }

    fn text_buffer(t: &str) -> SideBuffer {
        use crate::client::tracer::ContentRender;
        SideBuffer {
            loaded: t.as_bytes().to_vec(),
            decoded: ContentRender::Text {
                text: t.to_string(),
                pretty_printed: false,
            },
            fully_loaded: true,
            ceiling_hit: false,
            in_flight: false,
            last_error: None,
        }
    }

    // ── resolve_diffable tests ────────────────────────────────────────────────

    #[test]
    fn diffable_ok_when_mime_equal_and_allowlisted() {
        let header = header_with_mime("application/json", "application/json", 1024, 1024);
        let (input, output) = (text_buffer("a"), text_buffer("b"));
        assert_eq!(resolve_diffable(&header, &input, &output), Diffable::Ok);
    }

    #[test]
    fn diffable_wildcard_text_star() {
        let header = header_with_mime("text/html", "text/html", 1024, 1024);
        let (input, output) = (text_buffer("a"), text_buffer("b"));
        assert_eq!(resolve_diffable(&header, &input, &output), Diffable::Ok);
    }

    #[test]
    fn diffable_wildcard_structured_json() {
        let header = header_with_mime(
            "application/vnd.api+json",
            "application/vnd.api+json",
            1024,
            1024,
        );
        let (input, output) = (text_buffer("a"), text_buffer("b"));
        assert_eq!(resolve_diffable(&header, &input, &output), Diffable::Ok);
    }

    #[test]
    fn diffable_mime_mismatch() {
        let header = header_with_mime("application/json", "text/csv", 1024, 1024);
        let (input, output) = (text_buffer("a"), text_buffer("b"));
        assert_eq!(
            resolve_diffable(&header, &input, &output),
            Diffable::NotAvailable(NotDiffableReason::MimeMismatch)
        );
    }

    #[test]
    fn diffable_utf8_fallback_when_no_mime() {
        let mut header = header_with_mime("", "", 1024, 1024);
        header.input_mime = None;
        header.output_mime = None;
        let (input, output) = (text_buffer("a"), text_buffer("b"));
        assert_eq!(resolve_diffable(&header, &input, &output), Diffable::Ok);
    }

    #[test]
    fn diffable_size_exceeds_cap() {
        let header = header_with_mime("application/json", "application/json", 600_000, 1024);
        let (input, output) = (text_buffer("a"), text_buffer("b"));
        assert_eq!(
            resolve_diffable(&header, &input, &output),
            Diffable::NotAvailable(NotDiffableReason::SizeExceedsDiffCap)
        );
    }

    #[test]
    fn diffable_no_differences_when_bytes_equal() {
        let header = header_with_mime("application/json", "application/json", 10, 10);
        let (input, output) = (text_buffer("same"), text_buffer("same"));
        assert_eq!(
            resolve_diffable(&header, &input, &output),
            Diffable::NotAvailable(NotDiffableReason::NoDifferences)
        );
    }

    #[test]
    fn diffable_input_unavailable() {
        let mut header = header_with_mime("application/json", "application/json", 10, 10);
        header.input_available = false;
        let (input, output) = (SideBuffer::default(), text_buffer("x"));
        assert_eq!(
            resolve_diffable(&header, &input, &output),
            Diffable::NotAvailable(NotDiffableReason::InputUnavailable)
        );
    }

    #[test]
    fn diffable_no_mime_both_empty_fully_loaded_is_no_differences() {
        let mut header = header_with_mime("", "", 0, 0);
        header.input_mime = None;
        header.output_mime = None;
        let empty = SideBuffer {
            loaded: Vec::new(),
            decoded: crate::client::tracer::ContentRender::Empty,
            fully_loaded: true,
            ceiling_hit: false,
            in_flight: false,
            last_error: None,
        };
        assert_eq!(
            resolve_diffable(&header, &empty, &empty),
            Diffable::NotAvailable(NotDiffableReason::NoDifferences)
        );
    }

    #[test]
    fn compute_diff_cache_produces_lines_and_hunks() {
        let input = "line a\nline b\nline c\n";
        let output = "line a\nline B\nline c\n";
        let render = compute_diff_cache(input, output);
        let inserts = render
            .lines
            .iter()
            .filter(|l| matches!(l.tag, similar::ChangeTag::Insert))
            .count();
        let deletes = render
            .lines
            .iter()
            .filter(|l| matches!(l.tag, similar::ChangeTag::Delete))
            .count();
        assert!(inserts >= 1, "expected at least one insert");
        assert!(deletes >= 1, "expected at least one delete");
        assert_eq!(render.hunks.len(), 1);
    }

    #[test]
    fn close_content_modal_clears_state() {
        let mut state = TracerState {
            content_modal: Some(stub_modal(1, ContentModalTab::Input)),
            ..TracerState::default()
        };
        close_content_modal(&mut state);
        assert!(state.content_modal.is_none());
    }

    #[test]
    fn switch_tab_updates_active_and_clears_scroll() {
        let mut state = TracerState {
            content_modal: Some(stub_modal(1, ContentModalTab::Input)),
            ..TracerState::default()
        };
        let fired =
            switch_content_modal_tab(&mut state, ContentModalTab::Output, Some(4 * 1024 * 1024));
        let modal = state.content_modal.as_ref().unwrap();
        assert_eq!(modal.active_tab, ContentModalTab::Output);
        assert_eq!(modal.scroll_offset, 0);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].side, crate::client::ContentSide::Output);
    }

    #[test]
    fn switch_tab_to_already_loaded_side_does_not_refetch() {
        let mut modal = stub_modal(1, ContentModalTab::Input);
        modal.output.loaded = b"already".to_vec();
        modal.output.fully_loaded = true;
        let mut state = TracerState {
            content_modal: Some(modal),
            ..TracerState::default()
        };
        let fired =
            switch_content_modal_tab(&mut state, ContentModalTab::Output, Some(4 * 1024 * 1024));
        assert!(fired.is_empty());
    }

    #[test]
    fn hunk_next_advances_scroll_to_next_change_stop() {
        let mut modal = stub_modal(1, ContentModalTab::Diff);
        modal.diff_cache = Some(DiffRender {
            lines: Vec::new(),
            hunks: Vec::new(),
            change_stops: vec![10, 50],
        });
        modal.scroll_offset = 5;
        let mut state = TracerState {
            content_modal: Some(modal),
            ..TracerState::default()
        };
        hunk_next(&mut state);
        assert_eq!(state.content_modal.as_ref().unwrap().scroll_offset, 10);
        hunk_next(&mut state);
        assert_eq!(state.content_modal.as_ref().unwrap().scroll_offset, 50);
        hunk_next(&mut state);
        assert_eq!(state.content_modal.as_ref().unwrap().scroll_offset, 50);
    }

    #[test]
    fn hunk_prev_moves_backward() {
        let mut modal = stub_modal(1, ContentModalTab::Diff);
        modal.diff_cache = Some(DiffRender {
            lines: Vec::new(),
            hunks: Vec::new(),
            change_stops: vec![10, 50],
        });
        modal.scroll_offset = 75;
        let mut state = TracerState {
            content_modal: Some(modal),
            ..TracerState::default()
        };
        hunk_prev(&mut state);
        assert_eq!(state.content_modal.as_ref().unwrap().scroll_offset, 50);
        hunk_prev(&mut state);
        assert_eq!(state.content_modal.as_ref().unwrap().scroll_offset, 10);
        hunk_prev(&mut state);
        assert_eq!(state.content_modal.as_ref().unwrap().scroll_offset, 10);
    }

    /// Root cause for the CSV-diff bug: a body where every line changed
    /// collapses into a single `grouped_ops` hunk at line 0, so the old
    /// hunk-only navigation had no forward targets. `change_stops`
    /// must produce one stop per interleaved Replace pair so Ctrl+↓
    /// keeps advancing.
    #[test]
    fn change_stops_cover_every_replace_pair_for_csv_body() {
        let mut input = String::from("id,value,status\n");
        let mut output = String::from("id,value,status\n");
        for i in 0..10 {
            input.push_str(&format!("{i},42,OK\n"));
            output.push_str(&format!("{i},42,ok\n"));
        }
        let render = compute_diff_cache(&input, &output);
        assert_eq!(render.hunks.len(), 1, "grouped_ops collapses to 1 hunk");
        assert_eq!(
            render.change_stops.len(),
            10,
            "one change stop per modified row"
        );
    }

    #[test]
    fn copy_returns_raw_bytes_on_input_tab() {
        let mut modal = stub_modal(1, ContentModalTab::Input);
        modal.input.loaded = b"hello".to_vec();
        let state = TracerState {
            content_modal: Some(modal),
            ..TracerState::default()
        };
        let text = content_modal_copy_text(&state).unwrap();
        assert_eq!(text, "hello");
    }

    #[test]
    fn copy_returns_rendered_diff_on_diff_tab() {
        let mut modal = stub_modal(1, ContentModalTab::Diff);
        modal.diff_cache = Some(DiffRender {
            lines: vec![
                DiffLine {
                    tag: similar::ChangeTag::Delete,
                    text: "x".into(),
                    input_line: Some(1),
                    output_line: None,
                    inline_diff: None,
                },
                DiffLine {
                    tag: similar::ChangeTag::Insert,
                    text: "y".into(),
                    input_line: None,
                    output_line: Some(1),
                    inline_diff: None,
                },
            ],
            hunks: Vec::new(),
            change_stops: Vec::new(),
        });
        let state = TracerState {
            content_modal: Some(modal),
            ..TracerState::default()
        };
        let text = content_modal_copy_text(&state).unwrap();
        assert!(text.contains("- x"));
        assert!(text.contains("+ y"));
    }

    #[test]
    fn content_modal_scroll_down_fires_auto_stream_request() {
        use crate::client::ContentSide;
        use crate::client::tracer::ContentRender;

        // Build a modal with 200 lines loaded, not fully loaded, and a
        // viewport of 30 rows. Scrolling near the tail triggers a fetch.
        let text = "a\n".repeat(200);
        let loaded_bytes = text.clone().into_bytes();
        let len = loaded_bytes.len();
        let mut modal = stub_modal(1, ContentModalTab::Input);
        modal.input.loaded = loaded_bytes;
        modal.input.decoded = ContentRender::Text {
            text,
            pretty_printed: false,
        };
        modal.last_viewport_rows = 30;
        let mut state = TracerState {
            content_modal: Some(modal),
            ..TracerState::default()
        };

        // Scroll to line 165: viewport bottom = 195, distance to tail (200) = 5 < 100.
        let fired = content_modal_scroll_by(&mut state, 165, Some(4 * 1024 * 1024));
        assert_eq!(
            fired.len(),
            1,
            "auto-stream should fire when near tail after scroll"
        );
        assert_eq!(fired[0].side, ContentSide::Input);
        assert_eq!(fired[0].offset, len);
    }

    #[test]
    fn content_modal_scroll_up_clamped_at_zero() {
        use crate::client::tracer::ContentRender;

        // Fully loaded so no auto-stream fires; 100 lines, scroll at 0.
        let text = "line\n".repeat(100);
        let mut modal = stub_modal(1, ContentModalTab::Input);
        modal.input.fully_loaded = true;
        modal.input.decoded = ContentRender::Text {
            text,
            pretty_printed: false,
        };
        modal.scroll_offset = 0;
        modal.last_viewport_rows = 20;
        let mut state = TracerState {
            content_modal: Some(modal),
            ..TracerState::default()
        };
        let fired = content_modal_scroll_by(&mut state, -10, Some(4 * 1024 * 1024));
        assert!(fired.is_empty(), "no fetch when fully loaded");
        assert_eq!(
            state.content_modal.as_ref().unwrap().scroll_offset,
            0,
            "scroll must not go below 0"
        );
    }

    #[test]
    fn content_modal_scroll_to_end_jumps_to_tail() {
        use crate::client::tracer::ContentRender;

        let text = "line\n".repeat(50);
        let mut modal = stub_modal(1, ContentModalTab::Input);
        modal.input.fully_loaded = true;
        modal.input.decoded = ContentRender::Text {
            text,
            pretty_printed: false,
        };
        modal.last_viewport_rows = 10;
        let mut state = TracerState {
            content_modal: Some(modal),
            ..TracerState::default()
        };
        let line_count = content_modal_line_count(&state);
        let fired = content_modal_scroll_to(&mut state, line_count.saturating_sub(1), None);
        assert!(fired.is_empty(), "fully loaded: no fetch on Last");
        assert_eq!(
            state.content_modal.as_ref().unwrap().scroll_offset,
            line_count.saturating_sub(1)
        );
    }

    #[test]
    fn content_modal_page_down_advances_by_viewport() {
        use crate::client::tracer::ContentRender;

        let text = "row\n".repeat(100);
        let mut modal = stub_modal(1, ContentModalTab::Input);
        modal.input.fully_loaded = true;
        modal.input.decoded = ContentRender::Text {
            text,
            pretty_printed: false,
        };
        modal.scroll_offset = 10;
        modal.last_viewport_rows = 20;
        let mut state = TracerState {
            content_modal: Some(modal),
            ..TracerState::default()
        };
        let fired = content_modal_scroll_by(&mut state, 20, Some(4 * 1024 * 1024));
        assert!(fired.is_empty(), "fully loaded: no fetch");
        assert_eq!(
            state.content_modal.as_ref().unwrap().scroll_offset,
            30,
            "page-down by viewport rows"
        );
    }

    #[test]
    fn content_modal_search_push_recomputes_matches() {
        use crate::client::tracer::ContentRender;
        use crate::widget::search::SearchState;

        let body = "error: foo\nok\nerror: bar";
        let mut modal = stub_modal(1, ContentModalTab::Input);
        modal.input.decoded = ContentRender::Text {
            text: body.to_owned(),
            pretty_printed: false,
        };
        modal.search = Some(SearchState {
            input_active: true,
            ..Default::default()
        });
        let mut state = TracerState {
            content_modal: Some(modal),
            ..TracerState::default()
        };

        content_modal_search_push(&mut state, 'e');
        content_modal_search_push(&mut state, 'r');
        content_modal_search_push(&mut state, 'r');

        let s = state
            .content_modal
            .as_ref()
            .unwrap()
            .search
            .as_ref()
            .unwrap();
        assert_eq!(s.query, "err");
        assert_eq!(s.matches.len(), 2);
    }

    #[test]
    fn content_modal_search_pop_shrinks_query() {
        use crate::client::tracer::ContentRender;
        use crate::widget::search::SearchState;

        let body = "hello world";
        let mut modal = stub_modal(1, ContentModalTab::Input);
        modal.input.decoded = ContentRender::Text {
            text: body.to_owned(),
            pretty_printed: false,
        };
        modal.search = Some(SearchState {
            query: "wor".to_owned(),
            input_active: true,
            matches: vec![],
            ..Default::default()
        });
        let mut state = TracerState {
            content_modal: Some(modal),
            ..TracerState::default()
        };

        content_modal_search_pop(&mut state);

        let s = state
            .content_modal
            .as_ref()
            .unwrap()
            .search
            .as_ref()
            .unwrap();
        assert_eq!(s.query, "wo");
    }

    #[test]
    fn content_modal_search_commit_clears_search_on_empty_query() {
        use crate::widget::search::SearchState;

        let mut modal = stub_modal(1, ContentModalTab::Input);
        modal.search = Some(SearchState {
            input_active: true,
            ..Default::default()
        });
        let mut state = TracerState {
            content_modal: Some(modal),
            ..TracerState::default()
        };

        content_modal_search_commit(&mut state);

        assert!(
            state.content_modal.as_ref().unwrap().search.is_none(),
            "empty query commit should clear search"
        );
    }

    #[test]
    fn content_modal_scroll_to_match_scrolls_to_visible_line() {
        use crate::client::tracer::ContentRender;
        use crate::widget::search::{MatchSpan, SearchState};

        let text = "a\n".repeat(100);
        let mut modal = stub_modal(1, ContentModalTab::Input);
        modal.input.fully_loaded = true;
        modal.input.decoded = ContentRender::Text {
            text,
            pretty_printed: false,
        };
        // Match at line 50; viewport at 0..10. Scroll must jump to line 50.
        modal.scroll_offset = 0;
        modal.last_viewport_rows = 10;
        modal.search = Some(SearchState {
            query: "a".to_owned(),
            input_active: false,
            committed: true,
            matches: vec![MatchSpan {
                line_idx: 50,
                byte_start: 0,
                byte_end: 1,
            }],
            current: Some(0),
        });
        let mut state = TracerState {
            content_modal: Some(modal),
            ..TracerState::default()
        };

        let fired = content_modal_scroll_to_match(&mut state, None);
        // fully_loaded — no fetch fires; scroll offset updated.
        assert!(fired.is_empty(), "no fetch for fully loaded content");
        assert_eq!(
            state.content_modal.as_ref().unwrap().scroll_offset,
            50,
            "scroll must jump to match line"
        );
    }

    // ── resolve_and_cache_diff tests ──────────────────────────────────────────

    #[test]
    fn resolve_and_cache_diff_populates_cache_when_both_sides_loaded_and_ok() {
        let mut modal = stub_modal(1, ContentModalTab::Input);
        modal.input = text_buffer("line a\nline b\nline c\n");
        modal.output = text_buffer("line a\nline B\nline c\n");
        // Active tab is Input — cache must still populate (this is the
        // bug the helper fixes).
        resolve_and_cache_diff(&mut modal);
        assert_eq!(modal.diffable, Diffable::Ok);
        assert!(modal.diff_cache.is_some());
        let cache = modal.diff_cache.as_ref().unwrap();
        assert!(!cache.lines.is_empty());
        assert!(!cache.hunks.is_empty());
    }

    #[test]
    fn resolve_and_cache_diff_is_idempotent() {
        let mut modal = stub_modal(1, ContentModalTab::Input);
        modal.input = text_buffer("alpha\nbeta\n");
        modal.output = text_buffer("alpha\nBETA\n");
        resolve_and_cache_diff(&mut modal);
        let first_ptr = modal.diff_cache.as_ref().unwrap() as *const DiffRender;
        // Second call must not reallocate — same Box address.
        resolve_and_cache_diff(&mut modal);
        let second_ptr = modal.diff_cache.as_ref().unwrap() as *const DiffRender;
        assert_eq!(first_ptr, second_ptr, "cached diff must not be recomputed");
    }

    #[test]
    fn resolve_and_cache_diff_skips_when_pending() {
        let mut modal = stub_modal(1, ContentModalTab::Input);
        // Input loaded but output not (in_flight) → resolve_diffable
        // returns Pending → cache stays None.
        modal.input = text_buffer("anything");
        modal.output.in_flight = true;
        resolve_and_cache_diff(&mut modal);
        assert_eq!(modal.diffable, Diffable::Pending);
        assert!(modal.diff_cache.is_none());
    }

    #[test]
    fn compute_diff_cache_interleaves_replace_pairs() {
        // Three CSV-shaped lines, all changed in place — the classic
        // "every line is a delete" case that the interleave fix targets.
        let input = "a,1,OK\nb,2,WARN\nc,3,OK\n";
        let output = "a,1,ok\nb,2,warn\nc,3,ok\n";
        let render = compute_diff_cache(input, output);

        // Expected order: -a OK, +a ok, -b WARN, +b warn, -c OK, +c ok
        let tags: Vec<similar::ChangeTag> = render.lines.iter().map(|l| l.tag).collect();
        assert_eq!(
            tags,
            vec![
                similar::ChangeTag::Delete,
                similar::ChangeTag::Insert,
                similar::ChangeTag::Delete,
                similar::ChangeTag::Insert,
                similar::ChangeTag::Delete,
                similar::ChangeTag::Insert,
            ],
            "delete and insert lines must interleave per row, got {tags:?}"
        );

        // Spot-check that paired lines belong to the same row.
        assert!(render.lines[0].text.contains("OK"));
        assert!(render.lines[1].text.contains("ok"));
        assert!(render.lines[2].text.contains("WARN"));
        assert!(render.lines[3].text.contains("warn"));
    }

    #[test]
    fn compute_diff_cache_replace_with_unequal_lengths_appends_remainder() {
        // Two old lines replaced by three new lines — first two pair,
        // the third trails as a pure insert.
        let input = "alpha\nbeta\n";
        let output = "ALPHA\nBETA\nGAMMA\n";
        let render = compute_diff_cache(input, output);

        let tags: Vec<similar::ChangeTag> = render.lines.iter().map(|l| l.tag).collect();
        assert_eq!(
            tags,
            vec![
                similar::ChangeTag::Delete, // alpha
                similar::ChangeTag::Insert, // ALPHA
                similar::ChangeTag::Delete, // beta
                similar::ChangeTag::Insert, // BETA
                similar::ChangeTag::Insert, // GAMMA (trailing remainder)
            ],
            "trailing inserts must follow paired interleave, got {tags:?}"
        );
    }

    #[test]
    fn compute_inline_segments_marks_only_differing_bytes() {
        // CSV-shaped: only the `OK`/`ok` substring differs.
        let old = "SENSOR-0000,15.0,OK,zone-0";
        let new = "SENSOR-0000,15.0,ok,zone-0";
        let (old_segs, new_segs) = compute_inline_segments(old, new);
        let old_segs = old_segs.expect("segments computed");
        let new_segs = new_segs.expect("segments computed");

        // Coverage: segments must tile the full string length without gaps.
        let old_covered: usize = old_segs.iter().map(|s| s.range.end - s.range.start).sum();
        let new_covered: usize = new_segs.iter().map(|s| s.range.end - s.range.start).sum();
        assert_eq!(old_covered, old.len());
        assert_eq!(new_covered, new.len());

        // Changed bytes in old should be exactly "OK" (position 17..19
        // in "SENSOR-0000,15.0,OK,zone-0").
        let differing_old: String = old_segs
            .iter()
            .filter(|s| s.differs)
            .map(|s| &old[s.range.clone()])
            .collect();
        assert_eq!(differing_old, "OK");
        let differing_new: String = new_segs
            .iter()
            .filter(|s| s.differs)
            .map(|s| &new[s.range.clone()])
            .collect();
        assert_eq!(differing_new, "ok");
    }

    #[test]
    fn compute_inline_segments_returns_none_for_identical_inputs() {
        let (old_segs, new_segs) = compute_inline_segments("same", "same");
        assert!(old_segs.is_none());
        assert!(new_segs.is_none());
    }

    #[test]
    fn compute_diff_cache_populates_inline_diff_on_replace_pairs() {
        // Two CSV-shaped rows, every one changed — classic Replace op.
        let input = "a,1,OK\nb,2,WARN\n";
        let output = "a,1,ok\nb,2,warn\n";
        let render = compute_diff_cache(input, output);

        // Every Delete/Insert row must carry inline_diff segments.
        for line in &render.lines {
            match line.tag {
                similar::ChangeTag::Delete | similar::ChangeTag::Insert => {
                    assert!(
                        line.inline_diff.is_some(),
                        "paired Replace rows must carry inline_diff: {line:?}"
                    );
                    let segs = line.inline_diff.as_ref().unwrap();
                    // At least one segment must mark a differing byte range.
                    assert!(
                        segs.iter().any(|s| s.differs),
                        "expected differing segment, got {segs:?}"
                    );
                    // And at least one unchanged segment (the shared prefix).
                    assert!(
                        segs.iter().any(|s| !s.differs),
                        "expected unchanged segment (shared prefix), got {segs:?}"
                    );
                }
                _ => {}
            }
        }
    }

    #[test]
    fn resolve_and_cache_diff_uses_pretty_printed_text_for_json() {
        // Compact JSON in the raw bytes…
        let input_compact = br#"[{"id":1,"v":"a"},{"id":2,"v":"b"}]"#.to_vec();
        let output_compact = br#"[{"id":1,"v":"A"},{"id":2,"v":"B"}]"#.to_vec();
        // …but classify_content pretty-prints into the decoded variant.
        let input_decoded = crate::client::tracer::classify_content(input_compact.clone());
        let output_decoded = crate::client::tracer::classify_content(output_compact.clone());

        let mut modal = stub_modal(1, ContentModalTab::Input);
        modal.input = SideBuffer {
            loaded: input_compact,
            decoded: input_decoded,
            fully_loaded: true,
            ceiling_hit: false,
            in_flight: false,
            last_error: None,
        };
        modal.output = SideBuffer {
            loaded: output_compact,
            decoded: output_decoded,
            fully_loaded: true,
            ceiling_hit: false,
            in_flight: false,
            last_error: None,
        };

        resolve_and_cache_diff(&mut modal);
        let cache = modal
            .diff_cache
            .as_ref()
            .expect("diff cache populated for diffable JSON");

        // If the diff used the compact bytes, every line would be the
        // entire JSON document → at most ~4 lines (one per change tag).
        // Pretty-printed across multiple lines, the diff should produce
        // significantly more rendered lines AND distinct +/- lines for
        // the changed `v` field (lowercase → uppercase).
        assert!(
            cache.lines.len() >= 6,
            "expected pretty-printed diff to span multiple lines, got {}",
            cache.lines.len()
        );
        let inserts: Vec<&str> = cache
            .lines
            .iter()
            .filter(|l| matches!(l.tag, similar::ChangeTag::Insert))
            .map(|l| l.text.as_str())
            .collect();
        assert!(
            inserts.iter().any(|t| t.contains("\"A\"")),
            "uppercase A must appear as an insert; inserts: {inserts:?}"
        );
        assert!(
            inserts.iter().any(|t| t.contains("\"B\"")),
            "uppercase B must appear as an insert; inserts: {inserts:?}"
        );
    }

    #[test]
    fn resolve_and_cache_diff_skips_when_no_differences() {
        let mut modal = stub_modal(1, ContentModalTab::Input);
        modal.input = text_buffer("identical content");
        modal.output = text_buffer("identical content");
        resolve_and_cache_diff(&mut modal);
        assert_eq!(
            modal.diffable,
            Diffable::NotAvailable(NotDiffableReason::NoDifferences)
        );
        assert!(modal.diff_cache.is_none());
    }

    // ── horizontal scroll reducer tests ───────────────────────────────────────

    #[test]
    fn horizontal_scroll_by_advances_and_clamps_at_zero() {
        let mut state = TracerState {
            content_modal: Some(stub_modal(1, ContentModalTab::Input)),
            ..TracerState::default()
        };
        content_modal_scroll_horizontal_by(&mut state, 5);
        assert_eq!(
            state
                .content_modal
                .as_ref()
                .unwrap()
                .horizontal_scroll_offset,
            5
        );
        content_modal_scroll_horizontal_by(&mut state, -3);
        assert_eq!(
            state
                .content_modal
                .as_ref()
                .unwrap()
                .horizontal_scroll_offset,
            2
        );
        // Saturates at 0 on the left — scrolling further left is a no-op.
        content_modal_scroll_horizontal_by(&mut state, -10);
        assert_eq!(
            state
                .content_modal
                .as_ref()
                .unwrap()
                .horizontal_scroll_offset,
            0
        );
    }

    #[test]
    fn horizontal_scroll_home_resets_offset() {
        let mut state = TracerState {
            content_modal: Some(stub_modal(1, ContentModalTab::Input)),
            ..TracerState::default()
        };
        content_modal_scroll_horizontal_by(&mut state, 42);
        content_modal_scroll_horizontal_home(&mut state);
        assert_eq!(
            state
                .content_modal
                .as_ref()
                .unwrap()
                .horizontal_scroll_offset,
            0
        );
    }

    #[test]
    fn tab_switch_resets_horizontal_scroll() {
        let mut state = TracerState {
            content_modal: Some(stub_modal(1, ContentModalTab::Input)),
            ..TracerState::default()
        };
        content_modal_scroll_horizontal_by(&mut state, 25);
        switch_content_modal_tab(&mut state, ContentModalTab::Output, None);
        assert_eq!(
            state
                .content_modal
                .as_ref()
                .unwrap()
                .horizontal_scroll_offset,
            0,
            "switching tabs must reset horizontal scroll"
        );
    }
}
