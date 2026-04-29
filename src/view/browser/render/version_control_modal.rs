//! Version-control modal renderer. Identity panel + diff body + footer.
//! Below `widget::modal::MIN_WIDTH × MIN_HEIGHT` degrades to a centered
//! "terminal too small" line via `widget::modal::render_too_small`.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::theme;
use crate::view::browser::state::version_control_modal::short_id;
use crate::view::browser::state::{VersionControlDifferenceLoad, VersionControlModalState};
use crate::widget::panel::Panel;
use crate::widget::search::{MatchSpan, SearchState};

pub fn render(frame: &mut Frame, area: Rect, modal: &VersionControlModalState) {
    if crate::widget::modal::render_too_small(frame, area) {
        return;
    }

    frame.render_widget(Clear, area);
    let outer_title = Line::from(vec![
        Span::raw(" "),
        Span::styled(modal.pg_name.as_str(), theme::accent()),
        Span::raw(" "),
        Span::styled("·", theme::muted()),
        Span::raw(" "),
        Span::styled("version control", theme::muted()),
        Span::raw(" "),
    ]);
    let outer = Panel::new(outer_title).into_block();
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Fill(1),
            Constraint::Length(2),
        ])
        .split(inner);

    render_identity(frame, rows[0], modal);
    render_diff_body(frame, rows[1], modal);
    render_footer(frame, rows[2], modal);
}

