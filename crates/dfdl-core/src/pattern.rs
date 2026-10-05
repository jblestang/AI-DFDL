//! DFDL regular-expression patterns with trailing look-ahead support.
//!
//! DFDL 1.0 (§12.3.2 `lengthPattern`, §7.3.1 `testPattern`, §9.4.1 discriminators)
//! specifies ICU/Java-style regular expressions. Real-world schemas use
//! zero-width look-ahead, e.g. `.*?(?=END)`, to describe "the data up to, but
//! not including, a terminator". The `regex` crate (linear time, no
//! backtracking) rejects look-around, so we rewrite positive look-ahead
//! `(?=X)` into an empty capture group followed by `X`:
//!
//! ```text
//!   P(?=X)   ==>   P()(?:X)
//! ```
//!
//! The regex crate uses leftmost-first (Perl-like) semantics, so the overall
//! match is identical to the backtracking engine's. The *consumed* length is
//! the start offset of the first empty group that participated in the match
//! (the look-ahead boundary) instead of the overall match end.
//!
//! Limitation: a look-ahead must be the last consuming construct of its
//! alternative (the universal use in DFDL schemas). Negative look-ahead and
//! look-behind are rejected.

extern crate alloc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// A compiled DFDL pattern (regex with optional trailing look-ahead).
#[derive(Debug, Clone)]
pub struct DfdlRegex {
    re: regex::Regex,
    /// Capture-group indices of the rewritten (empty) look-ahead markers.
    lookahead_groups: Vec<usize>,
}

/// Rewrites positive look-ahead groups; returns the new pattern and the capture
/// indices of the inserted markers, or an error message for unsupported syntax.
#[allow(clippy::arithmetic_side_effects)]
fn rewrite_lookahead(pat: &str) -> Result<(String, Vec<usize>), String> {
    let chars: Vec<char> = pat.chars().collect();
    let mut out = String::with_capacity(pat.len() + 8);
    let mut groups = Vec::new();
    let mut capture_count = 0usize;
    let mut in_class = false;
    let mut i = 0;
    while i < chars.len() {
        let Some(&c) = chars.get(i) else { break; };
        let peek = |idx: usize| chars.get(idx).copied();
        if c == '\\' {
            if peek(i + 1) == Some('U') {
                return Err("escape \\U is not a valid regular expression escape in Java/DFDL regex".to_string());
            }
            out.push(c);
            if let Some(n) = peek(i + 1) {
                out.push(n);
            }
            i += 2;
            continue;
        }
        if in_class {
            if c == ']' {
                in_class = false;
            }
            out.push(c);
            i += 1;
            continue;
        }
        match c {
            '[' => {
                in_class = true;
                out.push(c);
                i += 1;
                // A leading `^` and/or `]` are literal members of the class.
                if peek(i) == Some('^') {
                    out.push('^');
                    i += 1;
                }
                if peek(i) == Some(']') {
                    out.push(']');
                    i += 1;
                }
            }
            '(' => {
                if peek(i + 1) == Some('?') {
                    match (peek(i + 2), peek(i + 3)) {
                        (Some('='), _) => {
                            capture_count += 1;
                            groups.push(capture_count);
                            out.push_str("()(?:");
                            i += 3;
                            continue;
                        }
                        (Some('!'), _) => return Err("negative look-ahead is not supported".to_string()),
                        (Some('<'), Some('=')) | (Some('<'), Some('!')) => {
                            return Err("look-behind is not supported".to_string())
                        }
                        (Some('P'), Some('<')) | (Some('<'), _) => capture_count += 1,
                        _ => {}
                    }
                } else {
                    capture_count += 1;
                }
                out.push(c);
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    Ok((out, groups))
}

impl DfdlRegex {
    /// Compiles `pat`, rewriting supported look-ahead. Errors carry a short reason.
    pub fn new(pat: &str) -> Result<Self, String> {
        let (rewritten, lookahead_groups) = rewrite_lookahead(pat)?;
        let re = regex::Regex::new(&rewritten).map_err(|e| e.to_string())?;
        Ok(Self {
            re,
            lookahead_groups,
        })
    }

    /// Unanchored test: does the pattern match anywhere in `text`?
    #[must_use]
    pub fn is_match(&self, text: &str) -> bool {
        self.re.is_match(text)
    }

    /// Anchored match at offset 0. Returns the number of bytes *consumed*
    /// (excluding any look-ahead), or `None` if there is no match at offset 0.
    #[must_use]
    pub fn match_prefix_len(&self, text: &str) -> Option<usize> {
        let caps = self.re.captures(text)?;
        let whole = caps.get(0)?;
        if whole.start() != 0 {
            return None;
        }
        let end = self
            .lookahead_groups
            .iter()
            .find_map(|&g| caps.get(g))
            .map_or(whole.end(), |m| m.start());
        Some(end)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn plain_pattern_consumes_whole_match() {
        let re = DfdlRegex::new("[a-c]+").unwrap();
        assert_eq!(re.match_prefix_len("abcd"), Some(3));
        assert_eq!(re.match_prefix_len("xabc"), None);
    }

    #[test]
    fn lazy_lookahead_stops_before_terminator() {
        let re = DfdlRegex::new(".*?(?=END)").unwrap();
        assert_eq!(re.match_prefix_len("abcENDxyz"), Some(3));
        assert_eq!(re.match_prefix_len("ENDxyz"), Some(0));
        assert_eq!(re.match_prefix_len("no terminator"), None);
    }

    #[test]
    fn lookahead_with_dollar_alternative() {
        let re = DfdlRegex::new(r".*?[^\\](?=,|$)").unwrap();
        assert_eq!(re.match_prefix_len("ab,cd"), Some(2));
        assert_eq!(re.match_prefix_len("abcd"), Some(4));
    }

    #[test]
    fn top_level_alternation_with_lookahead_in_one_branch() {
        let re = DfdlRegex::new("[^END]{0,9}(?=END)|.{10}").unwrap();
        assert_eq!(re.match_prefix_len("abcEND"), Some(3));
        assert_eq!(re.match_prefix_len("0123456789ab"), Some(10));
    }

    #[test]
    fn lookahead_marker_inside_class_or_escape_is_literal() {
        let re = DfdlRegex::new(r"[(?=]+\(").unwrap();
        assert_eq!(re.match_prefix_len("(?=("), Some(4));
    }

    #[test]
    fn capture_groups_before_lookahead_do_not_shift_marker() {
        let re = DfdlRegex::new(r"(\d)(\d)(?=\D+)").unwrap();
        assert_eq!(re.match_prefix_len("12ab"), Some(2));
        let re = DfdlRegex::new(r"(?P<n>\d\d)(?=\D+)").unwrap();
        assert_eq!(re.match_prefix_len("12ab"), Some(2));
    }

    #[test]
    fn unsupported_lookaround_is_rejected() {
        assert!(DfdlRegex::new("a(?!b)").is_err());
        assert!(DfdlRegex::new("(?<=a)b").is_err());
        assert!(DfdlRegex::new("(unbalanced").is_err());
    }

    #[test]
    fn is_match_is_unanchored() {
        let re = DfdlRegex::new("b(?=c)").unwrap();
        assert!(re.is_match("abc"));
        assert!(!re.is_match("abd"));
    }
}
