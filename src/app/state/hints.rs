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
            for &v in BrowserVerb::all() {
                push_verb(&mut out, v, &ctx);
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
