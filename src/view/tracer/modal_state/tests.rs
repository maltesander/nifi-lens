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
        scroll: BidirectionalScrollState::default(),
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
    use crate::config::TracerCeilingConfig;

    let mut modal = stub_modal(1, ContentModalTab::Input);
    modal.input.in_flight = true;
    let mut state = TracerState {
        content_modal: Some(modal),
        ..TracerState::default()
    };

    // Ceiling = 10 bytes (text & tabular both set to 10), chunk = 20 bytes → ceiling_hit
    let cfg = TracerCeilingConfig {
        text: Some(10),
        tabular: Some(10),
        diff: Some(512 * 1024),
    };
    apply_modal_chunk_with_ceiling(
        &mut state,
        1,
        ContentSide::Input,
        0,
        vec![0u8; 20],
        false,
        20,
        &cfg,
    );

    let buf = &state.content_modal.as_ref().unwrap().input;
    assert!(buf.ceiling_hit);
    assert!(buf.fully_loaded);
}

#[test]
fn apply_modal_chunk_multi_chunk_progressive_ceiling_hit() {
    use crate::client::ContentSide;
    use crate::config::TracerCeilingConfig;

    let mut modal = stub_modal(1, ContentModalTab::Input);
    modal.input.in_flight = true;
    let mut state = TracerState {
        content_modal: Some(modal),
        ..TracerState::default()
    };

    let cfg = TracerCeilingConfig {
        text: Some(10),
        tabular: Some(10),
        diff: Some(512 * 1024),
    };

    // Chunk 1: 5 bytes — well below the 10-byte ceiling.
    apply_modal_chunk_with_ceiling(
        &mut state,
        1,
        ContentSide::Input,
        0,
        vec![b'a'; 5],
        false,
        5,
        &cfg,
    );
    {
        let buf = &state.content_modal.as_ref().unwrap().input;
        assert_eq!(buf.loaded.len(), 5, "after first chunk, 5 bytes loaded");
        assert!(!buf.ceiling_hit, "ceiling not yet hit");
        assert!(!buf.fully_loaded, "not yet fully loaded");
    }

    // Chunk 2: 8 bytes appended at offset 5 — total would be 13, ceiling is 10.
    // Reducer truncates to 10 and flips ceiling_hit + fully_loaded.
    apply_modal_chunk_with_ceiling(
        &mut state,
        1,
        ContentSide::Input,
        5,
        vec![b'b'; 8],
        false,
        8,
        &cfg,
    );
    {
        let buf = &state.content_modal.as_ref().unwrap().input;
        assert_eq!(buf.loaded.len(), 10, "boundary chunk truncates to ceiling");
        assert!(
            buf.ceiling_hit,
            "ceiling_hit must flip after boundary chunk"
        );
        assert!(
            buf.fully_loaded,
            "fully_loaded must flip alongside ceiling_hit"
        );
    }
}

#[test]
fn apply_modal_chunk_after_ceiling_hit_is_dropped_via_offset_mismatch() {
    // After ceiling_hit + truncate, buf.loaded.len() == ceiling. A late
    // chunk arriving with the original-pre-truncation offset is dropped
    // by the `offset != buf.loaded.len()` guard. This pins the stale-
    // chunk behavior so a future refactor of the offset check breaks
    // this test deterministically.
    use crate::client::ContentSide;
    use crate::config::TracerCeilingConfig;

    let mut modal = stub_modal(1, ContentModalTab::Input);
    modal.input.in_flight = true;
    let mut state = TracerState {
        content_modal: Some(modal),
        ..TracerState::default()
    };

    let cfg = TracerCeilingConfig {
        text: Some(10),
        tabular: Some(10),
        diff: Some(512 * 1024),
    };

    // First chunk overflows; ceiling_hit fires, loaded truncates to 10.
    apply_modal_chunk_with_ceiling(
        &mut state,
        1,
        ContentSide::Input,
        0,
        vec![b'x'; 20],
        false,
        20,
        &cfg,
    );
    assert_eq!(state.content_modal.as_ref().unwrap().input.loaded.len(), 10);
    assert!(state.content_modal.as_ref().unwrap().input.ceiling_hit);

    // Stale chunk with offset 20 (the producer's pre-truncate position).
    apply_modal_chunk_with_ceiling(
        &mut state,
        1,
        ContentSide::Input,
        20,
        vec![b'y'; 5],
        false,
        5,
        &cfg,
    );
    let buf = &state.content_modal.as_ref().unwrap().input;
    assert_eq!(
        buf.loaded.len(),
        10,
        "stale chunk with offset != loaded.len() must be dropped"
    );
    // No bytes from chunk 2 leaked into the buffer.
    assert!(
        buf.loaded.iter().all(|&b| b == b'x'),
        "buf contains only the original 'x' bytes"
    );
}

