//! Hint-bar span collection.
//!
//! Produces the per-frame vector of [`HintSpan`]s rendered at the bottom
//! of the TUI. The logic branches on modal priority, text-input mode,
//! and the active tab's `Verb::all()` iterator.

use super::{AppState, Modal, ViewId};

/// Collect the hint spans for the current state, respecting modal priority.
pub fn collect_hints(state: &AppState) -> Vec<crate::widget::hint_bar::HintSpan> {
    use std::borrow::Cow;

    use crate::input::{AppAction, BrowserVerb, BulletinsVerb, EventsVerb, TracerVerb, Verb};
    use crate::widget::hint_bar::HintSpan;

    // Modal-priority hints remain hand-written because they're short
    // and context-specific.
    if let Some(ref modal) = state.modal {
        return modal_hints(modal);
    }

    // Text-input-focused views show their own edit-mode hint strip.
    // The keymap is bypassed in this mode; the hint bar advertises
    // the conventional type/apply/cancel contract.
    if state.text_input_is_active() {
        return vec![
            HintSpan {
                key: Cow::Borrowed("type"),
                action: Cow::Borrowed("filter"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("Enter"),
                action: Cow::Borrowed("apply"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("Esc"),
                action: Cow::Borrowed("cancel"),
                enabled: true,
            },
        ];
    }

    // Default path: tab-specific verbs only. General navigation,
    // history, tab cycling, fuzzy find, quit, and the help modal are
    // documented via `?` — no point repeating them in every frame.
    let ctx = crate::input::HintContext::new(state);
    let mut out: Vec<HintSpan> = Vec::new();

    fn push_verb<V: crate::input::Verb>(
        out: &mut Vec<HintSpan>,
        v: V,
        ctx: &crate::input::HintContext<'_>,
    ) {
        if !v.show_in_hint_bar() {
            return;
        }
        out.push(HintSpan {
            key: Cow::Owned(v.chord().display()),
            action: Cow::Borrowed(v.hint()),
            enabled: v.enabled(ctx),
        });
    }

    // Per-view verbs — these are the tab-specific commands. Disabled
    // verbs (e.g. Browser Properties with no eligible selection) stay
    // in the bar but render dim, so users learn what's possible.
    match state.current_tab {
        ViewId::Overview => {}
        ViewId::Bulletins => {
            for &v in BulletinsVerb::all() {
                push_verb(&mut out, v, &ctx);
            }
        }
        ViewId::Browser => {
            // Peek modal open → BrowserPeekVerb chords shadow the
            // outer hints. Listing focused → BrowserQueueVerb chords
            // shadow them. Otherwise → standard BrowserVerb dispatch.
            let peek_open = state
                .browser
                .queue_listing
                .as_ref()
                .and_then(|l| l.peek.as_ref())
                .is_some();
            if peek_open {
                use crate::input::BrowserPeekVerb;
                for &v in BrowserPeekVerb::all() {
                    push_verb(&mut out, v, &ctx);
                }
                // Bare `?` and Goto trailers still apply.
                push_verb(&mut out, AppAction::Goto, &ctx);
                out.push(HintSpan {
                    key: Cow::Borrowed("?"),
                    action: Cow::Borrowed("help"),
                    enabled: true,
                });
                return out;
            }
            if state.browser.listing_focused {
                use crate::input::BrowserQueueVerb;
                for &v in BrowserQueueVerb::all() {
                    push_verb(&mut out, v, &ctx);
                }
                push_verb(&mut out, AppAction::Goto, &ctx);
                out.push(HintSpan {
                    key: Cow::Borrowed("?"),
                    action: Cow::Borrowed("help"),
                    enabled: true,
                });
                return out;
            }
            // Multiple BrowserVerbs can share the same chord (e.g. both
            // `OpenProperties` and `OpenParameterContext` are bound to `p`).
            // The dispatcher routes correctly via `enabled()`, but the hint
            // bar must not show two entries for the same key. Dedup by chord,
            // preferring the enabled verb; if none is enabled, keep the first.
            use std::collections::HashMap;
            let all_browser = BrowserVerb::all();
            // Build a chord → verb map, replacing any incumbent when the
            // challenger is enabled and the incumbent is not.
            let mut by_chord: HashMap<crate::input::Chord, BrowserVerb> =
                HashMap::with_capacity(all_browser.len());
            for &v in all_browser {
                match by_chord.entry(v.chord()) {
                    std::collections::hash_map::Entry::Vacant(e) => {
                        e.insert(v);
                    }
                    std::collections::hash_map::Entry::Occupied(mut e) => {
                        if !e.get().enabled(&ctx) && v.enabled(&ctx) {
                            e.insert(v);
                        }
                    }
                }
            }
            // Re-walk `BrowserVerb::all()` to preserve the canonical ordering,
            // emitting only the verb that won its chord slot.
            for &v in all_browser {
                if by_chord.get(&v.chord()) == Some(&v) {
                    push_verb(&mut out, v, &ctx);
                }
            }
        }
        ViewId::Events => {
            for &v in EventsVerb::all() {
                push_verb(&mut out, v, &ctx);
            }
        }
        ViewId::Tracer => {
            for &v in TracerVerb::all() {
                push_verb(&mut out, v, &ctx);
            }
            // Override the "save" hint label when the content pane shows a
            // truncated preview — append "(fetches full <total>)" so the user
            // knows the save action will re-fetch the complete content.
            let save_label: Option<Cow<'static, str>> = {
                use crate::client::ContentSide;
                use crate::view::tracer::state::{ContentPane, EventDetail, TracerMode};
                if let TracerMode::Lineage(ref view) = state.tracer.mode
                    && let EventDetail::Loaded {
                        event,
                        content:
                            ContentPane::Shown {
                                truncated: true,
                                side,
                                ..
                            },
                    } = &view.event_detail
                {
                    let total_size = match side {
                        ContentSide::Input => event.input_size,
                        ContentSide::Output => event.output_size,
                    };
                    Some(match total_size {
                        Some(total) => Cow::Owned(format!(
                            "save (fetches full {})",
                            crate::view::tracer::render::human_bytes(total),
                        )),
                        None => Cow::Borrowed("save (fetches full)"),
                    })
                } else {
                    None
                }
            };
            // Rewrite the save hint span that was just pushed, but only when
            // the label differs from the static default (i.e. content is truncated).
            if let Some(label) = save_label
                && let Some(span) = out.iter_mut().rev().find(|s| s.action == "save")
            {
                span.action = label;
            }
        }
    }

    // Cross-tab goto — show when the current selection has at least one
    // actionable destination so the bar doesn't advertise a dead combo.
    push_verb(&mut out, AppAction::Goto, &ctx);

    // Trailing `?` pointer so users always know where to find the
    // full reference. Everything else (navigation, history, tab
    // cycling, quit, fuzzy find, context switcher) lives in the help
    // modal.
    out.push(HintSpan {
        key: Cow::Borrowed("?"),
        action: Cow::Borrowed("help"),
        enabled: true,
    });

    out
}

fn modal_hints(modal: &Modal) -> Vec<crate::widget::hint_bar::HintSpan> {
    use std::borrow::Cow;

    use crate::widget::hint_bar::HintSpan;
    match modal {
        Modal::Help => vec![HintSpan {
            key: Cow::Borrowed("Esc"),
            action: Cow::Borrowed("close"),
            enabled: true,
        }],
        Modal::ContextSwitcher(_) => vec![
            HintSpan {
                key: Cow::Borrowed("\u{2191}/\u{2193}"),
                action: Cow::Borrowed("nav"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("Enter"),
                action: Cow::Borrowed("switch"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("Esc"),
                action: Cow::Borrowed("cancel"),
                enabled: true,
            },
        ],
        Modal::FuzzyFind(_) => vec![
            HintSpan {
                key: Cow::Borrowed("type"),
                action: Cow::Borrowed("filter"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("Enter"),
                action: Cow::Borrowed("select"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("Esc"),
                action: Cow::Borrowed("cancel"),
                enabled: true,
            },
        ],
        Modal::Properties(_) => vec![
            HintSpan {
                key: Cow::Borrowed("\u{2191}/\u{2193}"),
                action: Cow::Borrowed("nav"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("Enter"),
                action: Cow::Borrowed("goto"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("c"),
                action: Cow::Borrowed("copy value"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("Esc"),
                action: Cow::Borrowed("close"),
                enabled: true,
            },
        ],
        Modal::ErrorDetail => vec![HintSpan {
            key: Cow::Borrowed("Esc"),
            action: Cow::Borrowed("close"),
            enabled: true,
        }],
        Modal::SaveEventContent(_) => vec![
            HintSpan {
                key: Cow::Borrowed("Enter"),
                action: Cow::Borrowed("confirm"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("Esc"),
                action: Cow::Borrowed("cancel"),
                enabled: true,
            },
        ],
        Modal::NodeDetail(_) => vec![HintSpan {
            key: Cow::Borrowed("Esc"),
            action: Cow::Borrowed("close"),
            enabled: true,
        }],
        // Task 11 adds full hint spans for the goto menu.
        Modal::GotoMenu(_) => vec![
            HintSpan {
                key: Cow::Borrowed("\u{2191}/\u{2193}"),
                action: Cow::Borrowed("nav"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("Enter"),
                action: Cow::Borrowed("goto"),
                enabled: true,
            },
            HintSpan {
                key: Cow::Borrowed("Esc"),
                action: Cow::Borrowed("cancel"),
                enabled: true,
            },
        ],
    }
}

#[cfg(test)]
pub(crate) fn modal_hints_for_test(state: &AppState) -> Vec<crate::widget::hint_bar::HintSpan> {
    state.modal.as_ref().map(modal_hints).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::collect_hints;
    use crate::app::state::ViewId;
    use crate::app::state::tests::fresh_state;

    /// On a PG row with a bound parameter context, only `p param` should
    /// appear — not a second `p props` — and it should be enabled.
    #[test]
    fn browser_hint_bar_deduplicates_p_chord_on_pg_row() {
        use crate::client::{NodeKind, NodeStatusSummary, RawNode, RecursiveSnapshot};
        use crate::cluster::snapshot::ParameterContextRef;
        use std::time::SystemTime;

        let mut s = fresh_state();
        let snap = RecursiveSnapshot {
            nodes: vec![
                RawNode {
                    parent_idx: None,
                    kind: NodeKind::ProcessGroup,
                    id: "root".into(),
                    group_id: "root".into(),
                    name: "root".into(),
                    status_summary: NodeStatusSummary::ProcessGroup {
                        running: 0,
                        stopped: 0,
                        invalid: 0,
                        disabled: 0,
                    },
                },
                RawNode {
                    parent_idx: Some(0),
                    kind: NodeKind::ProcessGroup,
                    id: "ingest".into(),
                    group_id: "root".into(),
                    name: "ingest".into(),
                    status_summary: NodeStatusSummary::ProcessGroup {
                        running: 0,
                        stopped: 0,
                        invalid: 0,
                        disabled: 0,
                    },
                },
            ],
            fetched_at: SystemTime::now(),
        };
        crate::view::browser::state::apply_tree_snapshot(&mut s.browser, snap);
        // Stamp a binding on "ingest" (arena index 1) so OpenParameterContext
        // is enabled (required since Fix 1 tightened the enabled predicate).
        s.browser.nodes[1].parameter_context_ref = Some(ParameterContextRef {
            id: "ctx-x".into(),
            name: "ctx-x".into(),
        });
        s.current_tab = ViewId::Browser;
        s.browser.selected = 1; // "ingest" PG row

        let hints = collect_hints(&s);
        let p_hints: Vec<_> = hints.iter().filter(|h| h.key == "p").collect();

        assert_eq!(
            p_hints.len(),
            1,
            "expected exactly one `p` hint on a PG row; got: {:?}",
            p_hints
        );
        // The surviving hint should be the enabled one: `param` (OpenParameterContext).
        assert_eq!(
            p_hints[0].action, "param",
            "PG row: surviving `p` hint should be `param`, got {:?}",
            p_hints[0].action
        );
        assert!(
            p_hints[0].enabled,
            "PG row: the surviving `p` hint should be enabled"
        );
    }

    /// On a Processor row, only `p props` should appear — not a second `p param`.
    #[test]
    fn browser_hint_bar_deduplicates_p_chord_on_processor_row() {
        use crate::client::{NodeKind, NodeStatusSummary, RawNode, RecursiveSnapshot};
        use std::time::SystemTime;

        let mut s = fresh_state();
        let snap = RecursiveSnapshot {
            nodes: vec![
                RawNode {
                    parent_idx: None,
                    kind: NodeKind::ProcessGroup,
                    id: "root".into(),
                    group_id: "root".into(),
                    name: "root".into(),
                    status_summary: NodeStatusSummary::ProcessGroup {
                        running: 0,
                        stopped: 0,
                        invalid: 0,
                        disabled: 0,
                    },
                },
                RawNode {
                    parent_idx: Some(0),
                    kind: NodeKind::Processor,
                    id: "gen".into(),
                    group_id: "root".into(),
                    name: "Gen".into(),
                    status_summary: NodeStatusSummary::Processor {
                        run_status: "Running".into(),
                    },
                },
            ],
            fetched_at: SystemTime::now(),
        };
        crate::view::browser::state::apply_tree_snapshot(&mut s.browser, snap);
        s.current_tab = ViewId::Browser;
        s.browser.selected = 1; // "gen" Processor row

        let hints = collect_hints(&s);
        let p_hints: Vec<_> = hints.iter().filter(|h| h.key == "p").collect();

        assert_eq!(
            p_hints.len(),
            1,
            "expected exactly one `p` hint on a Processor row; got: {:?}",
            p_hints
        );
        // The surviving hint should be the enabled one: `props` (OpenProperties).
        assert_eq!(
            p_hints[0].action, "props",
            "Processor row: surviving `p` hint should be `props`, got {:?}",
            p_hints[0].action
        );
        assert!(
            p_hints[0].enabled,
            "Processor row: the surviving `p` hint should be enabled"
        );
    }
}
