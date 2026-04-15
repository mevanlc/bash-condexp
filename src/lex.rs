//! Lexer for bash `[[` conditional expression syntax.
//!
//! The lexer emits a flat stream of [`Token`]s. Operators (`==`, `!=`, `=~`,
//! `<`, `>`, `&&`, `||`, `!`, `[[`, `]]`) are only recognized when
//! **unquoted** — so `[[ "==" == "==" ]]` would have the first and third
//! as operator `==` and the second/fourth as quoted word literals. This
//! matches bash.
//!
//! Words are assembled with per-part quoting preserved in [`Word`], so the
//! evaluator can honor bash's literalness rules for the RHS of `==` / `=~`.
//!
//! A hand-written lexer is used (rather than `logos`) because assembling
//! multi-part words across quote boundaries doesn't fit logos's one-token-
//! per-regex model cleanly.

use crate::ast::{Word, WordPart};
use crate::error::ParseError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    OpenBracket,  // [[
    CloseBracket, // ]]
    AndAnd,       // &&
    OrOr,         // ||
    Bang,         // !
    OpenParen,    // (  -- tracked for good error messages
    CloseParen,   // )
    Eq,           // = or ==
    NotEq,        // !=
    RegexEq,      // =~
    Lt,           // <
    Gt,           // >
    Word(Word),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spanned<T> {
    pub value: T,
    pub start: usize,
    pub end: usize,
}

pub fn lex(input: &str) -> Result<Vec<Spanned<Token>>, ParseError> {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    // After emitting `==` / `!=` / `=~`, the next non-whitespace run is
    // read with the special word reader: regex/glob metacharacters
    // (parens, |, *, ?, +, ^, $, ., <, >, and most importantly nested
    // [...] including `[[:class:]]`) pass through as part of the word;
    // only whitespace and an unquoted top-level `]]` terminate.
    let mut pattern_or_regex_mode = false;

    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        if pattern_or_regex_mode {
            let (word, end) = read_pattern_word(bytes, i)?;
            out.push(span(Token::Word(word), i, end));
            i = end;
            pattern_or_regex_mode = false;
            continue;
        }

        // Multi-char unquoted operators first.
        if c == b'[' && bytes.get(i + 1) == Some(&b'[') {
            out.push(span(Token::OpenBracket, i, i + 2));
            i += 2;
            continue;
        }
        if c == b']' && bytes.get(i + 1) == Some(&b']') {
            out.push(span(Token::CloseBracket, i, i + 2));
            i += 2;
            continue;
        }
        if c == b'&' && bytes.get(i + 1) == Some(&b'&') {
            out.push(span(Token::AndAnd, i, i + 2));
            i += 2;
            continue;
        }
        if c == b'|' && bytes.get(i + 1) == Some(&b'|') {
            out.push(span(Token::OrOr, i, i + 2));
            i += 2;
            continue;
        }
        if c == b'=' && bytes.get(i + 1) == Some(&b'=') {
            out.push(span(Token::Eq, i, i + 2));
            i += 2;
            pattern_or_regex_mode = true;
            continue;
        }
        if c == b'!' && bytes.get(i + 1) == Some(&b'=') {
            out.push(span(Token::NotEq, i, i + 2));
            i += 2;
            pattern_or_regex_mode = true;
            continue;
        }
        if c == b'=' && bytes.get(i + 1) == Some(&b'~') {
            out.push(span(Token::RegexEq, i, i + 2));
            i += 2;
            pattern_or_regex_mode = true;
            continue;
        }

        // Single-char operators. `<` and `>` are operators only if
        // surrounded by whitespace — otherwise they'd collide with redirection
        // syntax in real bash. Inside our limited grammar we treat them as
        // operators whenever they appear unquoted between whitespace-delimited
        // tokens. Good enough for v1.
        match c {
            b'!' => {
                out.push(span(Token::Bang, i, i + 1));
                i += 1;
                continue;
            }
            b'=' => {
                out.push(span(Token::Eq, i, i + 1));
                i += 1;
                pattern_or_regex_mode = true;
                continue;
            }
            b'<' => {
                out.push(span(Token::Lt, i, i + 1));
                i += 1;
                continue;
            }
            b'>' => {
                out.push(span(Token::Gt, i, i + 1));
                i += 1;
                continue;
            }
            b'(' => {
                out.push(span(Token::OpenParen, i, i + 1));
                i += 1;
                continue;
            }
            b')' => {
                out.push(span(Token::CloseParen, i, i + 1));
                i += 1;
                continue;
            }
            _ => {}
        }

        // Otherwise: start of a word.
        let (word, end) = read_word(bytes, i)?;
        out.push(span(Token::Word(word), i, end));
        i = end;
    }
    Ok(out)
}

