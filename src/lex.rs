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

use crate::ast::{
    CaseModifyKind, ParameterExpansion, ParameterOp, RemoveKind, ReplaceKind, Word, WordPart,
};
use crate::error::ParseError;
use crate::parse::ParseOptions;

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

/// Lex using the default, Bash-compatible syntax options.
pub fn lex(input: &str) -> Result<Vec<Spanned<Token>>, ParseError> {
    lex_with_options(input, ParseOptions::default())
}

/// Lex using explicit syntax options.
pub fn lex_with_options(
    input: &str,
    options: ParseOptions,
) -> Result<Vec<Spanned<Token>>, ParseError> {
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
            let (word, end) = read_pattern_word(bytes, i, options)?;
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
        let (word, end) = read_word(bytes, i, options)?;
        out.push(span(Token::Word(word), i, end));
        i = end;
    }
    Ok(out)
}

/// Read a pattern-or-regex-RHS word. Almost everything is literal; only
/// whitespace and an unquoted closing `]]` (when not inside a bracket
/// expression) terminate. Quotes and `$var` still work. Used after
/// `==` / `!=` / `=~`.
fn read_pattern_word(
    bytes: &[u8],
    start: usize,
    options: ParseOptions,
) -> Result<(Word, usize), ParseError> {
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
                let end = read_double_quoted(bytes, i, &mut word, options)?;
                i = end;
            }
            b'$' => {
                if let Some((part, end)) = read_dollar_expansion(bytes, i, false, options)? {
                    flush_literal(&mut word, &mut lit);
                    word.push(part);
                    i = end;
                } else {
                    lit.push('$');
                    i += 1;
                }
            }
            b'\\' => {
                if bytes.get(i + 1).is_some() {
                    // In regex context we preserve the backslash so the
                    // regex engine sees `\.`, `\d`, etc. as written.
                    lit.push('\\');
                    let (next, end) = decode_char(bytes, i + 1);
                    lit.push(next);
                    i = end;
                } else {
                    lit.push('\\');
                    i += 1;
                }
            }
            _ => {
                let (ch, end) = decode_char(bytes, i);
                lit.push(ch);
                i = end;
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
fn read_word(
    bytes: &[u8],
    start: usize,
    options: ParseOptions,
) -> Result<(Word, usize), ParseError> {
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
                let end = read_double_quoted(bytes, i, &mut word, options)?;
                i = end;
            }
            b'$' => {
                if let Some((part, end)) = read_dollar_expansion(bytes, i, false, options)? {
                    flush_literal(&mut word, &mut lit);
                    word.push(part);
                    i = end;
                } else {
                    // Bare `$` — treat as literal.
                    lit.push('$');
                    i += 1;
                }
            }
            b'\\' => {
                // Backslash escapes the next character, outside quotes.
                if bytes.get(i + 1).is_some() {
                    let (next, end) = decode_char(bytes, i + 1);
                    lit.push(next);
                    i = end;
                } else {
                    lit.push('\\');
                    i += 1;
                }
            }
            _ => {
                let (ch, end) = decode_char(bytes, i);
                lit.push(ch);
                i = end;
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
        let (ch, end) = decode_char(bytes, i);
        s.push(ch);
        i = end;
    }
    Err(ParseError::UnterminatedString { pos: start })
}

fn read_double_quoted(
    bytes: &[u8],
    start: usize,
    word: &mut Word,
    options: ParseOptions,
) -> Result<usize, ParseError> {
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
        if c == b'\\'
            && let Some(&nc) = bytes.get(i + 1)
        {
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
                    let (next, end) = decode_char(bytes, i + 1);
                    buf.push(next);
                    i = end;
                    continue;
                }
            }
        }
        if c == b'$'
            && let Some((part, end)) = read_dollar_expansion(bytes, i, true, options)?
        {
            if !buf.is_empty() {
                word.push(WordPart::Quoted(std::mem::take(&mut buf)));
            }
            word.push(part);
            i = end;
            continue;
        }
        let (ch, end) = decode_char(bytes, i);
        buf.push(ch);
        i = end;
    }
    Err(ParseError::UnterminatedString { pos: start })
}

