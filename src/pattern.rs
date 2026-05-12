//! Glob / extglob → regex translation, used for `==` / `!=` matching.
//!
//! Bash's pattern semantics for `[[ x == pat ]]`:
//!
//! - `*` matches zero or more characters.
//! - `?` matches any single character.
//! - `[abc]`, `[!abc]`, `[a-z]`, `[[:alpha:]]` — bracket expressions.
//! - `\X` escapes the next character (outside brackets).
//! - **Quoted** portions are matched literally — glob metacharacters in
//!   them lose their meaning. We model this by accepting a list of
//!   `(text, is_pattern)` segments rather than a single pattern string.
//!
//! Extglob (`?(...)`, `*(...)`, `+(...)`, `@(...)`, `!(...)`) is **not
//! supported in v1** — patterns containing those forms are translated as
//! literal text. This is a known v1 limitation; see `aidocs/PLAN.md`.
//!
//! Implementation: translate the glob to an anchored regex and let the
//! `regex` crate do the matching.

use crate::ast::{Word, WordPart};
use regex::Regex;

fn segments(word: &Word, expand_var: impl Fn(&str) -> String) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for p in &word.parts {
        match p {
            WordPart::Literal(s) => out.push((s.clone(), true)),
            WordPart::Quoted(s) => out.push((s.clone(), false)),
            // Per spec: an unquoted $var expansion is treated as pattern,
            // but a quoted "$var" expansion is matched literally.
            WordPart::Var(name) => out.push((expand_var(name), true)),
            WordPart::QuotedVar(name) => out.push((expand_var(name), false)),
        }
    }
    out
}

/// Compile the RHS of `==` / `!=` into a regex. `nocase` honors `nocasematch`.
pub fn compile_glob<F>(rhs: &Word, nocase: bool, expand_var: F) -> Result<Regex, regex::Error>
where
    F: Fn(&str) -> String,
{
    let segs = segments(rhs, expand_var);
    let mut re = String::with_capacity(64);
    re.push('^');
    for (text, is_pattern) in &segs {
        if *is_pattern {
            translate_glob(text, &mut re);
        } else {
            re.push_str(&regex::escape(text));
        }
    }
    re.push('$');
    let mut builder = regex::RegexBuilder::new(&re);
    builder.case_insensitive(nocase);
    builder.build()
}

/// Test the LHS string against the compiled pattern.
pub fn matches_glob(re: &Regex, lhs: &str) -> bool {
    re.is_match(lhs)
}

/// Compile the RHS of `=~` into a regex. Same quoting rule: quoted parts
/// are passed to `regex::escape`, unquoted parts are passed through (so
/// the user can write regex metacharacters); unquoted `$var` is also
/// passed through (per bash's rule that an unquoted variable expansion
/// keeps its regex specialness).
pub fn compile_regex<F>(rhs: &Word, nocase: bool, expand_var: F) -> Result<Regex, regex::Error>
where
    F: Fn(&str) -> String,
{
    let mut pattern = String::new();
    for p in &rhs.parts {
        match p {
            WordPart::Literal(s) => pattern.push_str(s),
            WordPart::Quoted(s) => pattern.push_str(&regex::escape(s)),
            WordPart::Var(name) => pattern.push_str(&expand_var(name)),
            WordPart::QuotedVar(name) => pattern.push_str(&regex::escape(&expand_var(name))),
        }
    }
    let mut builder = regex::RegexBuilder::new(&pattern);
    builder.case_insensitive(nocase);
    builder.build()
}

/// Translate a single glob segment into regex syntax, appending to `out`.
fn translate_glob(glob: &str, out: &mut String) {
    let bytes = glob.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'*' => {
                out.push_str(".*");
                i += 1;
            }
            b'?' => {
                out.push('.');
                i += 1;
            }
            b'\\' => {
                if let Some(&nc) = bytes.get(i + 1) {
                    out.push_str(&regex::escape(&(nc as char).to_string()));
                    i += 2;
                } else {
                    out.push_str(r"\\");
                    i += 1;
                }
            }
            b'[' => {
                if let Some(end) = find_bracket_end(bytes, i + 1) {
                    translate_bracket(&bytes[i + 1..end], out);
                    i = end + 1;
                } else {
                    out.push_str(r"\[");
                    i += 1;
                }
            }
            // Regex metacharacters that aren't glob meta — escape.
            b'.' | b'+' | b'(' | b')' | b'{' | b'}' | b'|' | b'^' | b'$' => {
                out.push('\\');
                out.push(c as char);
                i += 1;
            }
            _ => {
                out.push(c as char);
                i += 1;
            }
        }
    }
}