/// Read a pattern-or-regex-RHS word. Almost everything is literal; only
/// whitespace and an unquoted closing `]]` (when not inside a bracket
/// expression) terminate. Quotes and `$var` still work. Used after
/// `==` / `!=` / `=~`.
fn read_pattern_word(bytes: &[u8], start: usize) -> Result<(Word, usize), ParseError> {
    let mut word = Word::default();
    let mut lit = String::new();
    let mut i = start;
    let mut bracket_depth: i32 = 0;

    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() && bracket_depth == 0 {
            break;
        }
        if c == b']' && bytes.get(i + 1) == Some(&b']') && bracket_depth == 0 {
            break;
        }
        if c == b'[' {
            bracket_depth += 1;
        } else if c == b']' && bracket_depth > 0 {
            bracket_depth -= 1;
        }
        match c {
            b'\'' => {
                flush_literal(&mut word, &mut lit);
                let (text, end) = read_single_quoted(bytes, i)?;
                word.push(WordPart::Quoted(text));
                i = end;
            }
            b'"' => {
                flush_literal(&mut word, &mut lit);
                let end = read_double_quoted(bytes, i, &mut word)?;
                i = end;
            }
            b'$' => {
                if let Some((var, end)) = read_var_ref(bytes, i) {
                    flush_literal(&mut word, &mut lit);
                    word.push(WordPart::Var(var));
                    i = end;
                } else {
                    lit.push('$');
                    i += 1;
                }
            }
            b'\\' => {
                if let Some(&nc) = bytes.get(i + 1) {
                    // In regex context we preserve the backslash so the
                    // regex engine sees `\.`, `\d`, etc. as written.
                    lit.push('\\');
                    lit.push(nc as char);
                    i += 2;
                } else {
                    lit.push('\\');
                    i += 1;
                }
            }
            _ => {
                lit.push(c as char);
                i += 1;
            }
        }
    }
    flush_literal(&mut word, &mut lit);
    Ok((word, i))
}

fn span<T>(value: T, start: usize, end: usize) -> Spanned<T> {
    Spanned { value, start, end }
}

/// Read a word starting at `start`. Words are terminated by unquoted
/// whitespace or by the first character of an operator/bracket token.
fn read_word(bytes: &[u8], start: usize) -> Result<(Word, usize), ParseError> {
    let mut word = Word::default();
    let mut lit = String::new();
    let mut i = start;

    loop {
        if i >= bytes.len() {
            break;
        }
        let c = bytes[i];

        // Unquoted terminators.
        if c.is_ascii_whitespace() {
            break;
        }
        if is_operator_start(bytes, i) {
            break;
        }

        match c {
            b'\'' => {
                flush_literal(&mut word, &mut lit);
                let (text, end) = read_single_quoted(bytes, i)?;
                word.push(WordPart::Quoted(text));
                i = end;
            }
            b'"' => {
                flush_literal(&mut word, &mut lit);
                let end = read_double_quoted(bytes, i, &mut word)?;
                i = end;
            }
            b'$' => {
                if let Some((var, end)) = read_var_ref(bytes, i) {
                    flush_literal(&mut word, &mut lit);
                    word.push(WordPart::Var(var));
                    i = end;
                } else {
                    // Bare `$` — treat as literal.
                    lit.push('$');
                    i += 1;
                }
            }
            b'\\' => {
                // Backslash escapes the next character, outside quotes.
                if let Some(&nc) = bytes.get(i + 1) {
                    lit.push(nc as char);
                    i += 2;
                } else {
                    lit.push('\\');
                    i += 1;
                }
            }
            _ => {
                lit.push(c as char);
                i += 1;
            }
        }
    }

    flush_literal(&mut word, &mut lit);
    Ok((word, i))
}

fn flush_literal(word: &mut Word, lit: &mut String) {
    if !lit.is_empty() {
        word.push(WordPart::Literal(std::mem::take(lit)));
    }
}

/// Is position `i` the start of an unquoted operator/bracket token?
fn is_operator_start(bytes: &[u8], i: usize) -> bool {
    let c = bytes[i];
    let n = bytes.get(i + 1).copied();
    matches!(
        (c, n),
        (b'[', Some(b'['))
            | (b']', Some(b']'))
            | (b'&', Some(b'&'))
            | (b'|', Some(b'|'))
    )
        // single-char operators (only when not adjacent to word chars)
        || matches!(c, b'<' | b'>' | b'(' | b')')
        // `=` begins a word only mid-word; at word start treat as operator
        // (shouldn't happen because words can't start with `=` — bash only
        // accepts `=` as an operator, not part of a word). Be conservative.
        || c == b'='
}

fn read_single_quoted(bytes: &[u8], start: usize) -> Result<(String, usize), ParseError> {
    // `'...'` — no escapes inside.
    debug_assert_eq!(bytes[start], b'\'');
    let mut s = String::new();
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            return Ok((s, i + 1));
        }
        s.push(bytes[i] as char);
        i += 1;
    }
    Err(ParseError::UnterminatedString { pos: start })
}

