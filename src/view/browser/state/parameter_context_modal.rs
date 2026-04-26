//! State for the Browser tab's parameter-context modal.
//! Mirrors `version_control_modal` in shape: per-open struct on
//! `BrowserState.parameter_modal: Option<...>`, populated by reducers
//! when the worker resolves the chain.

use crate::client::parameter_context::{ParameterContextNode, ParameterEntry};
use crate::widget::scroll::VerticalScrollState;
use crate::widget::search::SearchState;

/// Which pane of the parameter-context modal has keyboard focus.
/// Default is `Sidebar` — the user starts on the chain list and
/// presses Enter to move focus to the Body (params table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParameterContextPane {
    #[default]
    Sidebar,
    Body,
}

#[derive(Debug, Clone)]
pub struct ParameterContextModalState {
    /// PG that triggered the open. Stays fixed for the modal session.
    pub originating_pg_id: String,
    pub originating_pg_path: String,
    /// Optional pre-selected param name from a `#{name}` cross-link.
    pub preselect: Option<String>,
    pub load: ParameterContextLoad,
    /// Which pane currently has keyboard focus. Default `Sidebar`.
    /// Reset to `Sidebar` each time the modal is opened.
    pub focused_pane: ParameterContextPane,
    /// Index into `chain` selecting which context the sidebar cursor
    /// is on. `0` is the bound (originating) context.
    pub sidebar_index: usize,
    /// Whether the right pane shows the resolved-flat view (false)
    /// or the per-context view scoped to `sidebar_index` (true).
    pub by_context_mode: bool,
    /// Whether shadowed rows are inlined under their winners.
    pub show_shadowed: bool,
    /// Whether the Used-by panel replaces the params table.
    pub show_used_by: bool,
    pub scroll: VerticalScrollState,
    pub search: Option<SearchState>,
}

#[derive(Debug, Clone)]
pub enum ParameterContextLoad {
    Loading,
    Loaded { chain: Vec<ParameterContextNode> },
    Error { message: String },
}

impl ParameterContextModalState {
    pub fn pending(
        originating_pg_id: String,
        originating_pg_path: String,
        preselect: Option<String>,
    ) -> Self {
        Self {
            originating_pg_id,
            originating_pg_path,
            preselect,
            load: ParameterContextLoad::Loading,
            focused_pane: ParameterContextPane::Sidebar,
            sidebar_index: 0,
            by_context_mode: false,
            show_shadowed: false,
            show_used_by: false,
            scroll: VerticalScrollState::default(),
            search: None,
        }
    }

