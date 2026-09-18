//! Compiled glob and extglob matching used by conditional and parameter
//! expansion patterns.

use crate::ast::{Word, WordPart};
use regex::Regex;
use std::collections::{HashMap, HashSet, VecDeque};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PatternError {
    #[error("unterminated extended glob")]
    UnterminatedExtglob,
    #[error("parameter expansion must be evaluated before compiling a pattern")]
    UnexpandedParameter,
    #[error(transparent)]
    Regex(#[from] regex::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GlobOptions {
    pub case_insensitive: bool,
    pub extglob: bool,
}

#[derive(Debug, Clone)]
pub struct CompiledGlob {
    sequence: Vec<Atom>,
    options: GlobOptions,
}

#[derive(Debug, Clone)]
enum Atom {
    Literal(char),
    AnyChar,
    AnyString,
    Class(Regex),
    Extglob {
        kind: ExtglobKind,
        alternatives: Vec<Vec<Atom>>,
    },
}

#[derive(Debug, Clone, Copy)]
enum ExtglobKind {
    ZeroOrOne,
    ZeroOrMore,
    OneOrMore,
    ExactlyOne,
    Negate,
}

#[derive(Debug, Clone, Copy)]
struct PatternChar {
    ch: char,
    active: bool,
}

fn segments(
    word: &Word,
    expand_var: impl Fn(&str) -> String,
) -> Result<Vec<(String, bool)>, PatternError> {
    let mut out = Vec::new();
    for part in &word.parts {
        match part {
            WordPart::Literal(text) => out.push((text.clone(), true)),
            WordPart::Quoted(text) => out.push((text.clone(), false)),
            WordPart::Var(name) => out.push((expand_var(name), true)),
            WordPart::QuotedVar(name) => out.push((expand_var(name), false)),
            WordPart::Expansion { .. } => return Err(PatternError::UnexpandedParameter),
        }
    }
    Ok(out)
}

/// Compile a shell pattern. Variable references are expanded by `expand_var`;
/// structured parameter expansions must already have been evaluated.
pub fn compile_glob<F>(
    rhs: &Word,
    options: GlobOptions,
    expand_var: F,
) -> Result<CompiledGlob, PatternError>
where
    F: Fn(&str) -> String,
{
    let mut chars = Vec::new();
    for (text, active) in segments(rhs, expand_var)? {
        chars.extend(text.chars().map(|ch| PatternChar { ch, active }));
    }
    let mut parser = GlobParser {
        chars: &chars,
        pos: 0,
        options,
    };
    let sequence = parser.parse_sequence(false)?;
    Ok(CompiledGlob { sequence, options })
}

impl CompiledGlob {
    pub fn is_match(&self, input: &str) -> bool {
        let chars: Vec<char> = input.chars().collect();
        let mut memo = HashMap::new();
        match_sequence(&self.sequence, 0, 0, &chars, self.options, &mut memo).contains(&chars.len())
    }
}

pub fn matches_glob(pattern: &CompiledGlob, input: &str) -> bool {
    pattern.is_match(input)
}

struct GlobParser<'a> {
    chars: &'a [PatternChar],
    pos: usize,
    options: GlobOptions,
}

