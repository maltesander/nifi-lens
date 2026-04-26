//! Detects NiFi parameter references (`#{name}`) in property values.
//! Honours the `##` escape: `##{foo}` is a literal `#{foo}`, not a
//! reference. Used by the Browser detail-pane renderer to gate the
//! `→` cross-link annotation on processor / CS property rows.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamRefScan {
    None,
    Single { name: String },
    Multiple,
}

pub fn scan(value: &str) -> ParamRefScan {
    let bytes = value.as_bytes();
    let mut i = 0;
    let mut found: Option<String> = None;
    while i < bytes.len() {
        if bytes[i] == b'#' {
            // Look for `##` escape: skip both bytes, no match.
            if i + 1 < bytes.len() && bytes[i + 1] == b'#' {
                i += 2;
                continue;
            }
            // Look for `#{name}`.
            if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                let start = i + 2;
                if let Some(end_rel) = bytes[start..].iter().position(|&b| b == b'}') {
                    let end = start + end_rel;
                    if end > start {
                        let name = match std::str::from_utf8(&bytes[start..end]) {
                            Ok(s) => s.to_string(),
                            Err(_) => {
                                i = end + 1;
                                continue;
                            }
                        };
                        if found.is_some() {
                            return ParamRefScan::Multiple;
                        }
                        found = Some(name);
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    match found {
        Some(name) => ParamRefScan::Single { name },
        None => ParamRefScan::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_refs() {
        assert_eq!(scan("hello world"), ParamRefScan::None);
        assert_eq!(scan(""), ParamRefScan::None);
    }

    #[test]
    fn single_ref() {
        assert_eq!(scan("#{foo}"), ParamRefScan::Single { name: "foo".into() });
        assert_eq!(
            scan("hello #{foo} world"),
            ParamRefScan::Single { name: "foo".into() }
        );
    }

    #[test]
    fn escaped_ref_is_literal() {
        assert_eq!(scan("##{foo}"), ParamRefScan::None);
        assert_eq!(scan("prefix ##{foo} suffix"), ParamRefScan::None);
    }

    #[test]
    fn one_escaped_one_real() {
        assert_eq!(
            scan("##{a} and #{b}"),
            ParamRefScan::Single { name: "b".into() }
        );
    }

    #[test]
    fn multiple_real_refs() {
        assert_eq!(scan("#{a} and #{b}"), ParamRefScan::Multiple);
        assert_eq!(scan("#{a}#{b}#{c}"), ParamRefScan::Multiple);
    }

    #[test]
    fn empty_name_is_not_a_ref() {
        assert_eq!(scan("#{}"), ParamRefScan::None);
    }

    #[test]
    fn unterminated_is_not_a_ref() {
        assert_eq!(scan("#{foo"), ParamRefScan::None);
        assert_eq!(scan("hello #{foo and that's it"), ParamRefScan::None);
    }
}