    /// Build the flat string body that the search primitives operate on.
    ///
    /// The output MUST be byte-for-byte identical to the plain text that
    /// `render_flat` / `render_by_context` produce, line-for-line, so that
    /// `MatchSpan` byte offsets computed by `compute_matches` land on the
    /// correct rendered spans in `apply_search_highlights`.
    ///
    /// Column widths here mirror the constants in the render module (22/22/12).
    /// Line 0 is the column-header line; line 1 is a separator placeholder
    /// (the actual rendered separator is area-width `─` chars that searches
    /// never match, so any single-line placeholder works). Data rows start
    /// at line 2, matching `render_flat`'s `all_lines` layout.
    ///
    /// In `by_context_mode` the body mirrors `render_by_context` (same
    /// 2-line header + data rows without a `from` column).
    pub fn searchable_body(&self) -> String {
        // Column widths must mirror the render module exactly. The order is
        // flags | name | value | from. `by_context_mode` omits `from`.
        // Flags render as a single combined chip, e.g. `[SPO]`.
        const FLAG_W: usize = 5;
        const NAME_W: usize = 22;
        const VALUE_W: usize = 22;
        const FROM_W: usize = 18;

        let chain = match &self.load {
            ParameterContextLoad::Loaded { chain } => chain,
            _ => return String::new(),
        };
        let mut out = String::new();
        if self.by_context_mode {
            let Some(ctx) = chain.get(self.sidebar_index) else {
                return String::new();
            };
            // 2-line header matching render_by_context.
            out.push_str(&format!(
                "{:<FLAG_W$} {:<NAME_W$} {:<VALUE_W$}\n",
                "flags", "name", "value"
            ));
            out.push('\n'); // separator placeholder
            for entry in &ctx.parameters {
                // Combined flag chip — by_context view only emits S and P.
                let mut letters = String::new();
                if entry.sensitive {
                    letters.push('S');
                }
                if entry.provided {
                    letters.push('P');
                }
                let mut flags = if letters.is_empty() {
                    String::new()
                } else {
                    format!("[{letters}]")
                };
                while flags.chars().count() < FLAG_W {
                    flags.push(' ');
                }

                let name = truncate_for_body(&entry.name, NAME_W);
                let value = if entry.sensitive {
                    truncate_for_body("(sensitive)", VALUE_W)
                } else {
                    truncate_for_body(entry.value.as_deref().unwrap_or("\u{2014}"), VALUE_W)
                };
                out.push_str(&format!("{flags} {name:<NAME_W$} {value:<VALUE_W$}\n"));
            }
        } else {
            let resolved = resolve(chain, self.preselect.as_deref());
            // 2-line header matching render_flat.
            out.push_str(&format!(
                "{:<FLAG_W$} {:<NAME_W$} {:<VALUE_W$} {:<FROM_W$}\n",
                "flags", "name", "value", "from"
            ));
            out.push('\n'); // separator placeholder
            for row in &resolved {
                // Combined flag chip in canonical order (S, P, O, !).
                let mut letters = String::new();
                if row.winner.sensitive {
                    letters.push('S');
                }
                if row.winner.provided {
                    letters.push('P');
                }
                if !row.shadowed.is_empty() && !row.unresolved {
                    letters.push('O');
                }
                if row.unresolved {
                    letters.push('!');
                }
                let mut flags = if letters.is_empty() {
                    String::new()
                } else {
                    format!("[{letters}]")
                };
                while flags.chars().count() < FLAG_W {
                    flags.push(' ');
                }

                let name = truncate_for_body(&row.winner.name, NAME_W);
                let value = if row.winner.sensitive {
                    truncate_for_body("(sensitive)", VALUE_W)
                } else {
                    truncate_for_body(row.winner.value.as_deref().unwrap_or("\u{2014}"), VALUE_W)
                };
                let from = if row.unresolved {
                    truncate_for_body("\u{2014}", FROM_W)
                } else {
                    truncate_for_body(&row.winner_context, FROM_W)
                };
                out.push_str(&format!(
                    "{flags} {name:<NAME_W$} {value:<VALUE_W$} {from:<FROM_W$}\n"
                ));
                if self.show_shadowed {
                    for (shadowed_entry, shadowed_ctx) in &row.shadowed {
                        let blank_flags = " ".repeat(FLAG_W);
                        let sname = truncate_for_body(&shadowed_entry.name, NAME_W - 2);
                        let svalue = if shadowed_entry.sensitive {
                            truncate_for_body("(sensitive)", VALUE_W)
                        } else {
                            truncate_for_body(
                                shadowed_entry.value.as_deref().unwrap_or("\u{2014}"),
                                VALUE_W,
                            )
                        };
                        let sfrom = truncate_for_body(shadowed_ctx, FROM_W);
                        out.push_str(&format!(
                            "{blank_flags}   {sname:<width$} {svalue:<VALUE_W$} {sfrom:<FROM_W$}\n",
                            width = NAME_W - 2
                        ));
                    }
                }
            }
        }
        out
    }
}

/// Truncate `s` to at most `max_chars` characters (char-count), appending
/// `…` if cut. Mirrors the render-side `truncate_str` so that the byte
/// offsets in `searchable_body` align with what `apply_search_highlights`
/// reconstructs from the rendered spans.
fn truncate_for_body(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{truncated}…")
    }
}