/// Read an unbraced variable or a balanced braced parameter expansion.
fn read_dollar_expansion(
    bytes: &[u8],
    start: usize,
    quoted: bool,
    options: ParseOptions,
) -> Result<Option<(WordPart, usize)>, ParseError> {
    debug_assert_eq!(bytes[start], b'$');
    let Some(&next) = bytes.get(start + 1) else {
        return Ok(None);
    };
    if next == b'{' {
        let end = find_parameter_end(bytes, start)?;
        let inner = &bytes[start + 2..end];
        let raw = String::from_utf8_lossy(inner).into_owned();
        let part = match parse_parameter_expansion(inner, start, options)? {
            Some(expansion) => WordPart::Expansion { expansion, quoted },
            None if quoted => WordPart::QuotedVar(raw),
            None => WordPart::Var(raw),
        };
        Ok(Some((part, end + 1)))
    } else if is_ident_start(next) {
        let mut j = start + 1;
        while j < bytes.len() && is_ident_cont(bytes[j]) {
            j += 1;
        }
        let name = String::from_utf8_lossy(&bytes[start + 1..j]).into_owned();
        Ok(Some((
            if quoted {
                WordPart::QuotedVar(name)
            } else {
                WordPart::Var(name)
            },
            j,
        )))
    } else {
        Ok(None)
    }
}