fn render_identity(frame: &mut Frame, area: Rect, modal: &VersionControlModalState) {
    let block = Block::default().borders(Borders::ALL).title(" Identity ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let id = match &modal.identity {
        Some(s) => s,
        None => {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled("loading…", theme::muted()))),
                inner,
            );
            return;
        }
    };
    use nifi_rust_client::dynamic::types::VersionControlInformationDtoState as S;
    let state_word = match id.state {
        S::UpToDate => "UP TO DATE",
        S::Stale => "STALE",
        S::LocallyModified => "MODIFIED",
        S::LocallyModifiedAndStale => "STALE+MOD",
        S::SyncFailure => "SYNC-ERR",
        _ => "UNKNOWN",
    };
    let state_style: Style = match id.state {
        S::UpToDate => theme::muted(),
        S::Stale | S::LocallyModified | S::LocallyModifiedAndStale => theme::warning(),
        S::SyncFailure => theme::error(),
        _ => theme::muted(),
    };
    let registry = format!(
        "{} / bucket={}{}",
        id.registry_name.as_deref().unwrap_or("?"),
        id.bucket_name.as_deref().unwrap_or("?"),
        id.branch
            .as_deref()
            .map(|b| format!(" / branch={b}"))
            .unwrap_or_default(),
    );
    let flow = format!(
        "{} · flow_id={} · v{}",
        id.flow_name.as_deref().unwrap_or("?"),
        short_id(id.flow_id.as_deref().unwrap_or("?")),
        id.version.as_deref().unwrap_or("?"),
    );
    let why = id.state_explanation.as_deref().unwrap_or("—");
    let lines = vec![
        Line::from(vec![
            Span::styled(format!("{:<10}", "registry"), theme::muted()),
            Span::raw(registry),
        ]),
        Line::from(vec![
            Span::styled(format!("{:<10}", "flow"), theme::muted()),
            Span::raw(flow),
        ]),
        Line::from(vec![
            Span::styled(format!("{:<10}", "state"), theme::muted()),
            Span::styled(state_word, state_style.add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled(format!("{:<10}", "why"), theme::muted()),
            Span::styled(why.to_string(), theme::muted()),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_diff_body(frame: &mut Frame, area: Rect, modal: &VersionControlModalState) {
    match &modal.differences {
        VersionControlDifferenceLoad::Pending => {
            let line = Line::from(Span::styled("loading…", theme::muted()));
            frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), area);
        }
        VersionControlDifferenceLoad::Failed(err) => {
            let lines = vec![
                Line::from(Span::styled("failed to load:", theme::error())),
                Line::from(Span::styled(err.clone(), theme::error())),
                Line::from(""),
                Line::from(Span::styled("press r to retry", theme::muted())),
            ];
            frame.render_widget(Paragraph::new(lines), area);
        }
        VersionControlDifferenceLoad::Loaded(sections) => {
            // Filter out environmental diffs when show_environmental is false.
            // Sections whose remaining diffs are zero are collapsed entirely.
            let visible: Vec<(
                &crate::client::ComponentDiffSection,
                Vec<&crate::client::RenderedDifference>,
            )> = sections
                .iter()
                .filter_map(|s| {
                    let kept: Vec<_> = s
                        .differences
                        .iter()
                        .filter(|d| modal.show_environmental || !d.environmental)
                        .collect();
                    if kept.is_empty() {
                        None
                    } else {
                        Some((s, kept))
                    }
                })
                .collect();
            if visible.is_empty() {
                let line = Line::from(Span::styled("no local modifications", theme::muted()));
                frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), area);
                return;
            }
            // Build the body lines.
            let mut all_lines: Vec<Line<'static>> = Vec::new();
            for (section, diffs) in &visible {
                // Section header line — accent-colored.
                let header = format!(
                    "─ {} · {} · {} ─",
                    section.component_type,
                    section.display_label,
                    short_id(&section.component_id)
                );
                all_lines.push(Line::from(Span::styled(header, theme::accent())));
                for d in diffs {
                    let style = if d.environmental {
                        theme::muted()
                    } else {
                        Style::default()
                    };
                    let line = Line::from(vec![
                        Span::styled(format!("{:<18}", d.kind), theme::muted()),
                        Span::raw(" "),
                        Span::styled(d.description.clone(), style),
                    ]);
                    all_lines.push(line);
                }
                all_lines.push(Line::from(""));
            }
            // Apply search highlights if a committed search is active.
            // Match spans are computed against `searchable_body`, which
            // is byte-aligned with `all_lines` above, so the per-line
            // `(byte_start, byte_end)` offsets land cleanly on the
            // rendered plaintext.
            if let Some(search) = modal.search.as_ref()
                && search.committed
                && !search.matches.is_empty()
            {
                apply_search_highlights(&mut all_lines, search);
            }
            // Apply scroll offset. `BidirectionalScrollState.vertical.offset`
            // is a usize that the modal verbs already mutate via
            // `VerticalScrollState::scroll_by` / `page_up` / `page_down`.
            let scroll_y = modal.scroll.vertical.offset as u16;
            let p = Paragraph::new(all_lines)
                .scroll((scroll_y, 0))
                .wrap(ratatui::widgets::Wrap { trim: false });
            frame.render_widget(p, area);
        }
    }
}

/// Re-build each line's spans, splitting at every match's
/// `(byte_start, byte_end)` and applying `UNDERLINED` (plus
/// `REVERSED | BOLD` for the active match identified by
/// `search.current`). Caller guarantees the line indices match the
/// `searchable_body` used by `compute_matches`. This loses per-row
/// styling on matched lines — matching the trade-off the Bulletins
/// detail modal accepts.
fn apply_search_highlights(lines: &mut [Line<'static>], search: &SearchState) {
    for (line_idx, line) in lines.iter_mut().enumerate() {
        let per_line: Vec<(usize, &MatchSpan)> = search
            .matches
            .iter()
            .enumerate()
            .filter(|(_, m)| m.line_idx == line_idx)
            .collect();
        if per_line.is_empty() {
            continue;
        }
        // Reconstruct the line's plaintext to drive byte-offset slicing.
        let plain: String = line.spans.iter().map(|s| s.content.as_ref()).collect();

        let mut new_spans: Vec<Span<'static>> = Vec::new();
        let mut cursor = 0usize;
        for (global_idx, m) in per_line {
            if m.byte_start > cursor {
                new_spans.push(Span::raw(plain[cursor..m.byte_start].to_string()));
            }
            let hit = plain[m.byte_start..m.byte_end].to_string();
            let style = if search.current == Some(global_idx) {
                theme::search_match_active()
            } else {
                theme::search_match()
            };
            new_spans.push(Span::styled(hit, style));
            cursor = m.byte_end;
        }
        if cursor < plain.len() {
            new_spans.push(Span::raw(plain[cursor..].to_string()));
        }
        if new_spans.is_empty() {
            new_spans.push(Span::raw(""));
        }
        *line = Line::from(new_spans);
    }
}

fn render_footer(frame: &mut Frame, area: Rect, modal: &VersionControlModalState) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    render_footer_status(frame, rows[0], modal);
    render_footer_hint(frame, rows[1], modal);
}

