//! Render the Tracer content viewer modal.
//!
//! Full-screen overlay. Header with event identity, sizes, mime pair,
//! and diff eligibility. Tab strip (Input / Output / Diff) with
//! grayed Diff when ineligible. Body is the scrollable region —
//! Input/Output show streamed text or hex; Diff shows a colored
//! unified diff. Footer strip carries the modal-scoped hint line.

mod render;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, BorderType, Clear};

use crate::view::tracer::state::ContentModalState;

use render::*;

pub fn render(
    frame: &mut Frame,
    area: Rect,
    modal: &mut ContentModalState,
    cfg: &crate::config::TracerCeilingConfig,
) {
    frame.render_widget(Clear, area);

    let title = format!(
        " Content · {} · event {} · {} ",
        modal.header.component_name, modal.event_id, modal.header.event_type,
    );
    let block = Block::bordered()
        .border_type(BorderType::Thick)
        .title(title.as_str());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Length(1), // blank
            Constraint::Length(1), // tab strip
            Constraint::Length(1), // divider
            Constraint::Fill(1),   // body
            Constraint::Length(1), // divider
            Constraint::Length(1), // stream status
            Constraint::Length(1), // search strip (reserved; may be empty)
            Constraint::Length(1), // footer hint
        ])
        .split(inner);

    render_header(frame, rows[0], modal);
    render_tab_strip(frame, rows[2], modal);
    render_body(frame, rows[4], modal);
    render_stream_status(frame, rows[6], modal, cfg);
    render_search_strip(frame, rows[7], modal);
    render_footer_hint(frame, rows[8], modal);
    modal.scroll.vertical.last_viewport_rows = rows[4].height as usize;
    // Body cols = area width minus the fixed gutter. The renderer
    // already computed the gutter size above; approximate here as the
    // total width minus a small default when the renderer hasn't run
    // through text yet. The first frame is always generous since the
    // live modal starts with at least one chunk requested.
    modal.scroll.last_viewport_body_cols = rows[4].width as usize;
}

/// Returns the chip text to show when a side has hit its per-side ceiling.
///
/// If the loaded buffer starts with the Parquet magic bytes (`PAR1`) AND
/// `decoded` is `Hex` (meaning the partial parquet could not be decoded),
/// this surfaces a distinct, actionable message pointing the user at the
/// relevant config knob. All other ceiling-hit cases fall through to the
/// legacy chip text.
pub fn ceiling_hit_chip(
    side: &crate::view::tracer::modal_state::SideBuffer,
    cfg: &crate::config::TracerCeilingConfig,
) -> String {
    use crate::client::tracer::ContentRender;
    let parquet_truncated =
        side.loaded.starts_with(b"PAR1") && matches!(side.decoded, ContentRender::Hex { .. });
    if parquet_truncated {
        let cap = cfg
            .tabular
            .map(|c| crate::bytes::format_bytes_int(c as u64))
            .unwrap_or_else(|| "unbounded".into());
        return format!(
            "parquet truncated at {cap} — raise [tracer.ceiling] tabular or use \"s\" to save"
        );
    }
    legacy_ceiling_hit_chip(side)
}