fn find_parameter_end(bytes: &[u8], start: usize) -> Result<usize, ParseError> {
    let mut depth = 1usize;
    let mut quote = None;
    let mut i = start + 2;
    while i < bytes.len() {
        let c = bytes[i];
        if let Some(q) = quote {
            if c == b'\\' && q == b'"' {
                i = (i + 2).min(bytes.len());
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match c {
            b'\'' | b'"' => {
                quote = Some(c);
                i += 1;
            }
            b'\\' => i = (i + 2).min(bytes.len()),
            b'$' if bytes.get(i + 1) == Some(&b'{') => {
                depth += 1;
                i += 2;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(i);
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    Err(ParseError::UnterminatedParameterExpansion { pos: start })
}

fn parse_parameter_expansion(
    inner: &[u8],
    source_pos: usize,
    options: ParseOptions,
) -> Result<Option<ParameterExpansion>, ParseError> {
    // Bash gives `${#}` the special-parameter meaning, but treats a leading
    // `#` followed by a complete parameter name as the length operator.
    if inner.first() == Some(&b'#') && is_bash_parameter_name(&inner[1..]) {
        let name = parameter_name(&inner[1..]);
        return Ok(Some(ParameterExpansion {
            name,
            op: ParameterOp::Length,
        }));
    }

    let split = if let Some(end) = bash_parameter_name_end(inner) {
        if end == inner.len() {
            return Ok(None);
        }
        match inner[end] {
            b'#' | b'%' | b'/' | b':' | b'^' | b',' | b'-' | b'+' | b'=' | b'?' => Some(end),
            _ => return Err(invalid_parameter_name(inner, source_pos)),
        }
    } else if options.allows_punctuation_variables() && is_punctuation_parameter_name(inner) {
        find_custom_operator(inner)
    } else if options.allows_punctuation_variables() {
        let split = find_custom_operator(inner);
        if split.is_some_and(|i| is_punctuation_parameter_name(&inner[..i])) {
            split
        } else {
            return Err(invalid_parameter_name(inner, source_pos));
        }
    } else {
        return Err(invalid_parameter_name(inner, source_pos));
    };

    let Some(split) = split else {
        return Ok(None);
    };
    let name = String::from_utf8_lossy(&inner[..split]).into_owned();
    let operator = inner[split];
    let tail = &inner[split + 1..];

    let op = match operator {
        b'#' => {
            let (kind, pattern) = if tail.first() == Some(&b'#') {
                (RemoveKind::LongestPrefix, &tail[1..])
            } else {
                (RemoveKind::ShortestPrefix, tail)
            };
            ParameterOp::Remove {
                kind,
                pattern: read_parameter_word(pattern, options)?,
            }
        }
        b'%' => {
            let (kind, pattern) = if tail.first() == Some(&b'%') {
                (RemoveKind::LongestSuffix, &tail[1..])
            } else {
                (RemoveKind::ShortestSuffix, tail)
            };
            ParameterOp::Remove {
                kind,
                pattern: read_parameter_word(pattern, options)?,
            }
        }
        b'/' => parse_replacement(tail, options)?,
        b':' => match tail.first() {
            Some(b'-') => ParameterOp::DefaultValue {
                test_null: true,
                word: read_parameter_word(&tail[1..], options)?,
            },
            Some(b'+') => ParameterOp::AlternateValue {
                test_null: true,
                word: read_parameter_word(&tail[1..], options)?,
            },
            Some(b'=') | Some(b'?') => {
                return Err(ParseError::UnsupportedParameterExpansion {
                    operator: format!(":{}", tail[0] as char),
                    pos: source_pos + split + 2,
                });
            }
            _ => {
                if tail.is_empty() {
                    return Err(ParseError::InvalidParameterExpansion {
                        raw: String::from_utf8_lossy(inner).into_owned(),
                        pos: source_pos,
                        reason: "substring offset is missing".into(),
                    });
                }
                let separator = find_substring_separator(tail);
                let (offset, length) = match separator {
                    Some(i) => (
                        read_parameter_word(&tail[..i], options)?,
                        Some(read_parameter_word(&tail[i + 1..], options)?),
                    ),
                    None => (read_parameter_word(tail, options)?, None),
                };
                ParameterOp::Substring { offset, length }
            }
        },
        b'^' | b',' => {
            let doubled = tail.first() == Some(&operator);
            let pattern = if doubled { &tail[1..] } else { tail };
            let kind = match (operator, doubled) {
                (b'^', false) => CaseModifyKind::UpperFirst,
                (b'^', true) => CaseModifyKind::UpperAll,
                (b',', false) => CaseModifyKind::LowerFirst,
                (b',', true) => CaseModifyKind::LowerAll,
                _ => unreachable!(),
            };
            ParameterOp::CaseModify {
                kind,
                pattern: (!pattern.is_empty())
                    .then(|| read_parameter_word(pattern, options))
                    .transpose()?,
            }
        }
        b'-' => ParameterOp::DefaultValue {
            test_null: false,
            word: read_parameter_word(tail, options)?,
        },
        b'+' => ParameterOp::AlternateValue {
            test_null: false,
            word: read_parameter_word(tail, options)?,
        },
        b'=' | b'?' => {
            return Err(ParseError::UnsupportedParameterExpansion {
                operator: (operator as char).to_string(),
                pos: source_pos + split + 2,
            });
        }
        _ => unreachable!(),
    };
    Ok(Some(ParameterExpansion { name, op }))
}

fn invalid_parameter_name(inner: &[u8], source_pos: usize) -> ParseError {
    ParseError::InvalidParameterExpansion {
        raw: parameter_name(inner),
        pos: source_pos,
        reason: "invalid parameter name".into(),
    }
}

fn parameter_name(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn bash_parameter_name_end(input: &[u8]) -> Option<usize> {
    let first = *input.first()?;
    if is_ident_start(first) {
        let mut end = 1;
        while end < input.len() && is_ident_cont(input[end]) {
            end += 1;
        }
        Some(end)
    } else if first.is_ascii_digit() {
        let mut end = 1;
        while end < input.len() && input[end].is_ascii_digit() {
            end += 1;
        }
        Some(end)
    } else if matches!(first, b'@' | b'*' | b'#' | b'?' | b'-' | b'$' | b'!') {
        Some(1)
    } else {
        None
    }
}

fn is_bash_parameter_name(input: &[u8]) -> bool {
    bash_parameter_name_end(input) == Some(input.len())
}

fn is_punctuation_parameter_name(input: &[u8]) -> bool {
    input.is_empty() || input.iter().all(u8::is_ascii_punctuation)
}

fn find_custom_operator(inner: &[u8]) -> Option<usize> {
    top_level_indices(inner)
        .find(|&i| i > 0 && matches!(inner[i], b'#' | b'%' | b':' | b'^' | b',' | b'-' | b'+'))
}

fn parse_replacement(tail: &[u8], options: ParseOptions) -> Result<ParameterOp, ParseError> {
    let (kind, body) = match tail.first() {
        Some(b'/') => (ReplaceKind::All, &tail[1..]),
        Some(b'#') => (ReplaceKind::Prefix, &tail[1..]),
        Some(b'%') => (ReplaceKind::Suffix, &tail[1..]),
        _ => (ReplaceKind::First, tail),
    };
    let split = top_level_indices(body).find(|&i| body[i] == b'/');
    let (pattern, replacement) = match split {
        Some(i) => (&body[..i], &body[i + 1..]),
        None => (body, &[][..]),
    };
    Ok(ParameterOp::Replace {
        kind,
        pattern: read_parameter_word(pattern, options)?,
        replacement: read_parameter_word(replacement, options)?,
    })
}

fn find_substring_separator(input: &[u8]) -> Option<usize> {
    let mut parens = 0usize;
    let mut ternaries = 0usize;
    for i in top_level_indices(input) {
        match input[i] {
            b'(' => parens += 1,
            b')' => parens = parens.saturating_sub(1),
            b'?' if parens == 0 => ternaries += 1,
            b':' if parens == 0 && ternaries > 0 => ternaries -= 1,
            b':' if parens == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

fn top_level_indices(input: &[u8]) -> impl Iterator<Item = usize> + '_ {
    let mut i = 0usize;
    std::iter::from_fn(move || {
        while i < input.len() {
            let current = i;
            match input[i] {
                b'\\' => i = (i + 2).min(input.len()),
                b'\'' | b'"' => {
                    let q = input[i];
                    i += 1;
                    while i < input.len() && input[i] != q {
                        if q == b'"' && input[i] == b'\\' {
                            i = (i + 2).min(input.len());
                        } else {
                            i += 1;
                        }
                    }
                    i = (i + 1).min(input.len());
                }
                b'$' if input.get(i + 1) == Some(&b'{') => {
                    if let Ok(end) = find_parameter_end(input, i) {
                        i = end + 1;
                    } else {
                        i = input.len();
                    }
                }
                _ => {
                    i += 1;
                    return Some(current);
                }
            }
        }
        None
    })
}

fn read_parameter_word(bytes: &[u8], options: ParseOptions) -> Result<Word, ParseError> {
    let mut word = Word::default();
    let mut literal = String::new();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' => {
                flush_literal(&mut word, &mut literal);
                let (text, end) = read_single_quoted(bytes, i)?;
                word.push(WordPart::Quoted(text));
                i = end;
            }
            b'"' => {
                flush_literal(&mut word, &mut literal);
                i = read_double_quoted(bytes, i, &mut word, options)?;
            }
            b'$' => {
                if let Some((part, end)) = read_dollar_expansion(bytes, i, false, options)? {
                    flush_literal(&mut word, &mut literal);
                    word.push(part);
                    i = end;
                } else {
                    literal.push('$');
                    i += 1;
                }
            }
            b'\\' if i + 1 < bytes.len() => {
                flush_literal(&mut word, &mut literal);
                let (next, end) = decode_char(bytes, i + 1);
                word.push(WordPart::Quoted(next.to_string()));
                i = end;
            }
            _ => {
                let (ch, end) = decode_char(bytes, i);
                literal.push(ch);
                i = end;
            }
        }
    }
    flush_literal(&mut word, &mut literal);
    Ok(word)
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_ident_cont(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn decode_char(bytes: &[u8], start: usize) -> (char, usize) {
    let ch = std::str::from_utf8(&bytes[start..])
        .expect("lexer input originates from a valid UTF-8 string")
        .chars()
        .next()
        .expect("caller checked that a byte is available");
    (ch, start + ch.len_utf8())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(input: &str) -> Vec<Token> {
        lex(input).unwrap().into_iter().map(|s| s.value).collect()
    }

    fn toks_with_punctuation_vars(input: &str) -> Vec<Token> {
        let options = ParseOptions::default().punctuation_variables(true);
        lex_with_options(input, options)
            .unwrap()
            .into_iter()
            .map(|s| s.value)
            .collect()
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
    fn lex_bash_special_and_positional_parameters() {
        let t = toks("${#}${##}${?}${10}");
        assert_eq!(
            t,
            vec![Token::Word(Word {
                parts: vec![
                    WordPart::Var("#".to_string()),
                    WordPart::Expansion {
                        expansion: ParameterExpansion {
                            name: "#".to_string(),
                            op: ParameterOp::Length,
                        },
                        quoted: false,
                    },
                    WordPart::Var("?".to_string()),
                    WordPart::Var("10".to_string()),
                ]
            })]
        );
    }

    #[test]
    fn lex_custom_braced_vars() {
        assert!(matches!(
            lex("${.}"),
            Err(ParseError::InvalidParameterExpansion { .. })
        ));

        let t = toks_with_punctuation_vars("${}${/}${//}${.}${/.}");
        assert_eq!(
            t,
            vec![Token::Word(Word {
                parts: vec![
                    WordPart::Var("".to_string()),
                    WordPart::Var("/".to_string()),
                    WordPart::Var("//".to_string()),
                    WordPart::Var(".".to_string()),
                    WordPart::Var("/.".to_string()),
                ]
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
