//! Attribute-matching predicate language for the Events watch sub-mode.
//!
//! See `docs/superpowers/specs/2026-05-01-attribute-watcher-design.md`
//! for grammar. Parser is hand-rolled, no `nom` / `serde` dependency.

use regex::Regex;
use snafu::Snafu;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    RegexMatch,
    RegexNotMatch,
}

#[derive(Debug, Clone)]
pub enum ClauseLiteral {
    Plain(String),
    Regex(Regex),
}

#[derive(Debug, Clone)]
pub struct Clause {
    pub attribute: String,
    pub op: Op,
    pub literal: ClauseLiteral,
}

#[derive(Debug, Clone, Default)]
pub struct Predicate {
    clauses: Vec<Clause>,
}

#[derive(Debug, Clone, PartialEq, Eq, Snafu)]
#[snafu(display("predicate parse error at column {column}: {message}"))]
pub struct PredicateParseError {
    pub message: String,
    pub column: usize,
}

impl Predicate {
    pub fn parse(input: &str) -> Result<Self, PredicateParseError> {
        let bytes = input.as_bytes();
        let mut cur = 0usize;
        let mut clauses = Vec::new();

        skip_ws(bytes, &mut cur);
        if cur >= bytes.len() {
            return Ok(Predicate { clauses });
        }

        loop {
            let attribute = parse_attr(bytes, &mut cur)?;
            skip_ws(bytes, &mut cur);
            let op = parse_op(bytes, &mut cur)?;
            skip_ws(bytes, &mut cur);
            let literal = parse_literal(bytes, &mut cur, op)?;
            clauses.push(Clause {
                attribute,
                op,
                literal,
            });

            skip_ws(bytes, &mut cur);
            if cur >= bytes.len() {
                break;
            }
            if !consume_keyword(bytes, &mut cur, b"AND") {
                return Err(PredicateParseError {
                    message: format!(
                        "expected 'AND' or end of input, got {:?}",
                        next_token_excerpt(bytes, cur)
                    ),
                    column: cur + 1,
                });
            }
            skip_ws(bytes, &mut cur);
        }

        Ok(Predicate { clauses })
    }

    pub fn is_empty(&self) -> bool {
        self.clauses.is_empty()
    }

    pub fn clauses(&self) -> &[Clause] {
        &self.clauses
    }

    /// Render the predicate as a string for logging, with all literal
    /// values replaced by the placeholder `<value>`. Attribute names
    /// and operators are kept verbatim. Used by `tracing::info!` calls
    /// in the watch worker so user-supplied values never leak into log
    /// files.
    pub fn redacted(&self) -> String {
        let mut out = String::new();
        for (i, c) in self.clauses.iter().enumerate() {
            if i > 0 {
                out.push_str(" AND ");
            }
            out.push_str(&c.attribute);
            out.push(' ');
            out.push_str(match c.op {
                Op::Eq => "=",
                Op::Ne => "!=",
                Op::RegexMatch => "=~",
                Op::RegexNotMatch => "!~",
            });
            out.push_str(" <value>");
        }
        out
    }

    /// Evaluate the predicate against an event's attribute list.
    /// Empty predicate matches anything. Missing attributes:
    /// `=`/`=~` -> false, `!=`/`!~` -> true.
    pub fn matches(&self, attrs: &[crate::client::AttributeTriple]) -> bool {
        self.clauses.iter().all(|c| clause_matches(c, attrs))
    }
}

fn clause_matches(clause: &Clause, attrs: &[crate::client::AttributeTriple]) -> bool {
    let value: Option<&str> = attrs
        .iter()
        .find(|a| a.key == clause.attribute)
        .and_then(|a| a.current.as_deref());
    match (&clause.literal, clause.op, value) {
        (ClauseLiteral::Plain(rhs), Op::Eq, Some(v)) => v == rhs.as_str(),
        (ClauseLiteral::Plain(_), Op::Eq, None) => false,
        (ClauseLiteral::Plain(rhs), Op::Ne, Some(v)) => v != rhs.as_str(),
        (ClauseLiteral::Plain(_), Op::Ne, None) => true,
        (ClauseLiteral::Regex(re), Op::RegexMatch, Some(v)) => re.is_match(v),
        (ClauseLiteral::Regex(_), Op::RegexMatch, None) => false,
        (ClauseLiteral::Regex(re), Op::RegexNotMatch, Some(v)) => !re.is_match(v),
        (ClauseLiteral::Regex(_), Op::RegexNotMatch, None) => true,
        // Mismatched literal/op combinations cannot occur because
        // parse_literal pairs them. Defensive fallback uses missing-semantics.
        (_, Op::Eq | Op::RegexMatch, _) => false,
        (_, Op::Ne | Op::RegexNotMatch, _) => true,
    }
}