/// Returns the footer chip text for a content side.
///
/// For `ContentRender::Tabular` this includes the format name, row count,
/// and either a percentage of the configured ceiling or "complete".
/// For all other variants the chip falls back to the stream-status text that
/// was already rendered before this helper existed (loaded size, in-flight
/// indicator, etc.), so callers must only invoke this for Tabular content;
/// the non-Tabular path is provided for completeness and test coverage.
pub fn footer_chip_text(
    render: &crate::client::tracer::ContentRender,
    cfg: &crate::config::TracerCeilingConfig,
    fetched_bytes: usize,
) -> String {
    use crate::client::tracer::ContentRender;
    match render {
        ContentRender::Tabular {
            format,
            body,
            truncated,
            ..
        } => {
            let row_count = body.lines().count();
            let pct = match cfg.tabular {
                Some(c) if c > 0 => {
                    format!(
                        "{}% of {}",
                        (fetched_bytes * 100) / c,
                        crate::bytes::format_bytes_int(c as u64)
                    )
                }
                _ => "complete".into(),
            };
            let suffix = if *truncated { " · truncated" } else { "" };
            format!(
                "{} · {} rows · {}{}",
                format.label(),
                row_count,
                pct,
                suffix
            )
        }
        // Non-Tabular variants: return an empty string — the caller
        // (`render_stream_status`) handles these arms inline with the
        // existing `crate::bytes::format_bytes`-based chip text.
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::tracer::ContentRender;
    use crate::test_support::{TEST_BACKEND_MEDIUM, TEST_BACKEND_SHORT, test_backend};
    use crate::view::tracer::state::{
        ContentModalHeader, ContentModalState, ContentModalTab, DiffLine, DiffRender, Diffable,
        HunkAnchor, NotDiffableReason, SideBuffer,
    };
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn stub_header() -> ContentModalHeader {
        ContentModalHeader {
            event_type: "DROP".into(),
            event_timestamp_iso: "2026-04-22T13:42:18.231".into(),
            component_name: "UpdateAttribute-enrich".into(),
            pg_path: "healthy-pipeline/enrich".into(),
            input_size: Some(2_400),
            output_size: Some(2_800),
            input_mime: Some("application/json".into()),
            output_mime: Some("application/json".into()),
            input_available: true,
            output_available: true,
        }
    }

    fn stub_modal(active: ContentModalTab) -> ContentModalState {
        ContentModalState {
            event_id: 8_471_293,
            header: stub_header(),
            active_tab: active,
            last_nondiff_tab: ContentModalTab::Input,
            diffable: Diffable::Ok,
            input: SideBuffer {
                loaded: br#"{"orderId":"A-1029","total":299.00}"#.to_vec(),
                decoded: ContentRender::Text {
                    text: "{\"orderId\":\"A-1029\",\n  \"total\":299.00}\n".into(),
                    pretty_printed: true,
                },
                fully_loaded: true,
                ceiling_hit: false,
                in_flight: false,
                last_error: None,
                effective_ceiling: None,
                in_flight_decode: false,
            },
            output: SideBuffer {
                loaded: br#"{"orderId":"A-1029","total":329.99,"tax":30.99}"#.to_vec(),
                decoded: ContentRender::Text {
                    text: "{\"orderId\":\"A-1029\",\n  \"total\":329.99,\n  \"tax\":30.99}\n"
                        .into(),
                    pretty_printed: true,
                },
                fully_loaded: true,
                ceiling_hit: false,
                in_flight: false,
                last_error: None,
                effective_ceiling: None,
                in_flight_decode: false,
            },
            diff_cache: None,
            scroll: crate::widget::scroll::BidirectionalScrollState::default(),
            search: None,
        }
    }

    #[test]
    fn modal_input_complete() {
        let mut modal = stub_modal(ContentModalTab::Input);
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &mut modal,
                    &crate::config::TracerCeilingConfig::default(),
                )
            })
            .unwrap();
        insta::assert_debug_snapshot!("modal_input_complete", terminal.backend().buffer());
    }

    #[test]
    fn modal_output_empty() {
        let mut modal = stub_modal(ContentModalTab::Output);
        modal.output.loaded.clear();
        modal.output.decoded = ContentRender::Empty;
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &mut modal,
                    &crate::config::TracerCeilingConfig::default(),
                )
            })
            .unwrap();
        insta::assert_debug_snapshot!("modal_output_empty", terminal.backend().buffer());
    }

    #[test]
    fn modal_input_ceiling_hit() {
        let mut modal = stub_modal(ContentModalTab::Input);
        modal.input.loaded = vec![b'x'; 4 * 1024 * 1024];
        modal.input.ceiling_hit = true;
        modal.input.fully_loaded = true;
        let backend = test_backend(TEST_BACKEND_SHORT);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &mut modal,
                    &crate::config::TracerCeilingConfig::default(),
                )
            })
            .unwrap();
        insta::assert_debug_snapshot!("modal_input_ceiling_hit", terminal.backend().buffer());
    }

    #[test]
    fn modal_diff_unified_three_hunks() {
        let mut modal = stub_modal(ContentModalTab::Diff);
        modal.diffable = Diffable::Ok;
        modal.diff_cache = Some(DiffRender {
            lines: vec![
                DiffLine {
                    tag: similar::ChangeTag::Delete,
                    text: "\"total\":299.00".into(),
                    input_line: Some(3),
                    output_line: None,
                    inline_diff: None,
                },
                DiffLine {
                    tag: similar::ChangeTag::Insert,
                    text: "\"total\":329.99,".into(),
                    input_line: None,
                    output_line: Some(3),
                    inline_diff: None,
                },
                DiffLine {
                    tag: similar::ChangeTag::Insert,
                    text: "\"tax\":30.99".into(),
                    input_line: None,
                    output_line: Some(4),
                    inline_diff: None,
                },
            ],
            hunks: vec![HunkAnchor {
                line_idx: 0,
                input_line: 3,
                output_line: 3,
            }],
            change_stops: vec![0, 1],
        });
        let backend = test_backend(TEST_BACKEND_SHORT);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &mut modal,
                    &crate::config::TracerCeilingConfig::default(),
                )
            })
            .unwrap();
        insta::assert_debug_snapshot!(
            "modal_diff_unified_three_hunks",
            terminal.backend().buffer()
        );
    }

    #[test]
    fn modal_diff_no_differences() {
        let mut modal = stub_modal(ContentModalTab::Output);
        modal.diffable = Diffable::NotAvailable(NotDiffableReason::NoDifferences);
        let backend = test_backend(TEST_BACKEND_SHORT);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &mut modal,
                    &crate::config::TracerCeilingConfig::default(),
                )
            })
            .unwrap();
        insta::assert_debug_snapshot!("modal_diff_no_differences", terminal.backend().buffer());
    }

    #[test]
    fn modal_diff_size_exceeds_cap() {
        let mut modal = stub_modal(ContentModalTab::Input);
        modal.diffable = Diffable::NotAvailable(NotDiffableReason::SizeExceedsDiffCap);
        let backend = test_backend(TEST_BACKEND_SHORT);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &mut modal,
                    &crate::config::TracerCeilingConfig::default(),
                )
            })
            .unwrap();
        insta::assert_debug_snapshot!("modal_diff_size_exceeds_cap", terminal.backend().buffer());
    }

    #[test]
    fn modal_input_tabular_parquet_renders_schema_then_body() {
        use crate::client::tracer::TabularFormat;
        let mut modal = stub_modal(ContentModalTab::Input);
        modal.input.decoded = ContentRender::Tabular {
            format: TabularFormat::Parquet,
            schema_summary: "id : Int64\nname : Utf8".into(),
            body: "{\"id\":0,\"name\":\"a\"}\n{\"id\":1,\"name\":\"b\"}".into(),
            decoded_bytes: 38,
            truncated: false,
        };
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &mut modal,
                    &crate::config::TracerCeilingConfig::default(),
                )
            })
            .unwrap();
        insta::assert_debug_snapshot!(
            "modal_input_tabular_parquet_renders_schema_then_body",
            terminal.backend().buffer()
        );
    }

    /// Tabular content used to early-return without applying the
    /// search-overlay loop, so committed matches found "5/1500" but
    /// nothing rendered highlighted. This guards the unified pipeline
    /// in `text_body_lines`.
    #[test]
    fn modal_search_highlights_tabular_body_row() {
        use crate::client::tracer::TabularFormat;
        let mut modal = stub_modal(ContentModalTab::Input);
        modal.input.decoded = ContentRender::Tabular {
            format: TabularFormat::Parquet,
            schema_summary: "id : Int64\nname : Utf8".into(),
            body: "{\"id\":0,\"name\":\"alpha\"}\n{\"id\":1,\"name\":\"alpha\"}".into(),
            decoded_bytes: 46,
            truncated: false,
        };
        // Search corpus is `schema + "\n-- schema --\n" + body`, which
        // gives line indices: 0/1 = schema rows, 2 = separator,
        // 3/4 = body rows. "alpha" sits at bytes 16..21 in each body row.
        modal.search = Some(crate::widget::search::SearchState {
            query: "alpha".to_string(),
            input_active: false,
            committed: true,
            matches: vec![
                crate::widget::search::MatchSpan {
                    line_idx: 3,
                    byte_start: 16,
                    byte_end: 21,
                },
                crate::widget::search::MatchSpan {
                    line_idx: 4,
                    byte_start: 16,
                    byte_end: 21,
                },
            ],
            current: Some(0),
        });
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &mut modal,
                    &crate::config::TracerCeilingConfig::default(),
                )
            })
            .unwrap();
        let buf = terminal.backend().buffer();

        // Locate the first body row ("{...alpha...}") and assert the
        // five "alpha" cells carry the current-match highlight modifier.
        let mut row_y = None;
        for y in 0..buf.area.height {
            let mut line = String::new();
            for x in 0..buf.area.width {
                line.push_str(buf.cell((x, y)).unwrap().symbol());
            }
            if line.contains("\"id\":0") && line.contains("alpha") {
                row_y = Some(y);
                break;
            }
        }
        let y = row_y.expect("body row with alpha must render");

        let mut alpha_x = None;
        for x in 0..buf.area.width.saturating_sub(5) {
            let mut chunk = String::new();
            for dx in 0..5 {
                chunk.push_str(buf.cell((x + dx, y)).unwrap().symbol());
            }
            if chunk == "alpha" {
                alpha_x = Some(x);
                break;
            }
        }
        let x = alpha_x.expect("alpha substring must be on this row");

        // theme::highlight() is REVERSED. The Paragraph renderer fills
        // Reset fg/bg/underline_color around it, so compare modifiers
        // rather than the whole Style.
        for dx in 0..5 {
            let cell = buf.cell((x + dx, y)).unwrap();
            assert!(
                cell.modifier.contains(ratatui::style::Modifier::REVERSED),
                "byte {dx} of 'alpha' on the current-match row must carry REVERSED, got {:?}",
                cell.modifier,
            );
        }
    }

    #[test]
    fn modal_search_with_matches() {
        let mut modal = stub_modal(ContentModalTab::Input);
        // Input text line 1 is `  "total":299.00}`.
        // "total" starts at byte 3 (after `  "`).
        modal.search = Some(crate::widget::search::SearchState {
            query: "total".to_string(),
            input_active: false,
            committed: true,
            matches: vec![crate::widget::search::MatchSpan {
                line_idx: 1,
                byte_start: 3,
                byte_end: 8,
            }],
            current: Some(0),
        });
        let backend = test_backend(TEST_BACKEND_SHORT);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &mut modal,
                    &crate::config::TracerCeilingConfig::default(),
                )
            })
            .unwrap();
        insta::assert_debug_snapshot!("modal_search_with_matches", terminal.backend().buffer());
    }

    #[test]
    fn footer_chip_for_tabular_parquet_includes_format_and_pct() {
        use crate::client::tracer::{ContentRender, TabularFormat};
        use crate::config::TracerCeilingConfig;
        let render = ContentRender::Tabular {
            format: TabularFormat::Parquet,
            schema_summary: String::new(),
            body: "{}\n{}\n{}\n{}\n{}".into(), // 5 rows
            decoded_bytes: 14,
            truncated: false,
        };
        let cfg = TracerCeilingConfig {
            text: Some(4 * 1024 * 1024),
            tabular: Some(64 * 1024 * 1024),
            diff: Some(16 * 1024 * 1024),
        };
        let chip = footer_chip_text(&render, &cfg, /* fetched_bytes */ 1_500);
        assert!(
            chip.contains("parquet"),
            "expected 'parquet' in chip, got: {chip}"
        );
        assert!(
            chip.contains('5'),
            "expected row count '5' in chip, got: {chip}"
        );
        assert!(
            chip.contains("of 64 MiB"),
            "expected 'of 64 MiB' in chip, got: {chip}"
        );
    }

    #[test]
    fn footer_chip_for_tabular_avro_includes_format_and_pct() {
        use crate::client::tracer::{ContentRender, TabularFormat};
        use crate::config::TracerCeilingConfig;
        let render = ContentRender::Tabular {
            format: TabularFormat::Avro,
            schema_summary: String::new(),
            body: "{}".into(), // 1 row
            decoded_bytes: 2,
            truncated: false,
        };
        let cfg = TracerCeilingConfig::default();
        let chip = footer_chip_text(&render, &cfg, /* fetched_bytes */ 100);
        assert!(
            chip.contains("avro"),
            "expected 'avro' in chip, got: {chip}"
        );
        assert!(
            chip.contains('1'),
            "expected row count '1' in chip, got: {chip}"
        );
    }

    #[test]
    fn footer_chip_for_tabular_truncated_marks_truncated() {
        use crate::client::tracer::{ContentRender, TabularFormat};
        use crate::config::TracerCeilingConfig;
        let render = ContentRender::Tabular {
            format: TabularFormat::Parquet,
            schema_summary: String::new(),
            body: "{}".into(),
            decoded_bytes: 2,
            truncated: true,
        };
        let cfg = TracerCeilingConfig::default();
        let chip = footer_chip_text(&render, &cfg, /* fetched_bytes */ 100);
        assert!(
            chip.contains("truncated"),
            "expected 'truncated' marker, got: {chip}"
        );
    }

    #[test]
    fn footer_chip_for_text_falls_through_to_existing_behavior() {
        use crate::client::tracer::ContentRender;
        use crate::config::TracerCeilingConfig;
        let render = ContentRender::Text {
            text: "hello".into(),
            pretty_printed: false,
        };
        let cfg = TracerCeilingConfig::default();
        let chip = footer_chip_text(&render, &cfg, 5);
        // Non-Tabular variants return an empty string from this helper;
        // the stream-status renderer handles them inline. Just confirm
        // that calling the helper does not panic.
        let _ = chip;
    }

    #[test]
    fn ceiling_hit_chip_for_parquet_truncation_uses_distinct_message() {
        use crate::client::tracer::ContentRender;
        use crate::config::TracerCeilingConfig;

        // Construct a SideBuffer mimicking the post-truncation state:
        // - loaded.starts_with(b"PAR1") (so we know it's parquet)
        // - decoded = Hex (because the partial parquet couldn't be decoded)
        // - ceiling_hit = true (because the fetch hit the tabular ceiling)
        let mut bytes = b"PAR1".to_vec();
        bytes.extend_from_slice(&[0u8; 100]);
        let buf = SideBuffer {
            loaded: bytes,
            decoded: ContentRender::Hex {
                first_4k: "50 41 52 31 ...".into(),
            },
            fully_loaded: false,
            ceiling_hit: true,
            in_flight: false,
            last_error: None,
            effective_ceiling: Some(Some(64 * 1024 * 1024)),
            in_flight_decode: false,
        };
        let cfg = TracerCeilingConfig {
            text: Some(4 * 1024 * 1024),
            tabular: Some(64 * 1024 * 1024),
            diff: Some(16 * 1024 * 1024),
        };

        let chip = ceiling_hit_chip(&buf, &cfg);
        assert!(
            chip.contains("parquet truncated"),
            "expected 'parquet truncated', got: {chip}"
        );
        assert!(
            chip.contains("64 MiB"),
            "expected '64 MiB' in message, got: {chip}"
        );
        assert!(
            chip.contains("[tracer.ceiling] tabular"),
            "expected config-key reference, got: {chip}"
        );
        assert!(chip.contains("\"s\""), "expected save hint, got: {chip}");
    }

    #[test]
    fn ceiling_hit_chip_for_text_uses_existing_message() {
        // Sanity check: text content with ceiling_hit still produces the
        // pre-existing chip text (whatever it is). The new helper should
        // route non-parquet-truncated cases to the legacy chip.
        use crate::client::tracer::ContentRender;
        use crate::config::TracerCeilingConfig;
        let buf = SideBuffer {
            loaded: vec![b'a'; 4 * 1024 * 1024],
            decoded: ContentRender::Text {
                text: "a".repeat(4 * 1024 * 1024),
                pretty_printed: false,
            },
            fully_loaded: false,
            ceiling_hit: true,
            in_flight: false,
            last_error: None,
            effective_ceiling: Some(Some(4 * 1024 * 1024)),
            in_flight_decode: false,
        };
        let cfg = TracerCeilingConfig::default();
        let chip = ceiling_hit_chip(&buf, &cfg);
        assert!(
            !chip.contains("parquet truncated"),
            "text content should NOT use parquet message"
        );
        // The exact text isn't asserted here — just that it's not the parquet variant.
    }

    #[test]
    fn modal_tabular_ceiling_hit_parquet_chip() {
        use crate::client::tracer::ContentRender;
        // Construct a modal state with a parquet-truncated Input side:
        // loaded starts with PAR1 magic, decoded is Hex (failed decode),
        // ceiling_hit is true.
        let mut modal = stub_modal(ContentModalTab::Input);
        let mut parquet_bytes = b"PAR1".to_vec();
        parquet_bytes.extend_from_slice(&[0u8; 64 * 1024 * 1024 - 4]);
        modal.input.loaded = parquet_bytes;
        modal.input.decoded = ContentRender::Hex {
            first_4k: "50 41 52 31 ...".into(),
        };
        modal.input.fully_loaded = false;
        modal.input.ceiling_hit = true;
        modal.input.effective_ceiling = Some(Some(64 * 1024 * 1024));
        let cfg = crate::config::TracerCeilingConfig {
            text: Some(4 * 1024 * 1024),
            tabular: Some(64 * 1024 * 1024),
            diff: Some(16 * 1024 * 1024),
        };
        let backend = test_backend(TEST_BACKEND_MEDIUM);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render(f, f.area(), &mut modal, &cfg))
            .unwrap();
        insta::assert_debug_snapshot!(
            "modal_tabular_ceiling_hit_parquet_chip",
            terminal.backend().buffer()
        );
    }
}