fn read_double_quoted(bytes: &[u8], start: usize, word: &mut Word) -> Result<usize, ParseError> {
    // `"..."` — supports `$var`, `${var}`, and backslash escapes for a few
    // chars. Other characters pass through literally.
    debug_assert_eq!(bytes[start], b'"');
    let mut i = start + 1;
    let mut buf = String::new();
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'"' {
            if !buf.is_empty() {
                word.push(WordPart::Quoted(std::mem::take(&mut buf)));
            }
            return Ok(i + 1);
        }
        if c == b'\\' {
            if let Some(&nc) = bytes.get(i + 1) {
                // In double quotes, backslash escapes only a few chars;
                // otherwise both the backslash and char are preserved.
                match nc {
                    b'$' | b'`' | b'"' | b'\\' | b'\n' => {
                        if nc != b'\n' {
                            buf.push(nc as char);
                        }
                        i += 2;
                        continue;
                    }
                    _ => {
                        buf.push('\\');
                        buf.push(nc as char);
                        i += 2;
                        continue;
                    }
                }
            }
        }
        if c == b'$' {
            if let Some((var, end)) = read_var_ref(bytes, i) {
                if !buf.is_empty() {
                    word.push(WordPart::Quoted(std::mem::take(&mut buf)));
                }
                word.push(WordPart::Var(var));
                i = end;
                continue;
            }
        }
        buf.push(c as char);
        i += 1;
    }
    Err(ParseError::UnterminatedString { pos: start })
}

/// Read `$name` or `${name}`. Returns `None` if the `$` isn't a real ref.
fn read_var_ref(bytes: &[u8], start: usize) -> Option<(String, usize)> {
    debug_assert_eq!(bytes[start], b'$');
    let next = *bytes.get(start + 1)?;
    if next == b'{' {
        let mut j = start + 2;
        let name_start = j;
        while j < bytes.len() && is_ident_cont(bytes[j]) {
            j += 1;
        }
        if j == name_start || bytes.get(j) != Some(&b'}') {
            return None;
        }
        let name = std::str::from_utf8(&bytes[name_start..j]).ok()?.to_owned();
        Some((name, j + 1))
    } else if is_ident_start(next) {
        let mut j = start + 1;
        while j < bytes.len() && is_ident_cont(bytes[j]) {
            j += 1;
        }
        let name = std::str::from_utf8(&bytes[start + 1..j]).ok()?.to_owned();
        Some((name, j))
    } else {
        None
    }
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_ident_cont(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(input: &str) -> Vec<Token> {
        lex(input).unwrap().into_iter().map(|s| s.value).collect()
    }

    #[test]
    fn lex_simple_primary() {
        let t = toks("-f foo.txt");
        assert_eq!(
            t,
            vec![
                Token::Word(Word::literal("-f")),
                Token::Word(Word::literal("foo.txt")),
            ]
        );
    }

    #[test]
    fn lex_double_bracket_and_ops() {
        let t = toks("[[ -f foo && -d bar ]]");
        assert_eq!(
            t,
            vec![
                Token::OpenBracket,
                Token::Word(Word::literal("-f")),
                Token::Word(Word::literal("foo")),
                Token::AndAnd,
                Token::Word(Word::literal("-d")),
                Token::Word(Word::literal("bar")),
                Token::CloseBracket,
            ]
        );
    }

    #[test]
    fn lex_equals_vs_quoted_equals() {
        // Unquoted `==` is an operator; quoted `"=="` is a literal word.
        let t = toks(r#"[[ $x == "==" ]]"#);
        assert_eq!(
            t,
            vec![
                Token::OpenBracket,
                Token::Word(Word {
                    parts: vec![WordPart::Var("x".to_string())]
                }),
                Token::Eq,
                Token::Word(Word {
                    parts: vec![WordPart::Quoted("==".to_string())]
                }),
                Token::CloseBracket,
            ]
        );
    }

    #[test]
    fn lex_regex_eq() {
        let t = toks("[[ $x =~ ^foo ]]");
        assert!(matches!(t[2], Token::RegexEq));
    }

    #[test]
    fn lex_var_and_concat() {
        let t = toks(r#"$foo"bar"$baz"#);
        assert_eq!(
            t,
            vec![Token::Word(Word {
                parts: vec![
                    WordPart::Var("foo".to_string()),
                    WordPart::Quoted("bar".to_string()),
                    WordPart::Var("baz".to_string()),
                ]
            })]
        );
    }

    #[test]
    fn lex_braced_var() {
        let t = toks("${HOME}");
        assert_eq!(
            t,
            vec![Token::Word(Word {
                parts: vec![WordPart::Var("HOME".to_string())]
            })]
        );
    }

    #[test]
    fn lex_single_quote_literal() {
        let t = toks("'$no expand'");
        assert_eq!(
            t,
            vec![Token::Word(Word {
                parts: vec![WordPart::Quoted("$no expand".to_string())]
            })]
        );
    }

    #[test]
    fn lex_unterminated_string() {
        assert!(matches!(
            lex(r#""unclosed"#),
            Err(ParseError::UnterminatedString { .. })
        ));
    }

    #[test]
    fn lex_or_and_not() {
        let t = toks("!a || b && c");
        assert_eq!(
            t,
            vec![
                Token::Bang,
                Token::Word(Word::literal("a")),
                Token::OrOr,
                Token::Word(Word::literal("b")),
                Token::AndAnd,
                Token::Word(Word::literal("c")),
            ]
        );
    }
}