fn find_bracket_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    // A `]` immediately after `[` (or `[!`) is treated as a literal.
    if bytes.get(i) == Some(&b'!') {
        i += 1;
    }
    if bytes.get(i) == Some(&b']') {
        i += 1;
    }
    while i < bytes.len() {
        if bytes[i] == b']' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn translate_bracket(inner: &[u8], out: &mut String) {
    out.push('[');
    let mut i = 0;
    if inner.first() == Some(&b'!') {
        out.push('^');
        i = 1;
    }
    while i < inner.len() {
        let c = inner[i];
        if c == b'[' && inner.get(i + 1) == Some(&b':') {
            // POSIX character class: [:alpha:] etc.
            if let Some(end) = find_posix_class_end(inner, i + 2) {
                out.push_str("[:");
                out.push_str(std::str::from_utf8(&inner[i + 2..end]).unwrap_or(""));
                out.push_str(":]");
                i = end + 2;
                continue;
            }
        }
        if c == b'\\'
            && let Some(&nc) = inner.get(i + 1)
        {
            out.push('\\');
            out.push(nc as char);
            i += 2;
            continue;
        }
        if c == b']' || c == b'\\' {
            out.push('\\');
        }
        out.push(c as char);
        i += 1;
    }
    out.push(']');
}

fn find_posix_class_end(inner: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 1 < inner.len() {
        if inner[i] == b':' && inner[i + 1] == b']' {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Word, WordPart};

    fn w_lit(s: &str) -> Word {
        Word::literal(s)
    }

    fn matches(pat: &Word, s: &str) -> bool {
        let re = compile_glob(pat, false, |_| String::new()).expect("compile");
        matches_glob(&re, s)
    }

    #[test]
    fn star() {
        assert!(matches(&w_lit("*.txt"), "foo.txt"));
        assert!(!matches(&w_lit("*.txt"), "foo.md"));
    }

    #[test]
    fn question_mark() {
        assert!(matches(&w_lit("?oo"), "foo"));
        assert!(!matches(&w_lit("?oo"), "fooo"));
    }

    #[test]
    fn bracket_class() {
        assert!(matches(&w_lit("[abc]oo"), "boo"));
        assert!(!matches(&w_lit("[abc]oo"), "doo"));
    }

    #[test]
    fn negated_bracket() {
        assert!(matches(&w_lit("[!abc]oo"), "doo"));
        assert!(!matches(&w_lit("[!abc]oo"), "aoo"));
    }

    #[test]
    fn posix_char_class() {
        assert!(matches(&w_lit("[[:digit:]][[:digit:]]"), "42"));
        assert!(!matches(&w_lit("[[:digit:]][[:digit:]]"), "ab"));
    }

    #[test]
    fn quoted_metas_are_literal() {
        let pat = Word {
            parts: vec![WordPart::Quoted("*.txt".into())],
        };
        assert!(matches(&pat, "*.txt"));
        assert!(!matches(&pat, "foo.txt"));
    }

    #[test]
    fn nocase() {
        let re = compile_glob(&w_lit("FOO*"), true, |_| String::new()).unwrap();
        assert!(matches_glob(&re, "foobar"));
    }

    #[test]
    fn anchored() {
        // Glob is whole-string match.
        assert!(!matches(&w_lit("foo"), "foobar"));
        assert!(matches(&w_lit("foo*"), "foobar"));
    }

    #[test]
    fn regex_uses_substring_match() {
        let re = compile_regex(&w_lit("oba"), false, |_| String::new()).unwrap();
        assert!(re.is_match("foobar"));
    }

    #[test]
    fn regex_quoted_is_literal() {
        let pat = Word {
            parts: vec![WordPart::Quoted(".*".into())],
        };
        let re = compile_regex(&pat, false, |_| String::new()).unwrap();
        // `.*` is escaped to literal dot-star.
        assert!(re.is_match("foo.*bar"));
        assert!(!re.is_match("foobar"));
    }
}