impl GlobParser<'_> {
    fn parse_sequence(&mut self, inside_extglob: bool) -> Result<Vec<Atom>, PatternError> {
        let mut out = Vec::new();
        while let Some(current) = self.chars.get(self.pos).copied() {
            if inside_extglob && current.active && matches!(current.ch, '|' | ')') {
                break;
            }
            if self.options.extglob
                && current.active
                && matches!(current.ch, '?' | '*' | '+' | '@' | '!')
                && self
                    .chars
                    .get(self.pos + 1)
                    .is_some_and(|next| next.active && next.ch == '(')
            {
                let kind = match current.ch {
                    '?' => ExtglobKind::ZeroOrOne,
                    '*' => ExtglobKind::ZeroOrMore,
                    '+' => ExtglobKind::OneOrMore,
                    '@' => ExtglobKind::ExactlyOne,
                    '!' => ExtglobKind::Negate,
                    _ => unreachable!(),
                };
                self.pos += 2;
                let mut alternatives = Vec::new();
                loop {
                    alternatives.push(self.parse_sequence(true)?);
                    match self.chars.get(self.pos) {
                        Some(PatternChar {
                            ch: '|',
                            active: true,
                        }) => self.pos += 1,
                        Some(PatternChar {
                            ch: ')',
                            active: true,
                        }) => {
                            self.pos += 1;
                            break;
                        }
                        _ => return Err(PatternError::UnterminatedExtglob),
                    }
                }
                out.push(Atom::Extglob { kind, alternatives });
                continue;
            }
            match current {
                PatternChar {
                    ch: '\\',
                    active: true,
                } if self.pos + 1 < self.chars.len() => {
                    out.push(Atom::Literal(self.chars[self.pos + 1].ch));
                    self.pos += 2;
                }
                PatternChar {
                    ch: '*',
                    active: true,
                } => {
                    out.push(Atom::AnyString);
                    self.pos += 1;
                }
                PatternChar {
                    ch: '?',
                    active: true,
                } => {
                    out.push(Atom::AnyChar);
                    self.pos += 1;
                }
                PatternChar {
                    ch: '[',
                    active: true,
                } => {
                    if let Some((regex, end)) = compile_class(self.chars, self.pos, self.options)? {
                        out.push(Atom::Class(regex));
                        self.pos = end;
                    } else {
                        out.push(Atom::Literal('['));
                        self.pos += 1;
                    }
                }
                _ => {
                    out.push(Atom::Literal(current.ch));
                    self.pos += 1;
                }
            }
        }
        Ok(out)
    }
}

type MatchMemo = HashMap<(usize, usize, usize, usize), Vec<usize>>;

fn match_sequence(
    sequence: &[Atom],
    atom_index: usize,
    input_index: usize,
    input: &[char],
    options: GlobOptions,
    memo: &mut MatchMemo,
) -> Vec<usize> {
    if atom_index == sequence.len() {
        return vec![input_index];
    }
    let key = (
        sequence.as_ptr() as usize,
        sequence.len(),
        atom_index,
        input_index,
    );
    if let Some(cached) = memo.get(&key) {
        return cached.clone();
    }
    let mut result = Vec::new();
    for next in atom_endpoints(&sequence[atom_index], input_index, input, options, memo) {
        result.extend(match_sequence(
            sequence,
            atom_index + 1,
            next,
            input,
            options,
            memo,
        ));
    }
    result.sort_unstable();
    result.dedup();
    memo.insert(key, result.clone());
    result
}

fn atom_endpoints(
    atom: &Atom,
    start: usize,
    input: &[char],
    options: GlobOptions,
    memo: &mut MatchMemo,
) -> Vec<usize> {
    match atom {
        Atom::Literal(expected) => input
            .get(start)
            .filter(|actual| chars_equal(**actual, *expected, options.case_insensitive))
            .map_or_else(Vec::new, |_| vec![start + 1]),
        Atom::AnyChar => (start < input.len())
            .then(|| start + 1)
            .into_iter()
            .collect(),
        Atom::AnyString => (start..=input.len()).collect(),
        Atom::Class(regex) => input.get(start).map_or_else(Vec::new, |ch| {
            regex
                .is_match(&ch.to_string())
                .then(|| start + 1)
                .into_iter()
                .collect()
        }),
        Atom::Extglob { kind, alternatives } => match kind {
            ExtglobKind::ExactlyOne => {
                alternative_endpoints(alternatives, start, input, options, memo)
            }
            ExtglobKind::ZeroOrOne => {
                let mut ends = vec![start];
                ends.extend(alternative_endpoints(
                    alternatives,
                    start,
                    input,
                    options,
                    memo,
                ));
                unique(ends)
            }
            ExtglobKind::ZeroOrMore => {
                repeated_endpoints(alternatives, start, input, options, memo, false)
            }
            ExtglobKind::OneOrMore => {
                repeated_endpoints(alternatives, start, input, options, memo, true)
            }
            ExtglobKind::Negate => {
                let forbidden: HashSet<_> =
                    alternative_endpoints(alternatives, start, input, options, memo)
                        .into_iter()
                        .collect();
                (start..=input.len())
                    .filter(|end| !forbidden.contains(end))
                    .collect()
            }
        },
    }
}