#[test]
fn apply_modal_chunk_unbounded_ceiling_keeps_streaming() {
    use crate::client::ContentSide;
    use crate::config::TracerCeilingConfig;

    let mut modal = stub_modal(1, ContentModalTab::Input);
    modal.input.in_flight = true;
    let mut state = TracerState {
        content_modal: Some(modal),
        ..TracerState::default()
    };

    // Unbounded config (text = None, tabular = None)
    let cfg = TracerCeilingConfig {
        text: None,
        tabular: None,
        diff: Some(512 * 1024),
    };
    apply_modal_chunk_with_ceiling(
        &mut state,
        1,
        ContentSide::Input,
        0,
        vec![0u8; 20],
        false,
        20,
        &cfg,
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
    use crate::client::tracer::{AttributeTriple, ProvenanceEventDetail, ProvenanceEventSummary};
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

    let cfg = crate::config::TracerCeilingConfig::default();
    let fired = open_content_modal(&mut state, &detail, ContentModalTab::Input, &cfg);
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
    use crate::config::TracerCeilingConfig;

    let detail = stub_event_detail();
    let mut state = TracerState::default();
    let cfg = TracerCeilingConfig {
        text: Some(100_000),
        tabular: Some(100_000),
        diff: Some(512 * 1024),
    };
    let fired = open_content_modal(&mut state, &detail, ContentModalTab::Input, &cfg);
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
    modal.scroll.vertical.last_viewport_rows = 30;
    let state = TracerState {
        content_modal: Some(modal),
        ..TracerState::default()
    };
    let mut state = state;

    // Viewport bottom = 965 + 30 = 995 → distance to tail (1000) = 5 < 100.
    let cfg = crate::config::TracerCeilingConfig::default();
    let fired = content_modal_scroll_to(&mut state, 965, &cfg);
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
    modal.scroll.vertical.last_viewport_rows = 30;
    let mut state = TracerState {
        content_modal: Some(modal),
        ..TracerState::default()
    };
    let cfg = crate::config::TracerCeilingConfig::default();
    let fired = content_modal_scroll_to(&mut state, 965, &cfg);
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
    modal.scroll.vertical.last_viewport_rows = 30;
    let mut state = TracerState {
        content_modal: Some(modal),
        ..TracerState::default()
    };
    let cfg = crate::config::TracerCeilingConfig::default();
    let fired = content_modal_scroll_to(&mut state, 965, &cfg);
    assert!(fired.is_empty());
}

#[test]
fn content_modal_scroll_respects_ceiling() {
    use crate::client::tracer::ContentRender;
    use crate::config::TracerCeilingConfig;

    let text = "a\n".repeat(1000);
    let loaded_bytes = text.as_bytes().to_vec();
    let loaded_len = loaded_bytes.len();
    let mut modal = stub_modal(1, ContentModalTab::Input);
    modal.input.loaded = loaded_bytes;
    modal.input.decoded = ContentRender::Text {
        text,
        pretty_printed: false,
    };
    // Pre-resolve the effective ceiling so the scroll reducer sees it.
    let cap = loaded_len + 48_576;
    modal.input.effective_ceiling = Some(Some(cap));
    modal.scroll.vertical.last_viewport_rows = 30;
    let mut state = TracerState {
        content_modal: Some(modal),
        ..TracerState::default()
    };
    // ceiling resolved to loaded_len + 48576; remaining = 48576, chunk = min(512K, 48576)
    let cfg = TracerCeilingConfig::default();
    let fired = content_modal_scroll_to(&mut state, 965, &cfg);
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
        effective_ceiling: None,
        in_flight_decode: false,
    }
}

// ── resolve_diffable tests ────────────────────────────────────────────────

fn default_ceiling() -> crate::config::TracerCeilingConfig {
    crate::config::TracerCeilingConfig::default()
}

#[test]
fn diffable_ok_when_mime_equal_and_allowlisted() {
    let header = header_with_mime("application/json", "application/json", 1024, 1024);
    let (input, output) = (text_buffer("a"), text_buffer("b"));
    assert_eq!(
        resolve_diffable(&header, &input, &output, &default_ceiling()),
        Diffable::Ok
    );
}

#[test]
fn diffable_wildcard_text_star() {
    let header = header_with_mime("text/html", "text/html", 1024, 1024);
    let (input, output) = (text_buffer("a"), text_buffer("b"));
    assert_eq!(
        resolve_diffable(&header, &input, &output, &default_ceiling()),
        Diffable::Ok
    );
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
    assert_eq!(
        resolve_diffable(&header, &input, &output, &default_ceiling()),
        Diffable::Ok
    );
}

#[test]
fn diffable_mime_mismatch() {
    let header = header_with_mime("application/json", "text/csv", 1024, 1024);
    let (input, output) = (text_buffer("a"), text_buffer("b"));
    assert_eq!(
        resolve_diffable(&header, &input, &output, &default_ceiling()),
        Diffable::NotAvailable(NotDiffableReason::MimeMismatch)
    );
}

#[test]
fn diffable_utf8_fallback_when_no_mime() {
    let mut header = header_with_mime("", "", 1024, 1024);
    header.input_mime = None;
    header.output_mime = None;
    let (input, output) = (text_buffer("a"), text_buffer("b"));
    assert_eq!(
        resolve_diffable(&header, &input, &output, &default_ceiling()),
        Diffable::Ok
    );
}

