//! State for the Browser tab's parameter-context modal.
//! Mirrors `version_control_modal` in shape: per-open struct on
//! `BrowserState.parameter_modal: Option<...>`, populated by reducers
//! when the worker resolves the chain.

use crate::client::parameter_context::{ParameterContextNode, ParameterEntry};
use crate::widget::scroll::VerticalScrollState;
use crate::widget::search::SearchState;

#[derive(Debug, Clone)]
pub struct ParameterContextModalState {
    /// PG that triggered the open. Stays fixed for the modal session.
    pub originating_pg_id: String,
    pub originating_pg_path: String,
    /// Optional pre-selected param name from a `#{name}` cross-link.
    pub preselect: Option<String>,
    pub load: ParameterContextLoad,
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
    /// Each row is `"{name}={value}\n"` (or `"{name}=(sensitive)\n"` for
    /// sensitive params). The output format MUST match the rendered
    /// parameter list line-for-line so `MatchSpan` byte offsets align
    /// with what the render pass displays.
    ///
    /// In `by_context_mode` the body is scoped to the context at
    /// `sidebar_index`; in flat mode (default) the resolved list is used.
    pub fn searchable_body(&self) -> String {
        let chain = match &self.load {
            ParameterContextLoad::Loaded { chain } => chain,
            _ => return String::new(),
        };
        let mut out = String::new();
        if self.by_context_mode {
            if let Some(ctx) = chain.get(self.sidebar_index) {
                for entry in &ctx.parameters {
                    let value = if entry.sensitive {
                        "(sensitive)".to_string()
                    } else {
                        entry.value.clone().unwrap_or_default()
                    };
                    out.push_str(&format!("{}={}\n", entry.name, value));
                }
            }
        } else {
            let resolved = resolve(chain, self.preselect.as_deref());
            for row in &resolved {
                let value = if row.winner.sensitive {
                    "(sensitive)".to_string()
                } else {
                    row.winner.value.clone().unwrap_or_default()
                };
                out.push_str(&format!("{}={}\n", row.winner.name, value));
            }
        }
        out
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