fn render_footer_status(frame: &mut Frame, area: Rect, modal: &VersionControlModalState) {
    // While the user is actively typing into the search bar, show
    // `/ {query}_` instead of the diff-count status.
    if let Some(s) = modal.search.as_ref()
        && s.input_active
    {
        let line = Line::from(vec![
            Span::styled("/ ".to_string(), theme::accent()),
            Span::raw(s.query.clone()),
            Span::styled("_".to_string(), theme::search_cursor()),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }

    let env_label = if modal.show_environmental {
        "env shown"
    } else {
        "env hidden"
    };
    let status = match &modal.differences {
        VersionControlDifferenceLoad::Pending => "loading…".to_string(),
        VersionControlDifferenceLoad::Failed(_) => "failed — press r to retry".to_string(),
        VersionControlDifferenceLoad::Loaded(sections) => {
            let mut diff_count = 0usize;
            let mut comp_count = 0usize;
            for s in sections {
                let kept = s
                    .differences
                    .iter()
                    .filter(|d| modal.show_environmental || !d.environmental)
                    .count();
                if kept > 0 {
                    comp_count += 1;
                    diff_count += kept;
                }
            }
            format!(
                "{} differences across {} components · {}",
                diff_count, comp_count, env_label
            )
        }
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(status, theme::muted()))),
        area,
    );
}