/// One row in the resolved-flat view. `shadowed` carries every
/// same-name param hidden by `winner` further down the chain.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedParameter {
    pub winner: ParameterEntry,
    pub winner_context: String,
    pub shadowed: Vec<(ParameterEntry, String)>,
    /// Set only when the modal was opened with a `preselect` and the
    /// preselected name does not appear anywhere in the chain.
    pub unresolved: bool,
}

/// Walk `chain` in resolution order (first-listed wins) and return
/// the resolved-flat list, sorted by name ascending. If
/// `preselect_unresolved` is `Some(name)`, prepend a synthetic
/// unresolved row for that name.
pub fn resolve(
    chain: &[ParameterContextNode],
    preselect_unresolved: Option<&str>,
) -> Vec<ResolvedParameter> {
    use std::collections::BTreeMap;

    let mut by_name: BTreeMap<String, ResolvedParameter> = BTreeMap::new();
    for node in chain {
        for entry in &node.parameters {
            match by_name.get_mut(&entry.name) {
                None => {
                    by_name.insert(
                        entry.name.clone(),
                        ResolvedParameter {
                            winner: entry.clone(),
                            winner_context: node.name.clone(),
                            shadowed: vec![],
                            unresolved: false,
                        },
                    );
                }
                Some(rp) => {
                    rp.shadowed.push((entry.clone(), node.name.clone()));
                }
            }
        }
    }

    let mut out: Vec<ResolvedParameter> = by_name.into_values().collect();

    if let Some(name) = preselect_unresolved
        && !out.iter().any(|r| r.winner.name == name)
    {
        out.insert(
            0,
            ResolvedParameter {
                winner: ParameterEntry {
                    name: name.into(),
                    value: None,
                    description: None,
                    sensitive: false,
                    provided: false,
                },
                winner_context: "—".into(),
                shadowed: vec![],
                unresolved: true,
            },
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, value: &str) -> ParameterEntry {
        ParameterEntry {
            name: name.into(),
            value: Some(value.into()),
            description: None,
            sensitive: false,
            provided: false,
        }
    }

    fn node(id: &str, name: &str, params: Vec<ParameterEntry>) -> ParameterContextNode {
        ParameterContextNode {
            id: id.into(),
            name: name.into(),
            parameters: params,
            inherited_ids: vec![],
            fetch_error: None,
        }
    }

    #[test]
    fn resolve_first_listed_wins_and_records_shadowed() {
        let chain = vec![
            node(
                "ctx-prod",
                "prod",
                vec![entry("retry", "5"), entry("only_in_prod", "p")],
            ),
            node(
                "ctx-base",
                "base",
                vec![entry("retry", "3"), entry("only_in_base", "b")],
            ),
        ];
        let resolved = resolve(&chain, None);
        let retry = resolved.iter().find(|r| r.winner.name == "retry").unwrap();
        assert_eq!(retry.winner.value.as_deref(), Some("5"));
        assert_eq!(retry.winner_context, "prod");
        assert_eq!(retry.shadowed.len(), 1);
        assert_eq!(retry.shadowed[0].1, "base");
    }

    #[test]
    fn resolve_synthesises_unresolved_row() {
        let chain = vec![node("ctx", "ctx", vec![entry("foo", "1")])];
        let resolved = resolve(&chain, Some("missing"));
        assert!(resolved[0].unresolved);
        assert_eq!(resolved[0].winner.name, "missing");
        assert_eq!(resolved.iter().filter(|r| r.unresolved).count(), 1);
    }

    #[test]
    fn resolve_no_synthetic_when_preselect_resolves() {
        let chain = vec![node("ctx", "ctx", vec![entry("foo", "1")])];
        let resolved = resolve(&chain, Some("foo"));
        assert!(resolved.iter().all(|r| !r.unresolved));
    }
}