#[test]
fn diffable_size_exceeds_cap() {
    // 600_000 bytes exceeds the default 16 MiB cap only if the cap is set
    // small; use an explicit 512 KiB cap to match the original hardcoded
    // constant.
    let header = header_with_mime("application/json", "application/json", 600_000, 1024);
    let (input, output) = (text_buffer("a"), text_buffer("b"));
    let cfg = crate::config::TracerCeilingConfig {
        diff: Some(512 * 1024),
        ..crate::config::TracerCeilingConfig::default()
    };
    assert_eq!(
        resolve_diffable(&header, &input, &output, &cfg),
        Diffable::NotAvailable(NotDiffableReason::SizeExceedsDiffCap)
    );
}

#[test]
fn diffable_no_differences_when_bytes_equal() {
    let header = header_with_mime("application/json", "application/json", 10, 10);
    let (input, output) = (text_buffer("same"), text_buffer("same"));
    assert_eq!(
        resolve_diffable(&header, &input, &output, &default_ceiling()),
        Diffable::NotAvailable(NotDiffableReason::NoDifferences)
    );
}

#[test]
fn diffable_input_unavailable() {
    let mut header = header_with_mime("application/json", "application/json", 10, 10);
    header.input_available = false;
    let (input, output) = (SideBuffer::default(), text_buffer("x"));
    assert_eq!(
        resolve_diffable(&header, &input, &output, &default_ceiling()),
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
        effective_ceiling: None,
        in_flight_decode: false,
    };
    assert_eq!(
        resolve_diffable(&header, &empty, &empty, &default_ceiling()),
        Diffable::NotAvailable(NotDiffableReason::NoDifferences)
    );
}

#[test]
fn compute_diff_cache_produces_lines_and_hunks() {
    let input = "line a\nline b\nline c\n";
    let output = "line a\nline B\nline c\n";
    let render = compute_diff_cache(input, output, &default_ceiling());
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
    let cfg = crate::config::TracerCeilingConfig::default();
    let fired = switch_content_modal_tab(&mut state, ContentModalTab::Output, &cfg);
    let modal = state.content_modal.as_ref().unwrap();
    assert_eq!(modal.active_tab, ContentModalTab::Output);
    assert_eq!(modal.scroll.vertical.offset, 0);
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
    let cfg = crate::config::TracerCeilingConfig::default();
    let fired = switch_content_modal_tab(&mut state, ContentModalTab::Output, &cfg);
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
    modal.scroll.vertical.offset = 5;
    let mut state = TracerState {
        content_modal: Some(modal),
        ..TracerState::default()
    };
    hunk_next(&mut state);
    assert_eq!(
        state.content_modal.as_ref().unwrap().scroll.vertical.offset,
        10
    );
    hunk_next(&mut state);
    assert_eq!(
        state.content_modal.as_ref().unwrap().scroll.vertical.offset,
        50
    );
    hunk_next(&mut state);
    assert_eq!(
        state.content_modal.as_ref().unwrap().scroll.vertical.offset,
        50
    );
}