fn alternative_endpoints(
    alternatives: &[Vec<Atom>],
    start: usize,
    input: &[char],
    options: GlobOptions,
    memo: &mut MatchMemo,
) -> Vec<usize> {
    unique(
        alternatives
            .iter()
            .flat_map(|alternative| match_sequence(alternative, 0, start, input, options, memo))
            .collect(),
    )
}

fn repeated_endpoints(
    alternatives: &[Vec<Atom>],
    start: usize,
    input: &[char],
    options: GlobOptions,
    memo: &mut MatchMemo,
    require_one: bool,
) -> Vec<usize> {
    let first = alternative_endpoints(alternatives, start, input, options, memo);
    let mut seen: HashSet<usize> = if require_one {
        first.iter().copied().collect()
    } else {
        [start].into_iter().collect()
    };
    let mut queue: VecDeque<usize> = if require_one {
        first.into()
    } else {
        [start].into_iter().collect()
    };
    while let Some(position) = queue.pop_front() {
        for end in alternative_endpoints(alternatives, position, input, options, memo) {
            if end != position && seen.insert(end) {
                queue.push_back(end);
            }
        }
    }
    let mut result: Vec<_> = seen.into_iter().collect();
    result.sort_unstable();
    result
}

fn unique(mut values: Vec<usize>) -> Vec<usize> {
    values.sort_unstable();
    values.dedup();
    values
}

fn chars_equal(left: char, right: char, case_insensitive: bool) -> bool {
    left == right
        || (case_insensitive && left.to_lowercase().to_string() == right.to_lowercase().to_string())
}

fn compile_class(
    chars: &[PatternChar],
    start: usize,
    options: GlobOptions,
) -> Result<Option<(Regex, usize)>, PatternError> {
    let mut end = start + 1;
    if chars
        .get(end)
        .is_some_and(|c| c.active && matches!(c.ch, '!' | '^'))
    {
        end += 1;
    }
    if chars.get(end).is_some_and(|c| c.active && c.ch == ']') {
        end += 1;
    }
    while end < chars.len() {
        if chars[end].active && chars[end].ch == '\\' && end + 1 < chars.len() {
            end += 2;
            continue;
        }
        if chars[end].active
            && chars[end].ch == '['
            && chars.get(end + 1).is_some_and(|c| c.active && c.ch == ':')
        {
            end += 2;
            while end + 1 < chars.len()
                && !(chars[end].active
                    && chars[end].ch == ':'
                    && chars[end + 1].active
                    && chars[end + 1].ch == ']')
            {
                end += 1;
            }
            if end + 1 < chars.len() {
                end += 2;
                continue;
            }
        }
        if chars[end].active && chars[end].ch == ']' {
            break;
        }
        end += 1;
    }
    if end == chars.len() {
        return Ok(None);
    }

    let mut source = String::from("^[");
    let mut i = start + 1;
    if chars
        .get(i)
        .is_some_and(|c| c.active && matches!(c.ch, '!' | '^'))
    {
        source.push('^');
        i += 1;
    }
    while i < end {
        let current = chars[i];
        if current.active && current.ch == '\\' && i + 1 < end {
            source.push_str(&regex::escape(&chars[i + 1].ch.to_string()));
            i += 2;
            continue;
        }
        if current.active
            && current.ch == '['
            && chars.get(i + 1).is_some_and(|c| c.active && c.ch == ':')
        {
            let mut class_end = i + 2;
            while class_end + 1 < end
                && !(chars[class_end].active
                    && chars[class_end].ch == ':'
                    && chars[class_end + 1].active
                    && chars[class_end + 1].ch == ']')
            {
                class_end += 1;
            }
            if class_end + 1 < end {
                source.push_str("[:");
                for item in &chars[i + 2..class_end] {
                    source.push(item.ch);
                }
                source.push_str(":]");
                i = class_end + 2;
                continue;
            }
        }
        if !current.active || matches!(current.ch, '[' | ']' | '\\' | '^' | '-') {
            source.push('\\');
        }
        source.push(current.ch);
        i += 1;
    }
    source.push_str("]$");
    let mut builder = regex::RegexBuilder::new(&source);
    builder.case_insensitive(options.case_insensitive);
    Ok(Some((builder.build()?, end + 1)))
}

