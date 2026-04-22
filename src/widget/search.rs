//! Shared primitives for wrap-aware substring search inside scrollable
//! bodies. Extracted from the Bulletins detail modal so the Tracer
//! content viewer modal can reuse them verbatim.

/// A substring match in a pre-wrap body. Byte offsets into the specific
/// line (split on `\n`); `line_idx` is the pre-wrap line index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchSpan {
    pub line_idx: usize,
    pub byte_start: usize,
    pub byte_end: usize,
}

/// Transient state of an in-progress or committed search.
#[derive(Debug, Clone, Default)]
pub struct SearchState {
    pub query: String,
    /// True while the user is still typing the query (`/` just pressed,
    /// `Enter` not yet). Consumes keystrokes as literal text.
    pub input_active: bool,
    /// True once the user pressed `Enter`. `n`/`N` only work in this
    /// state.
    pub committed: bool,
    /// Matches in pre-wrap coordinates. Recomputed on every query edit.
    pub matches: Vec<MatchSpan>,
    /// Index into `matches`; `None` when `matches` is empty.
    pub current: Option<usize>,
}

/// Case-insensitive, non-overlapping substring search. Returns matches
/// in pre-wrap coordinates. Input is split on `\n`; each line is
/// searched independently. Empty `query` returns an empty vec.
pub fn compute_matches(body: &str, query: &str) -> Vec<MatchSpan> {
    if query.is_empty() {
        return Vec::new();
    }
    let lq = query.to_ascii_lowercase();
    let mut out = Vec::new();
    for (line_idx, line) in body.split('\n').enumerate() {
        let ll = line.to_ascii_lowercase();
        let mut from = 0;
        while let Some(rel) = ll[from..].find(&lq) {
            let start = from + rel;
            let end = start + lq.len();
            out.push(MatchSpan {
                line_idx,
                byte_start: start,
                byte_end: end,
            });
            from = end;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_returns_no_matches() {
        assert!(compute_matches("anything at all", "").is_empty());
    }

    #[test]
    fn single_line_single_match() {
        let matches = compute_matches("hello world", "world");
        assert_eq!(
            matches,
            vec![MatchSpan {
                line_idx: 0,
                byte_start: 6,
                byte_end: 11
            }]
        );
    }

    #[test]
    fn multiple_matches_across_lines() {
        let body = "foo bar\nbar baz\nqux";
        let matches = compute_matches(body, "bar");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].line_idx, 0);
        assert_eq!(matches[1].line_idx, 1);
    }

    #[test]
    fn overlapping_matches_advance_past_end() {
        // "aaa" search in "aaaa" should yield only one match at 0-3
        // (non-overlapping).
        let matches = compute_matches("aaaa", "aaa");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].byte_start, 0);
        assert_eq!(matches[0].byte_end, 3);
    }

    #[test]
    fn case_insensitive_match() {
        let body = "Error: connection refused\nRetrying ERROR\nerror happens";
        let m = compute_matches(body, "error");
        assert_eq!(m.len(), 3);
        assert_eq!(m[0].line_idx, 0);
        assert_eq!(m[0].byte_start, 0);
        assert_eq!(m[1].line_idx, 1);
        assert_eq!(m[2].line_idx, 2);
    }

    #[test]
    fn non_overlapping_consecutive_matches() {
        // "aaaa" searching "aa" should return two non-overlapping matches.
        let m = compute_matches("aaaa", "aa");
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].byte_start, 0);
        assert_eq!(m[0].byte_end, 2);
        assert_eq!(m[1].byte_start, 2);
        assert_eq!(m[1].byte_end, 4);
    }
}