fn skip_ws(bytes: &[u8], cur: &mut usize) {
    while *cur < bytes.len() && (bytes[*cur] == b' ' || bytes[*cur] == b'\t') {
        *cur += 1;
    }
}

fn parse_attr(bytes: &[u8], cur: &mut usize) -> Result<String, PredicateParseError> {
    let start = *cur;
    if *cur >= bytes.len() {
        return Err(PredicateParseError {
            message: "expected attribute name".to_string(),
            column: *cur + 1,
        });
    }
    let first = bytes[*cur];
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return Err(PredicateParseError {
            message: format!(
                "attribute must start with letter or '_', got {:?}",
                first as char
            ),
            column: *cur + 1,
        });
    }
    while *cur < bytes.len() {
        let b = bytes[*cur];
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-' {
            *cur += 1;
        } else {
            break;
        }
    }
    Ok(std::str::from_utf8(&bytes[start..*cur])
        .map_err(|_| PredicateParseError {
            message: "attribute name not valid UTF-8".to_string(),
            column: start + 1,
        })?
        .to_string())
}

fn parse_op(bytes: &[u8], cur: &mut usize) -> Result<Op, PredicateParseError> {
    if *cur >= bytes.len() {
        return Err(PredicateParseError {
            message: "expected operator".to_string(),
            column: *cur + 1,
        });
    }
    let rest = &bytes[*cur..];
    if rest.starts_with(b"=~") {
        *cur += 2;
        return Ok(Op::RegexMatch);
    }
    if rest.starts_with(b"!~") {
        *cur += 2;
        return Ok(Op::RegexNotMatch);
    }
    if rest.starts_with(b"!=") {
        *cur += 2;
        return Ok(Op::Ne);
    }
    if rest.starts_with(b"=") {
        *cur += 1;
        return Ok(Op::Eq);
    }
    Err(PredicateParseError {
        message: format!(
            "expected operator (= != =~ !~), got {:?}",
            next_token_excerpt(bytes, *cur)
        ),
        column: *cur + 1,
    })
}

fn parse_literal(
    bytes: &[u8],
    cur: &mut usize,
    op: Op,
) -> Result<ClauseLiteral, PredicateParseError> {
    if matches!(op, Op::RegexMatch | Op::RegexNotMatch) {
        let (pattern, start_col) = parse_regex_literal(bytes, cur)?;
        let regex = Regex::new(&pattern).map_err(|e| PredicateParseError {
            message: format!("invalid regex: {e}"),
            column: start_col + 1,
        })?;
        Ok(ClauseLiteral::Regex(regex))
    } else if *cur < bytes.len() && bytes[*cur] == b'"' {
        Ok(ClauseLiteral::Plain(parse_quoted(bytes, cur)?))
    } else {
        Ok(ClauseLiteral::Plain(parse_bare(bytes, cur)?))
    }
}

fn parse_regex_literal(
    bytes: &[u8],
    cur: &mut usize,
) -> Result<(String, usize), PredicateParseError> {
    if *cur >= bytes.len() || bytes[*cur] != b'/' {
        return Err(PredicateParseError {
            message: "expected '/' starting regex literal".to_string(),
            column: *cur + 1,
        });
    }
    let start = *cur;
    *cur += 1;
    let mut out: Vec<u8> = Vec::new();
    while *cur < bytes.len() {
        match bytes[*cur] {
            b'\\' if *cur + 1 < bytes.len() && bytes[*cur + 1] == b'/' => {
                out.push(b'/');
                *cur += 2;
            }
            b'/' => {
                *cur += 1;
                let pattern = String::from_utf8(out).map_err(|_| PredicateParseError {
                    message: "regex literal not valid UTF-8".to_string(),
                    column: start + 1,
                })?;
                return Ok((pattern, start));
            }
            b => {
                out.push(b);
                *cur += 1;
            }
        }
    }
    Err(PredicateParseError {
        message: "unterminated regex literal".to_string(),
        column: start + 1,
    })
}

fn parse_quoted(bytes: &[u8], cur: &mut usize) -> Result<String, PredicateParseError> {
    let start = *cur;
    *cur += 1;
    let mut out: Vec<u8> = Vec::new();
    while *cur < bytes.len() {
        match bytes[*cur] {
            b'\\' if *cur + 1 < bytes.len() && bytes[*cur + 1] == b'"' => {
                out.push(b'"');
                *cur += 2;
            }
            b'"' => {
                *cur += 1;
                return String::from_utf8(out).map_err(|_| PredicateParseError {
                    message: "quoted string not valid UTF-8".to_string(),
                    column: start + 1,
                });
            }
            b => {
                out.push(b);
                *cur += 1;
            }
        }
    }
    Err(PredicateParseError {
        message: "unterminated quoted string".to_string(),
        column: start + 1,
    })
}