/// Compile the RHS of `=~` into a regular expression.
pub fn compile_regex<F>(rhs: &Word, nocase: bool, expand_var: F) -> Result<Regex, PatternError>
where
    F: Fn(&str) -> String,
{
    let mut pattern = String::new();
    for part in &rhs.parts {
        match part {
            WordPart::Literal(text) => pattern.push_str(text),
            WordPart::Quoted(text) => pattern.push_str(&regex::escape(text)),
            WordPart::Var(name) => pattern.push_str(&expand_var(name)),
            WordPart::QuotedVar(name) => {
                pattern.push_str(&regex::escape(&expand_var(name)));
            }
            WordPart::Expansion { .. } => return Err(PatternError::UnexpandedParameter),
        }
    }
    let mut builder = regex::RegexBuilder::new(&pattern);
    builder.case_insensitive(nocase);
    Ok(builder.build()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Word, WordPart};

    fn word(text: &str) -> Word {
        Word::literal(text)
    }

    fn compiled(text: &str, extglob: bool) -> CompiledGlob {
        compile_glob(
            &word(text),
            GlobOptions {
                extglob,
                case_insensitive: false,
            },
            |_| String::new(),
        )
        .unwrap()
    }

    #[test]
    fn basic_globs() {
        assert!(compiled("*.txt", false).is_match("foo.txt"));
        assert!(compiled("?oo", false).is_match("foo"));
        assert!(compiled("[!abc]oo", false).is_match("doo"));
        assert!(compiled("[[:digit:]][[:digit:]]", false).is_match("42"));
        assert!(!compiled("foo", false).is_match("foobar"));
        assert!(compiled("[[]", false).is_match("["));
        assert!(compiled(r"[\*]", false).is_match("*"));
        assert!(!compiled(r"[\*]", false).is_match(r"\"));
    }

    #[test]
    fn quoted_metas_are_literal() {
        let pattern = Word {
            parts: vec![WordPart::Quoted("*.txt".into())],
        };
        let quoted = compile_glob(&pattern, GlobOptions::default(), |_| String::new()).unwrap();
        assert!(quoted.is_match("*.txt"));
        assert!(!quoted.is_match("foo.txt"));

        let escaped = compiled(r"\*.txt", false);
        assert!(escaped.is_match("*.txt"));
        assert!(!escaped.is_match("foo.txt"));
    }

    #[test]
    fn case_insensitive() {
        let pattern = compile_glob(
            &word("FOO*"),
            GlobOptions {
                case_insensitive: true,
                extglob: false,
            },
            |_| String::new(),
        )
        .unwrap();
        assert!(pattern.is_match("foobar"));
    }

    #[test]
    fn every_extglob_operator() {
        assert!(compiled("@(foo|bar)", true).is_match("foo"));
        assert!(compiled("?(foo|bar)", true).is_match(""));
        assert!(compiled("?(foo|bar)", true).is_match("bar"));
        assert!(compiled("*(a|bc)", true).is_match("aabca"));
        assert!(compiled("+(a|bc)", true).is_match("bca"));
        assert!(compiled("!(foo|bar)", true).is_match("baz"));
        assert!(!compiled("!(foo|bar)", true).is_match("foo"));
    }

    #[test]
    fn nested_extglob() {
        assert!(compiled("+(@(a|b)|c)", true).is_match("abc"));
    }

    #[test]
    fn extglob_is_literal_when_disabled() {
        assert!(compiled("+(a)", false).is_match("+(a)"));
        assert!(!compiled("+(a)", false).is_match("aaa"));
    }

    #[test]
    fn regex_uses_substring_and_quoted_literals() {
        let re = compile_regex(&word("oba"), false, |_| String::new()).unwrap();
        assert!(re.is_match("foobar"));
        let quoted = Word {
            parts: vec![WordPart::Quoted(".*".into())],
        };
        let re = compile_regex(&quoted, false, |_| String::new()).unwrap();
        assert!(re.is_match("foo.*bar"));
        assert!(!re.is_match("foobar"));
    }
}