#[test]
fn hunk_prev_moves_backward() {
    let mut modal = stub_modal(1, ContentModalTab::Diff);
    modal.diff_cache = Some(DiffRender {
        lines: Vec::new(),
        hunks: Vec::new(),
        change_stops: vec![10, 50],
    });
    modal.scroll.vertical.offset = 75;
    let mut state = TracerState {
        content_modal: Some(modal),
        ..TracerState::default()
    };
    hunk_prev(&mut state);
    assert_eq!(
        state.content_modal.as_ref().unwrap().scroll.vertical.offset,
        50
    );
    hunk_prev(&mut state);
    assert_eq!(
        state.content_modal.as_ref().unwrap().scroll.vertical.offset,
        10
    );
    hunk_prev(&mut state);
    assert_eq!(
        state.content_modal.as_ref().unwrap().scroll.vertical.offset,
        10
    );
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
    let render = compute_diff_cache(&input, &output, &default_ceiling());
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
    modal.scroll.vertical.last_viewport_rows = 30;
    let mut state = TracerState {
        content_modal: Some(modal),
        ..TracerState::default()
    };

    // Scroll to line 165: viewport bottom = 195, distance to tail (200) = 5 < 100.
    let cfg = crate::config::TracerCeilingConfig::default();
    let fired = content_modal_scroll_by(&mut state, 165, &cfg);
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
    modal.scroll.vertical.offset = 0;
    modal.scroll.vertical.last_viewport_rows = 20;
    let mut state = TracerState {
        content_modal: Some(modal),
        ..TracerState::default()
    };
    let cfg = crate::config::TracerCeilingConfig::default();
    let fired = content_modal_scroll_by(&mut state, -10, &cfg);
    assert!(fired.is_empty(), "no fetch when fully loaded");
    assert_eq!(
        state.content_modal.as_ref().unwrap().scroll.vertical.offset,
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
    modal.scroll.vertical.last_viewport_rows = 10;
    let mut state = TracerState {
        content_modal: Some(modal),
        ..TracerState::default()
    };
    let cfg = crate::config::TracerCeilingConfig::default();
    let line_count = content_modal_line_count(&state);
    let fired = content_modal_scroll_to(&mut state, line_count.saturating_sub(1), &cfg);
    assert!(fired.is_empty(), "fully loaded: no fetch on Last");
    assert_eq!(
        state.content_modal.as_ref().unwrap().scroll.vertical.offset,
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
    modal.scroll.vertical.offset = 10;
    modal.scroll.vertical.last_viewport_rows = 20;
    let mut state = TracerState {
        content_modal: Some(modal),
        ..TracerState::default()
    };
    let cfg = crate::config::TracerCeilingConfig::default();
    let fired = content_modal_scroll_by(&mut state, 20, &cfg);
    assert!(fired.is_empty(), "fully loaded: no fetch");
    assert_eq!(
        state.content_modal.as_ref().unwrap().scroll.vertical.offset,
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
    modal.scroll.vertical.offset = 0;
    modal.scroll.vertical.last_viewport_rows = 10;
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

    let cfg = crate::config::TracerCeilingConfig::default();
    let fired = content_modal_scroll_to_match(&mut state, &cfg);
    // fully_loaded — no fetch fires; scroll offset updated.
    assert!(fired.is_empty(), "no fetch for fully loaded content");
    assert_eq!(
        state.content_modal.as_ref().unwrap().scroll.vertical.offset,
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
    resolve_and_cache_diff(&mut modal, &default_ceiling());
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
    resolve_and_cache_diff(&mut modal, &default_ceiling());
    let first_ptr = modal.diff_cache.as_ref().unwrap() as *const DiffRender;
    // Second call must not reallocate — same Box address.
    resolve_and_cache_diff(&mut modal, &default_ceiling());
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
    resolve_and_cache_diff(&mut modal, &default_ceiling());
    assert_eq!(modal.diffable, Diffable::Pending);
    assert!(modal.diff_cache.is_none());
}

#[test]
fn compute_diff_cache_interleaves_replace_pairs() {
    // Three CSV-shaped lines, all changed in place — the classic
    // "every line is a delete" case that the interleave fix targets.
    let input = "a,1,OK\nb,2,WARN\nc,3,OK\n";
    let output = "a,1,ok\nb,2,warn\nc,3,ok\n";
    let render = compute_diff_cache(input, output, &default_ceiling());

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
    let render = compute_diff_cache(input, output, &default_ceiling());

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
    let render = compute_diff_cache(input, output, &default_ceiling());

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
        effective_ceiling: None,
        in_flight_decode: false,
    };
    modal.output = SideBuffer {
        loaded: output_compact,
        decoded: output_decoded,
        fully_loaded: true,
        ceiling_hit: false,
        in_flight: false,
        last_error: None,
        effective_ceiling: None,
        in_flight_decode: false,
    };

    resolve_and_cache_diff(&mut modal, &default_ceiling());
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
    resolve_and_cache_diff(&mut modal, &default_ceiling());
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
            .scroll
            .horizontal_offset,
        5
    );
    content_modal_scroll_horizontal_by(&mut state, -3);
    assert_eq!(
        state
            .content_modal
            .as_ref()
            .unwrap()
            .scroll
            .horizontal_offset,
        2
    );
    // Saturates at 0 on the left — scrolling further left is a no-op.
    content_modal_scroll_horizontal_by(&mut state, -10);
    assert_eq!(
        state
            .content_modal
            .as_ref()
            .unwrap()
            .scroll
            .horizontal_offset,
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
            .scroll
            .horizontal_offset,
        0
    );
}

#[test]
fn tab_switch_resets_horizontal_scroll() {
    let mut state = TracerState {
        content_modal: Some(stub_modal(1, ContentModalTab::Input)),
        ..TracerState::default()
    };
    let cfg = crate::config::TracerCeilingConfig::default();
    content_modal_scroll_horizontal_by(&mut state, 25);
    switch_content_modal_tab(&mut state, ContentModalTab::Output, &cfg);
    assert_eq!(
        state
            .content_modal
            .as_ref()
            .unwrap()
            .scroll
            .horizontal_offset,
        0,
        "switching tabs must reset horizontal scroll"
    );
}

#[test]
fn first_chunk_with_parquet_magic_resolves_to_tabular_ceiling() {
    use crate::config::TracerCeilingConfig;
    let cfg = TracerCeilingConfig {
        text: Some(4 * 1024 * 1024),
        tabular: Some(64 * 1024 * 1024),
        diff: Some(16 * 1024 * 1024),
    };
    let mut buf = SideBuffer::default();
    let mut chunk = b"PAR1".to_vec();
    chunk.extend_from_slice(&[0u8; 100]);
    apply_first_chunk_and_resolve_ceiling(&mut buf, &chunk, &cfg);
    assert_eq!(buf.effective_ceiling, Some(Some(64 * 1024 * 1024)));
}

#[test]
fn first_chunk_without_tabular_magic_resolves_to_text_ceiling() {
    use crate::config::TracerCeilingConfig;
    let cfg = TracerCeilingConfig {
        text: Some(4 * 1024 * 1024),
        tabular: Some(64 * 1024 * 1024),
        diff: Some(16 * 1024 * 1024),
    };
    let mut buf = SideBuffer::default();
    apply_first_chunk_and_resolve_ceiling(&mut buf, b"hello world", &cfg);
    assert_eq!(buf.effective_ceiling, Some(Some(4 * 1024 * 1024)));
}

#[test]
fn first_chunk_with_avro_magic_resolves_to_tabular_ceiling() {
    use crate::config::TracerCeilingConfig;
    let cfg = TracerCeilingConfig::default();
    let mut buf = SideBuffer::default();
    let mut chunk = b"Obj\x01".to_vec();
    chunk.extend_from_slice(&[0u8; 100]);
    apply_first_chunk_and_resolve_ceiling(&mut buf, &chunk, &cfg);
    assert_eq!(buf.effective_ceiling, Some(cfg.tabular));
}

#[test]
fn apply_is_idempotent_after_first_resolution() {
    use crate::config::TracerCeilingConfig;
    let cfg = TracerCeilingConfig::default();
    let mut buf = SideBuffer::default();
    apply_first_chunk_and_resolve_ceiling(&mut buf, b"hello world", &cfg);
    let resolved_first = buf.effective_ceiling;
    // Second call (e.g. with parquet magic) must NOT overwrite the first
    // resolution — once decided, the ceiling is sticky.
    apply_first_chunk_and_resolve_ceiling(&mut buf, b"PAR1\0\0\0\0", &cfg);
    assert_eq!(buf.effective_ceiling, resolved_first);
}

#[test]
fn provisional_ceiling_is_max_of_text_and_tabular() {
    use crate::config::TracerCeilingConfig;
    let cfg = TracerCeilingConfig {
        text: Some(4 * 1024 * 1024),
        tabular: Some(64 * 1024 * 1024),
        diff: Some(16 * 1024 * 1024),
    };
    assert_eq!(provisional_ceiling(&cfg), Some(64 * 1024 * 1024));

    let cfg2 = TracerCeilingConfig {
        text: Some(64 * 1024 * 1024),
        tabular: Some(4 * 1024 * 1024),
        diff: Some(16 * 1024 * 1024),
    };
    assert_eq!(provisional_ceiling(&cfg2), Some(64 * 1024 * 1024));
}

#[test]
fn provisional_ceiling_is_unbounded_when_either_is() {
    use crate::config::TracerCeilingConfig;
    let cfg = TracerCeilingConfig {
        text: None,
        tabular: Some(64 * 1024 * 1024),
        diff: Some(16 * 1024 * 1024),
    };
    assert_eq!(provisional_ceiling(&cfg), None);

    let cfg2 = TracerCeilingConfig {
        text: Some(4 * 1024 * 1024),
        tabular: None,
        diff: Some(16 * 1024 * 1024),
    };
    assert_eq!(provisional_ceiling(&cfg2), None);
}

#[test]
fn should_fire_next_chunk_uses_resolved_ceiling_when_decided() {
    use crate::config::TracerCeilingConfig;
    let cfg = TracerCeilingConfig::default();
    let mut side = SideBuffer {
        loaded: vec![0u8; 100],
        ..SideBuffer::default()
    };
    // Not yet resolved → falls back to provisional (max(text, tabular)) = 64 MiB.
    assert!(should_fire_next_chunk(&side, &cfg));

    // Mark as resolved with a tiny cap.
    side.effective_ceiling = Some(Some(50));
    assert!(!should_fire_next_chunk(&side, &cfg)); // 100 >= 50, no room

    // Resolved as unbounded.
    side.effective_ceiling = Some(None);
    assert!(should_fire_next_chunk(&side, &cfg));
}

#[test]
fn diffable_size_exceeds_diff_cap_uses_config_value() {
    use crate::client::tracer::ContentRender;
    use crate::config::TracerCeilingConfig;
    let header = ContentModalHeader {
        event_type: "FORK".into(),
        event_timestamp_iso: "".into(),
        component_name: "".into(),
        pg_path: "".into(),
        input_size: Some(2_000_000),
        output_size: Some(2_000_000),
        input_mime: Some("application/json".into()),
        output_mime: Some("application/json".into()),
        input_available: true,
        output_available: true,
    };
    let mut input = SideBuffer::default();
    let mut output = SideBuffer::default();
    input.loaded = vec![b'a'; 2_000_000];
    output.loaded = vec![b'b'; 2_000_000];
    input.fully_loaded = true;
    output.fully_loaded = true;
    input.decoded = ContentRender::Text {
        text: "a".repeat(2_000_000),
        pretty_printed: false,
    };
    output.decoded = ContentRender::Text {
        text: "b".repeat(2_000_000),
        pretty_printed: false,
    };

    let cfg = TracerCeilingConfig {
        text: Some(4 * 1024 * 1024),
        tabular: Some(64 * 1024 * 1024),
        diff: Some(1_000_000), // 1 MB — both sides exceed this
    };
    let result = resolve_diffable(&header, &input, &output, &cfg);
    assert!(matches!(
        result,
        Diffable::NotAvailable(NotDiffableReason::SizeExceedsDiffCap)
    ));
}

#[test]
fn diffable_ok_for_two_parquet_sides() {
    use crate::client::tracer::{ContentRender, TabularFormat};
    use crate::config::TracerCeilingConfig;
    let header = ContentModalHeader {
        event_type: "FORK".into(),
        event_timestamp_iso: "".into(),
        component_name: "".into(),
        pg_path: "".into(),
        input_size: Some(100),
        output_size: Some(100),
        input_mime: None,
        output_mime: None,
        input_available: true,
        output_available: true,
    };
    let cfg = TracerCeilingConfig::default();
    let mut input = SideBuffer {
        fully_loaded: true,
        ..SideBuffer::default()
    };
    let mut output = SideBuffer {
        fully_loaded: true,
        ..SideBuffer::default()
    };
    input.loaded = vec![0u8; 100];
    output.loaded = vec![0u8; 100];
    input.decoded = ContentRender::Tabular {
        format: TabularFormat::Parquet,
        schema_summary: "id : Int64".into(),
        body: r#"{"id":0}"#.into(),
        decoded_bytes: 8,
        truncated: false,
    };
    output.decoded = ContentRender::Tabular {
        format: TabularFormat::Parquet,
        schema_summary: "id : Int64".into(),
        body: r#"{"id":1}"#.into(),
        decoded_bytes: 8,
        truncated: false,
    };
    assert!(matches!(
        resolve_diffable(&header, &input, &output, &cfg),
        Diffable::Ok
    ));
}

#[test]
fn diffable_mime_mismatch_for_parquet_vs_avro() {
    use crate::client::tracer::{ContentRender, TabularFormat};
    use crate::config::TracerCeilingConfig;
    let header = ContentModalHeader {
        event_type: "FORK".into(),
        event_timestamp_iso: "".into(),
        component_name: "".into(),
        pg_path: "".into(),
        input_size: Some(100),
        output_size: Some(100),
        input_mime: None,
        output_mime: None,
        input_available: true,
        output_available: true,
    };
    let cfg = TracerCeilingConfig::default();
    let make = |format| {
        let mut s = SideBuffer {
            fully_loaded: true,
            ..SideBuffer::default()
        };
        s.loaded = vec![0u8; 100];
        s.decoded = ContentRender::Tabular {
            format,
            schema_summary: String::new(),
            body: r#"{"id":0}"#.into(),
            decoded_bytes: 8,
            truncated: false,
        };
        s
    };
    let result = resolve_diffable(
        &header,
        &make(TabularFormat::Parquet),
        &make(TabularFormat::Avro),
        &cfg,
    );
    assert!(matches!(
        result,
        Diffable::NotAvailable(NotDiffableReason::MimeMismatch)
    ));
}

#[test]
fn diffable_mime_mismatch_for_tabular_vs_text() {
    use crate::client::tracer::{ContentRender, TabularFormat};
    use crate::config::TracerCeilingConfig;
    let header = ContentModalHeader {
        event_type: "FORK".into(),
        event_timestamp_iso: "".into(),
        component_name: "".into(),
        pg_path: "".into(),
        input_size: Some(100),
        output_size: Some(100),
        input_mime: None,
        output_mime: None,
        input_available: true,
        output_available: true,
    };
    let cfg = TracerCeilingConfig::default();
    let mut input = SideBuffer {
        fully_loaded: true,
        ..SideBuffer::default()
    };
    let mut output = SideBuffer {
        fully_loaded: true,
        ..SideBuffer::default()
    };
    input.loaded = vec![0u8; 100];
    output.loaded = vec![0u8; 100];
    input.decoded = ContentRender::Tabular {
        format: TabularFormat::Parquet,
        schema_summary: String::new(),
        body: r#"{"id":0}"#.into(),
        decoded_bytes: 8,
        truncated: false,
    };
    output.decoded = ContentRender::Text {
        text: "hello".into(),
        pretty_printed: false,
    };
    let result = resolve_diffable(&header, &input, &output, &cfg);
    assert!(matches!(
        result,
        Diffable::NotAvailable(NotDiffableReason::MimeMismatch)
    ));
}

#[test]
fn diffable_size_exceeds_diff_cap_for_tabular() {
    use crate::client::tracer::{ContentRender, TabularFormat};
    use crate::config::TracerCeilingConfig;
    let header = ContentModalHeader {
        event_type: "FORK".into(),
        event_timestamp_iso: "".into(),
        component_name: "".into(),
        pg_path: "".into(),
        input_size: Some(2_000_000),
        output_size: Some(2_000_000),
        input_mime: None,
        output_mime: None,
        input_available: true,
        output_available: true,
    };
    let cfg = TracerCeilingConfig {
        text: Some(4 * 1024 * 1024),
        tabular: Some(64 * 1024 * 1024),
        diff: Some(1_000_000),
    };
    let mut input = SideBuffer {
        fully_loaded: true,
        ..SideBuffer::default()
    };
    let mut output = SideBuffer {
        fully_loaded: true,
        ..SideBuffer::default()
    };
    input.loaded = vec![0u8; 100];
    output.loaded = vec![0u8; 100];
    input.decoded = ContentRender::Tabular {
        format: TabularFormat::Parquet,
        schema_summary: String::new(),
        body: "x".repeat(2_000_000),
        decoded_bytes: 2_000_000,
        truncated: false,
    };
    output.decoded = ContentRender::Tabular {
        format: TabularFormat::Parquet,
        schema_summary: String::new(),
        body: "y".repeat(2_000_000),
        decoded_bytes: 2_000_000,
        truncated: false,
    };
    let result = resolve_diffable(&header, &input, &output, &cfg);
    assert!(matches!(
        result,
        Diffable::NotAvailable(NotDiffableReason::SizeExceedsDiffCap)
    ));
}

#[test]
fn modal_diff_tabular_parquet_yields_changed_lines() {
    use crate::client::tracer::{ContentRender, TabularFormat};
    use crate::config::TracerCeilingConfig;
    let mut input = SideBuffer {
        fully_loaded: true,
        ..SideBuffer::default()
    };
    let mut output = SideBuffer {
        fully_loaded: true,
        ..SideBuffer::default()
    };
    input.decoded = ContentRender::Tabular {
        format: TabularFormat::Parquet,
        schema_summary: "id : Int64".into(),
        body: "{\"id\":0}\n{\"id\":1}\n{\"id\":2}".into(),
        decoded_bytes: 24,
        truncated: false,
    };
    output.decoded = ContentRender::Tabular {
        format: TabularFormat::Parquet,
        schema_summary: "id : Int64".into(),
        body: "{\"id\":0}\n{\"id\":2}\n{\"id\":3}".into(),
        decoded_bytes: 24,
        truncated: false,
    };
    let cfg = TracerCeilingConfig::default();
    let input_text = side_diff_text(&input);
    let output_text = side_diff_text(&output);
    let render = compute_diff_cache(&input_text, &output_text, &cfg);
    // Three lines diffed; expect at least one Insert and one Delete.
    let tags: Vec<_> = render.lines.iter().map(|l| l.tag).collect();
    assert!(
        tags.contains(&similar::ChangeTag::Insert),
        "expected at least one Insert tag"
    );
    assert!(
        tags.contains(&similar::ChangeTag::Delete),
        "expected at least one Delete tag"
    );
}

#[test]
fn tabular_scroll_decoded_line_count_includes_schema_and_separator() {
    use crate::client::tracer::{ContentRender, TabularFormat};
    let render = ContentRender::Tabular {
        format: TabularFormat::Parquet,
        schema_summary: "id : Int64\nname : Utf8".into(), // 2 lines
        body: "{\"id\":0}\n{\"id\":1}\n{\"id\":2}".into(), // 3 lines
        decoded_bytes: 24,
        truncated: false,
    };
    // 2 schema + 1 separator + 3 body = 6
    assert_eq!(decoded_line_count(&render), 6);
}

#[test]
fn tabular_search_body_includes_schema_separator_and_body() {
    use crate::client::tracer::{ContentRender, TabularFormat};
    let mut modal = stub_modal(1, ContentModalTab::Input);
    modal.input.decoded = ContentRender::Tabular {
        format: TabularFormat::Parquet,
        schema_summary: "active : Boolean".into(),
        body: "{\"active\":true}".into(),
        decoded_bytes: 16,
        truncated: false,
    };
    let body = content_modal_search_body(&modal);
    assert!(
        body.contains("active : Boolean"),
        "schema column name must be searchable"
    );
    assert!(body.contains("-- schema --"), "separator must be present");
    assert!(
        body.contains("\"active\":true"),
        "body content must be searchable"
    );
}

#[test]
fn tabular_scroll_does_not_panic() {
    use crate::client::tracer::{ContentRender, TabularFormat};

    // Construct a Tabular modal with enough lines to scroll.
    let schema = "id : Int64\nname : Utf8".to_string(); // 2 lines
    let body_rows: Vec<String> = (0..20).map(|i| format!("{{\"id\":{}}}", i)).collect();
    let body = body_rows.join("\n"); // 20 lines
    // total = 2 + 1 + 20 = 23

    let mut modal = stub_modal(1, ContentModalTab::Input);
    modal.input.fully_loaded = true;
    modal.input.decoded = ContentRender::Tabular {
        format: TabularFormat::Parquet,
        schema_summary: schema,
        body,
        decoded_bytes: 200,
        truncated: false,
    };
    modal.scroll.vertical.last_viewport_rows = 10;
    let mut state = TracerState {
        content_modal: Some(modal),
        ..TracerState::default()
    };

    // Scroll forward — must not panic and must advance offset.
    let cfg = crate::config::TracerCeilingConfig::default();
    let fired = content_modal_scroll_by(&mut state, 5, &cfg);
    assert!(fired.is_empty(), "fully_loaded: no fetch request expected");
    let offset = state.content_modal.as_ref().unwrap().scroll.vertical.offset;
    assert_eq!(offset, 5, "scroll_offset should advance to 5");

    // Scroll backward to zero.
    content_modal_scroll_by(&mut state, -10, &cfg);
    let offset = state.content_modal.as_ref().unwrap().scroll.vertical.offset;
    assert_eq!(offset, 0, "scroll_offset must clamp at 0");
}

// ── Finding 1: deferred tabular decode tests ─────────────────────────────

/// `take_pending_tabular_decode` returns `Some` and sets `in_flight_decode`
/// when the buffer has tabular magic bytes AND is fully loaded.
/// Verifies `decoded` stays `Empty` until the off-thread result lands.
#[test]
fn tabular_chunk_fully_loaded_yields_pending_decode() {
    use crate::client::ContentSide;
    use crate::client::tracer::ContentRender;
    use crate::config::TracerCeilingConfig;

    let mut modal = stub_modal(7, ContentModalTab::Input);
    modal.input.in_flight = true;
    let mut state = TracerState {
        content_modal: Some(modal),
        ..TracerState::default()
    };

    // Build a minimal chunk with PAR1 magic. Ceiling matches exactly so
    // ceiling_hit fires without a second chunk.
    let mut parquet_magic = b"PAR1".to_vec();
    parquet_magic.extend_from_slice(&[0u8; 6]); // 10 bytes total
    let cfg = TracerCeilingConfig {
        text: Some(10),
        tabular: Some(10),
        diff: Some(512 * 1024),
    };
    apply_modal_chunk_with_ceiling(
        &mut state,
        7,
        ContentSide::Input,
        0,
        parquet_magic.clone(),
        false,
        parquet_magic.len(),
        &cfg,
    );

    // decoded must remain Empty while the off-thread decode is pending.
    let buf = &state.content_modal.as_ref().unwrap().input;
    assert!(
        matches!(buf.decoded, ContentRender::Empty),
        "decoded must remain Empty until off-thread decode lands"
    );
    assert!(buf.ceiling_hit, "ceiling must be hit");
    assert!(buf.fully_loaded);
    assert!(!buf.in_flight_decode, "guard not set yet before take");

    // take_pending_tabular_decode should fire and set in_flight_decode.
    let pending = take_pending_tabular_decode(&mut state, 7, ContentSide::Input);
    assert!(pending.is_some(), "expected pending decode to be returned");
    let (eid, s, _bytes) = pending.unwrap();
    assert_eq!(eid, 7);
    assert!(matches!(s, ContentSide::Input));

    let buf = &state.content_modal.as_ref().unwrap().input;
    assert!(
        buf.in_flight_decode,
        "in_flight_decode must be set after take"
    );
}

/// Calling `take_pending_tabular_decode` a second time while
/// `in_flight_decode` is true must return `None` (prevents double-spawn).
#[test]
fn tabular_in_flight_guard_prevents_double_spawn() {
    use crate::client::ContentSide;
    use crate::config::TracerCeilingConfig;

    let mut modal = stub_modal(8, ContentModalTab::Input);
    modal.input.in_flight = true;
    let mut state = TracerState {
        content_modal: Some(modal),
        ..TracerState::default()
    };

    let mut parquet_magic = b"PAR1".to_vec();
    parquet_magic.extend_from_slice(&[0u8; 6]);
    let cfg = TracerCeilingConfig {
        text: Some(10),
        tabular: Some(10),
        diff: Some(512 * 1024),
    };
    apply_modal_chunk_with_ceiling(
        &mut state,
        8,
        ContentSide::Input,
        0,
        parquet_magic,
        false,
        10,
        &cfg,
    );

    // First take: sets in_flight_decode.
    let first = take_pending_tabular_decode(&mut state, 8, ContentSide::Input);
    assert!(first.is_some());

    // Second take: guard fires, returns None.
    let second = take_pending_tabular_decode(&mut state, 8, ContentSide::Input);
    assert!(second.is_none(), "in-flight guard must block double-spawn");
}

/// `apply_tabular_decode_result` drops results whose `event_id` doesn't
/// match the currently-open modal.
#[test]
fn stale_decode_result_is_dropped() {
    use crate::client::ContentSide;
    use crate::client::tracer::ContentRender;

    let modal = stub_modal(9, ContentModalTab::Input);
    let mut state = TracerState {
        content_modal: Some(modal),
        ..TracerState::default()
    };
    // Manually mark in_flight_decode to simulate an in-progress decode.
    state.content_modal.as_mut().unwrap().input.in_flight_decode = true;

    // Deliver result for event_id=999 (does not match modal's 9).
    apply_tabular_decode_result(
        &mut state,
        999,
        ContentSide::Input,
        ContentRender::Text {
            text: "stale".into(),
            pretty_printed: false,
        },
    );

    let buf = &state.content_modal.as_ref().unwrap().input;
    // guard still set, decoded unchanged
    assert!(buf.in_flight_decode, "guard must survive a stale drop");
    assert!(matches!(buf.decoded, ContentRender::Empty));
}

// ── Finding 2: spec-mandated ceiling tests ───────────────────────────────

#[test]
fn ceiling_selection_prefers_tabular_on_parquet_magic() {
    use crate::config::TracerCeilingConfig;
    let cfg = TracerCeilingConfig {
        text: Some(4 * 1024 * 1024),
        tabular: Some(64 * 1024 * 1024),
        diff: Some(16 * 1024 * 1024),
    };
    let mut buf = SideBuffer::default();
    let mut chunk = b"PAR1".to_vec();
    chunk.extend_from_slice(&[0u8; 100]);
    apply_first_chunk_and_resolve_ceiling(&mut buf, &chunk, &cfg);
    assert_eq!(buf.effective_ceiling, Some(Some(64 * 1024 * 1024)));
}

#[test]
fn ceiling_selection_prefers_text_when_no_tabular_magic() {
    use crate::config::TracerCeilingConfig;
    let cfg = TracerCeilingConfig {
        text: Some(4 * 1024 * 1024),
        tabular: Some(64 * 1024 * 1024),
        diff: Some(16 * 1024 * 1024),
    };
    let mut buf = SideBuffer::default();
    apply_first_chunk_and_resolve_ceiling(&mut buf, b"hello world", &cfg);
    assert_eq!(buf.effective_ceiling, Some(Some(4 * 1024 * 1024)));
}