fn render_footer_hint(frame: &mut Frame, area: Rect, _modal: &VersionControlModalState) {
    use crate::input::Verb;
    use crate::input::VersionControlModalVerb;
    crate::widget::modal::render_verb_hint_strip(frame, area, VersionControlModalVerb::all());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::snapshot::VersionControlSummary;
    use crate::test_support::test_backend;
    use crate::view::browser::state::VersionControlModalState;
    use insta::assert_snapshot;
    use nifi_rust_client::dynamic::types::VersionControlInformationDtoState;
    use ratatui::Terminal;

    fn modal_with_identity(state: VersionControlInformationDtoState) -> VersionControlModalState {
        VersionControlModalState::pending(
            "pg-1".into(),
            "ingest".into(),
            Some(VersionControlSummary {
                state,
                registry_name: Some("ops-registry".into()),
                bucket_name: Some("ops".into()),
                branch: Some("main".into()),
                flow_id: Some("4f3a-aaaa".into()),
                flow_name: Some("diff-pipeline".into()),
                version: Some("3".into()),
                state_explanation: Some(
                    "Local changes have been made and a newer version exists".into(),
                ),
            }),
        )
    }

    #[test]
    fn renders_identity_for_stale_modified() {
        let mut term = Terminal::new(test_backend(24)).unwrap();
        let modal = modal_with_identity(VersionControlInformationDtoState::LocallyModifiedAndStale);
        term.draw(|f| render(f, f.area(), &modal)).unwrap();
        assert_snapshot!(
            "vc_modal_identity_stale_modified",
            format!("{}", term.backend())
        );
    }

    #[test]
    fn pending_diff_body_shows_loading() {
        let mut term = Terminal::new(test_backend(24)).unwrap();
        let modal = modal_with_identity(VersionControlInformationDtoState::Stale);
        term.draw(|f| render(f, f.area(), &modal)).unwrap();
        assert_snapshot!("vc_modal_pending_loading", format!("{}", term.backend()));
    }

    #[test]
    fn renders_identity_for_sync_failure() {
        let mut term = Terminal::new(test_backend(24)).unwrap();
        let modal = modal_with_identity(VersionControlInformationDtoState::SyncFailure);
        term.draw(|f| render(f, f.area(), &modal)).unwrap();
        assert_snapshot!(
            "vc_modal_identity_sync_failure",
            format!("{}", term.backend())
        );
    }

    fn loaded_modal_with(
        state: VersionControlInformationDtoState,
        show_env: bool,
        sections: Vec<crate::client::ComponentDiffSection>,
    ) -> VersionControlModalState {
        let mut m = modal_with_identity(state);
        m.show_environmental = show_env;
        m.differences = crate::view::browser::state::VersionControlDifferenceLoad::Loaded(sections);
        m
    }

    #[test]
    fn loaded_diff_body_renders_sections_by_component() {
        use crate::client::{ComponentDiffSection, RenderedDifference};
        let mut term = Terminal::new(test_backend(28)).unwrap();
        let modal = loaded_modal_with(
            VersionControlInformationDtoState::LocallyModifiedAndStale,
            false,
            vec![
                ComponentDiffSection {
                    component_id: "4f3aaaaa".into(),
                    component_name: "UpdateRecord-enrich".into(),
                    component_type: "Processor".into(),
                    display_label: "UpdateRecord-enrich".into(),
                    differences: vec![
                        RenderedDifference {
                            kind: "PROPERTY_CHANGED".into(),
                            description: "\"Record Reader\"  'csv-reader' → 'json-reader'".into(),
                            environmental: false,
                        },
                        RenderedDifference {
                            kind: "BUNDLE_CHANGED".into(),
                            description: "Bundle upgraded".into(),
                            environmental: true,
                        },
                    ],
                },
                ComponentDiffSection {
                    component_id: "71b2bbbb".into(),
                    component_name: "csv→log".into(),
                    component_type: "Connection".into(),
                    display_label: "csv→log".into(),
                    differences: vec![RenderedDifference {
                        kind: "COMPONENT_REMOVED".into(),
                        description: "selected relationship 'retry'".into(),
                        environmental: false,
                    }],
                },
            ],
        );
        term.draw(|f| render(f, f.area(), &modal)).unwrap();
        assert_snapshot!(
            "vc_modal_loaded_two_components",
            format!("{}", term.backend())
        );
    }

    #[test]
    fn environmental_hidden_collapses_section_when_only_env_diffs() {
        use crate::client::{ComponentDiffSection, RenderedDifference};
        let mut term = Terminal::new(test_backend(24)).unwrap();
        let modal = loaded_modal_with(
            VersionControlInformationDtoState::Stale,
            false,
            vec![ComponentDiffSection {
                component_id: "4f3aaaaa".into(),
                component_name: "UpdateRecord".into(),
                component_type: "Processor".into(),
                display_label: "UpdateRecord".into(),
                differences: vec![RenderedDifference {
                    kind: "BUNDLE_CHANGED".into(),
                    description: "Bundle upgraded".into(),
                    environmental: true,
                }],
            }],
        );
        term.draw(|f| render(f, f.area(), &modal)).unwrap();
        assert_snapshot!(
            "vc_modal_env_hidden_collapses",
            format!("{}", term.backend())
        );
    }

    #[test]
    fn environmental_shown_renders_dimmed_inline() {
        use crate::client::{ComponentDiffSection, RenderedDifference};
        let mut term = Terminal::new(test_backend(24)).unwrap();
        let modal = loaded_modal_with(
            VersionControlInformationDtoState::Stale,
            true,
            vec![ComponentDiffSection {
                component_id: "4f3aaaaa".into(),
                component_name: "UpdateRecord".into(),
                component_type: "Processor".into(),
                display_label: "UpdateRecord".into(),
                differences: vec![RenderedDifference {
                    kind: "BUNDLE_CHANGED".into(),
                    description: "Bundle upgraded".into(),
                    environmental: true,
                }],
            }],
        );
        term.draw(|f| render(f, f.area(), &modal)).unwrap();
        assert_snapshot!("vc_modal_env_shown_dimmed", format!("{}", term.backend()));
    }

    #[test]
    fn loaded_empty_renders_no_local_modifications() {
        let mut term = Terminal::new(test_backend(24)).unwrap();
        let modal = loaded_modal_with(
            VersionControlInformationDtoState::UpToDate,
            false,
            Vec::new(),
        );
        term.draw(|f| render(f, f.area(), &modal)).unwrap();
        assert_snapshot!("vc_modal_loaded_empty", format!("{}", term.backend()));
    }

    #[test]
    fn failed_renders_error_with_retry_hint() {
        let mut term = Terminal::new(test_backend(24)).unwrap();
        let mut modal = modal_with_identity(VersionControlInformationDtoState::SyncFailure);
        modal.differences = crate::view::browser::state::VersionControlDifferenceLoad::Failed(
            "registry unreachable: timeout".into(),
        );
        term.draw(|f| render(f, f.area(), &modal)).unwrap();
        assert_snapshot!("vc_modal_failed", format!("{}", term.backend()));
    }

    #[test]
    fn footer_shows_diff_count_and_env_state() {
        use crate::client::{ComponentDiffSection, RenderedDifference};
        let mut term = Terminal::new(test_backend(28)).unwrap();
        let modal = loaded_modal_with(
            VersionControlInformationDtoState::LocallyModified,
            false,
            vec![ComponentDiffSection {
                component_id: "abcdabcd".into(),
                component_name: "X".into(),
                component_type: "Processor".into(),
                display_label: "X".into(),
                differences: vec![
                    RenderedDifference {
                        kind: "PROPERTY_CHANGED".into(),
                        description: "a".into(),
                        environmental: false,
                    },
                    RenderedDifference {
                        kind: "PROPERTY_CHANGED".into(),
                        description: "b".into(),
                        environmental: false,
                    },
                    RenderedDifference {
                        kind: "BUNDLE_CHANGED".into(),
                        description: "c".into(),
                        environmental: true,
                    },
                ],
            }],
        );
        term.draw(|f| render(f, f.area(), &modal)).unwrap();
        // Footer should read: "2 differences across 1 components · env hidden"
        assert_snapshot!("vc_modal_footer_diff_count", format!("{}", term.backend()));
    }

    #[test]
    fn footer_pending_shows_loading() {
        let mut term = Terminal::new(test_backend(24)).unwrap();
        let modal = modal_with_identity(VersionControlInformationDtoState::Stale);
        // differences stays Pending
        term.draw(|f| render(f, f.area(), &modal)).unwrap();
        assert_snapshot!("vc_modal_footer_pending", format!("{}", term.backend()));
    }

    #[test]
    fn search_highlights_matched_spans_in_diff_body() {
        use crate::client::{ComponentDiffSection, RenderedDifference};
        use crate::view::browser::state::VersionControlDifferenceLoad;
        use crate::widget::search::{MatchSpan, SearchState, compute_matches};

        let mut term = Terminal::new(test_backend(28)).unwrap();
        let mut modal = modal_with_identity(VersionControlInformationDtoState::LocallyModified);
        modal.differences = VersionControlDifferenceLoad::Loaded(vec![ComponentDiffSection {
            component_id: "abcdabcd".into(),
            component_name: "X".into(),
            component_type: "Processor".into(),
            display_label: "X".into(),
            differences: vec![
                RenderedDifference {
                    kind: "PROPERTY_CHANGED".into(),
                    description: "Record Reader changed".into(),
                    environmental: false,
                },
                RenderedDifference {
                    kind: "PROPERTY_CHANGED".into(),
                    description: "Record Writer changed".into(),
                    environmental: false,
                },
            ],
        }]);
        let body = modal.searchable_body();
        let matches: Vec<MatchSpan> = compute_matches(&body, "Record");
        modal.search = Some(SearchState {
            query: "Record".into(),
            input_active: false,
            committed: true,
            matches,
            current: Some(1),
        });
        term.draw(|f| render(f, f.area(), &modal)).unwrap();
        assert_snapshot!("vc_modal_search_highlights", format!("{}", term.backend()));
    }

    #[test]
    fn footer_shows_search_input_strip_when_active() {
        use crate::widget::search::SearchState;
        let mut term = Terminal::new(test_backend(24)).unwrap();
        let mut modal = modal_with_identity(VersionControlInformationDtoState::Stale);
        modal.search = Some(SearchState {
            query: "Record".into(),
            input_active: true,
            committed: false,
            matches: Vec::new(),
            current: None,
        });
        term.draw(|f| render(f, f.area(), &modal)).unwrap();
        assert_snapshot!(
            "vc_modal_footer_search_input_strip",
            format!("{}", term.backend())
        );
    }

    #[test]
    fn footer_failed_shows_retry_hint() {
        let mut term = Terminal::new(test_backend(24)).unwrap();
        let mut modal = modal_with_identity(VersionControlInformationDtoState::SyncFailure);
        modal.differences = crate::view::browser::state::VersionControlDifferenceLoad::Failed(
            "registry unreachable".into(),
        );
        term.draw(|f| render(f, f.area(), &modal)).unwrap();
        assert_snapshot!("vc_modal_footer_failed", format!("{}", term.backend()));
    }

    #[test]
    fn below_minimum_size_shows_terminal_too_small() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(40, 15);
        let mut term = Terminal::new(backend).unwrap();
        let modal = modal_with_identity(VersionControlInformationDtoState::Stale);
        term.draw(|f| render(f, f.area(), &modal)).unwrap();
        assert_snapshot!("vc_modal_below_minimum_size", format!("{}", term.backend()));
    }

    #[test]
    fn narrow_terminal_60x20_renders_without_panic() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(60, 20);
        let mut term = Terminal::new(backend).unwrap();
        let modal = modal_with_identity(VersionControlInformationDtoState::Stale);
        term.draw(|f| render(f, f.area(), &modal)).unwrap();
        assert_snapshot!("vc_modal_narrow_60x20", format!("{}", term.backend()));
    }

    /// Regression test: `RemoteProcessGroup` component-type diffs must appear
    /// in the diff body. The renderer uses the raw `component_type` string
    /// verbatim in the section header — this test pins that behaviour so a
    /// future per-type dispatcher cannot accidentally drop RPG sections.
    #[test]
    fn diff_modal_renders_remote_process_group_section() {
        use crate::client::{ComponentDiffSection, RenderedDifference};

        let mut term = Terminal::new(test_backend(28)).unwrap();
        let modal = loaded_modal_with(
            VersionControlInformationDtoState::LocallyModified,
            false,
            vec![ComponentDiffSection {
                component_id: "rpg-aabbccdd".into(),
                component_name: "downstream-cluster".into(),
                component_type: "Remote Process Group".into(),
                display_label: "downstream-cluster".into(),
                differences: vec![RenderedDifference {
                    kind: "PROPERTY_CHANGED".into(),
                    description: "\"targetUris\"  'https://old:8080' → 'https://new:8080'".into(),
                    environmental: false,
                }],
            }],
        );
        term.draw(|f| render(f, f.area(), &modal)).unwrap();
        let out = format!("{}", term.backend());
        assert!(
            out.contains("Remote Process Group"),
            "missing RPG section header in diff body: {out:?}"
        );
        assert!(
            out.contains("targetUris"),
            "missing targetUris field diff in diff body: {out:?}"
        );
    }
}