fn parse_bare(bytes: &[u8], cur: &mut usize) -> Result<String, PredicateParseError> {
    let start = *cur;
    while *cur < bytes.len() && bytes[*cur] != b' ' && bytes[*cur] != b'\t' {
        *cur += 1;
    }
    if *cur == start {
        return Err(PredicateParseError {
            message: "expected literal value".to_string(),
            column: *cur + 1,
        });
    }
    Ok(std::str::from_utf8(&bytes[start..*cur])
        .map_err(|_| PredicateParseError {
            message: "literal not valid UTF-8".to_string(),
            column: start + 1,
        })?
        .to_string())
}

fn consume_keyword(bytes: &[u8], cur: &mut usize, kw: &[u8]) -> bool {
    if bytes[*cur..].starts_with(kw) {
        let end = *cur + kw.len();
        let after_ok = end >= bytes.len() || bytes[end] == b' ' || bytes[end] == b'\t';
        if after_ok {
            *cur = end;
            return true;
        }
    }
    false
}

fn next_token_excerpt(bytes: &[u8], cur: usize) -> String {
    let end = (cur + 8).min(bytes.len());
    String::from_utf8_lossy(&bytes[cur..end]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::AttributeTriple;

    fn parse(s: &str) -> Predicate {
        Predicate::parse(s).expect("valid predicate")
    }

    fn attr(key: &str, current: Option<&str>) -> AttributeTriple {
        AttributeTriple {
            key: key.to_string(),
            previous: None,
            current: current.map(str::to_string),
        }
    }

    #[test]
    fn empty_input_is_empty_predicate() {
        let p = parse("");
        assert!(p.is_empty());
        assert_eq!(p.clauses().len(), 0);
    }

    #[test]
    fn whitespace_only_is_empty_predicate() {
        let p = parse("   \t  ");
        assert!(p.is_empty());
    }

    #[test]
    fn single_eq_clause() {
        let p = parse("filename = invoice.json");
        assert_eq!(p.clauses().len(), 1);
        assert_eq!(p.clauses()[0].attribute, "filename");
        assert_eq!(p.clauses()[0].op, Op::Eq);
        match &p.clauses()[0].literal {
            ClauseLiteral::Plain(v) => assert_eq!(v, "invoice.json"),
            _ => panic!("expected Plain"),
        }
    }

    #[test]
    fn single_ne_clause() {
        let p = parse("kafka.topic != orders.dlq");
        assert_eq!(p.clauses()[0].op, Op::Ne);
    }

    #[test]
    fn single_regex_match_clause() {
        let p = parse("filename =~ /^invoice-/");
        assert_eq!(p.clauses()[0].op, Op::RegexMatch);
        match &p.clauses()[0].literal {
            ClauseLiteral::Regex(r) => assert!(r.is_match("invoice-204.json")),
            _ => panic!("expected Regex"),
        }
    }

    #[test]
    fn single_regex_not_match_clause() {
        let p = parse("filename !~ /\\.tmp$/");
        assert_eq!(p.clauses()[0].op, Op::RegexNotMatch);
    }

    #[test]
    fn anded_clauses() {
        let p = parse("filename =~ /^invoice-/ AND mime.type = application/json");
        assert_eq!(p.clauses().len(), 2);
        assert_eq!(p.clauses()[0].attribute, "filename");
        assert_eq!(p.clauses()[1].attribute, "mime.type");
    }

    #[test]
    fn three_anded_clauses() {
        let p = parse(
            "kafka.topic != orders.dlq AND filename =~ /\\.json$/ AND mime.type = application/json",
        );
        assert_eq!(p.clauses().len(), 3);
    }

    #[test]
    fn quoted_bare_string_with_spaces() {
        let p = parse("description = \"hello world\"");
        match &p.clauses()[0].literal {
            ClauseLiteral::Plain(v) => assert_eq!(v, "hello world"),
            _ => panic!(),
        }
    }

    #[test]
    fn regex_literal_with_escaped_slash() {
        let p = parse("path =~ /a\\/b/");
        match &p.clauses()[0].literal {
            ClauseLiteral::Regex(r) => assert!(r.is_match("a/b")),
            _ => panic!(),
        }
    }

    #[test]
    fn attribute_with_dots_and_dashes() {
        let p = parse("kafka.consumer-group = svc-1");
        assert_eq!(p.clauses()[0].attribute, "kafka.consumer-group");
    }

    #[test]
    fn regex_literal_preserves_non_ascii() {
        let p = parse("filename =~ /naïve/");
        match &p.clauses()[0].literal {
            ClauseLiteral::Regex(r) => assert!(r.is_match("naïve")),
            _ => panic!("expected Regex"),
        }
    }

    #[test]
    fn quoted_literal_preserves_non_ascii() {
        let p = parse("description = \"héllo wörld\"");
        match &p.clauses()[0].literal {
            ClauseLiteral::Plain(v) => assert_eq!(v, "héllo wörld"),
            _ => panic!("expected Plain"),
        }
    }

    #[test]
    fn invalid_regex_reports_correct_column() {
        // Closed-but-malformed regex: `[` opens a class that never closes inside the /.../
        let err = Predicate::parse("filename =~ /[/").unwrap_err();
        // The opening / is at 0-based byte 12, so 1-based column is 13.
        assert!(
            err.message.contains("invalid regex"),
            "got message: {}",
            err.message
        );
        assert_eq!(err.column, 13);
    }

    #[test]
    fn empty_predicate_matches_anything() {
        let p = parse("");
        assert!(p.matches(&[]));
        assert!(p.matches(&[attr("filename", Some("x"))]));
    }

    #[test]
    fn eq_matches_equal_value() {
        let p = parse("filename = invoice.json");
        assert!(p.matches(&[attr("filename", Some("invoice.json"))]));
        assert!(!p.matches(&[attr("filename", Some("other.json"))]));
    }

    #[test]
    fn eq_missing_attr_does_not_match() {
        let p = parse("filename = invoice.json");
        assert!(!p.matches(&[]));
        assert!(!p.matches(&[attr("other", Some("x"))]));
    }

    #[test]
    fn ne_missing_attr_matches() {
        let p = parse("filename != bad");
        assert!(p.matches(&[]));
        assert!(p.matches(&[attr("filename", Some("good"))]));
        assert!(!p.matches(&[attr("filename", Some("bad"))]));
    }

    #[test]
    fn regex_match_missing_attr_does_not_match() {
        let p = parse("filename =~ /^invoice-/");
        assert!(!p.matches(&[]));
        assert!(p.matches(&[attr("filename", Some("invoice-1.json"))]));
        assert!(!p.matches(&[attr("filename", Some("other.json"))]));
    }

    #[test]
    fn regex_not_match_missing_attr_matches() {
        let p = parse("filename !~ /^bad-/");
        assert!(p.matches(&[]));
        assert!(p.matches(&[attr("filename", Some("good"))]));
        assert!(!p.matches(&[attr("filename", Some("bad-x"))]));
    }

    #[test]
    fn and_short_circuits_on_first_false() {
        let p = parse("filename =~ /^invoice-/ AND mime.type = application/json");
        assert!(!p.matches(&[attr("filename", Some("nope"))]));
        assert!(p.matches(&[
            attr("filename", Some("invoice-1.json")),
            attr("mime.type", Some("application/json")),
        ]));
    }

    #[test]
    fn null_current_value_treated_as_missing() {
        let p = parse("filename = x");
        assert!(!p.matches(&[attr("filename", None)]));
    }

    #[test]
    fn parse_error_unknown_operator() {
        let err = Predicate::parse("filename matches /foo/").unwrap_err();
        assert!(err.message.contains("operator"), "got: {}", err.message);
        assert_eq!(err.column, 10); // position of 'm' in "matches"
    }

    #[test]
    fn parse_error_unterminated_regex() {
        let err = Predicate::parse("filename =~ /unterm").unwrap_err();
        assert!(
            err.message.contains("unterminated regex"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn parse_error_unterminated_quoted() {
        let err = Predicate::parse("description = \"oops").unwrap_err();
        assert!(
            err.message.contains("unterminated quoted"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn parse_error_invalid_regex() {
        let err = Predicate::parse("filename =~ /[unclosed/").unwrap_err();
        assert!(
            err.message.contains("invalid regex"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn parse_error_empty_attribute() {
        let err = Predicate::parse("= x").unwrap_err();
        assert!(err.message.contains("attribute"), "got: {}", err.message);
    }

    #[test]
    fn parse_error_missing_and_between_clauses() {
        let err = Predicate::parse("a = 1 b = 2").unwrap_err();
        assert!(err.message.contains("AND"), "got: {}", err.message);
    }

    #[test]
    fn redacted_replaces_all_literal_values() {
        let p = parse("filename =~ /^invoice-/ AND mime.type = application/json");
        let r = p.redacted();
        assert_eq!(r, "filename =~ <value> AND mime.type = <value>");
    }

    #[test]
    fn redacted_empty_predicate() {
        let p = parse("");
        assert_eq!(p.redacted(), "");
    }
}
