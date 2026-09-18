//! Evaluator for parsed conditional expressions.
//!
//! Combinators (`&&`, `||`, `!`) live in this module too, but the actual
//! glob and regex matching for `==`/`!=`/`=~` are split into
//! [`crate::pattern`] and applied here once those modules are in place.

use std::path::Path;

use crate::ast::{
    BinaryOp, CaseModifyKind, Expr, ParameterExpansion, ParameterOp, Primary, RemoveKind,
    ReplaceKind, UnaryOp, Word, WordPart,
};
use crate::env::Env;
use crate::error::EvalError;
use crate::fs_abs::{AccessMode, FileKind, FileStat, FileSystem};
use crate::pattern;

pub struct Evaluator<'a, E: Env, F: FileSystem> {
    env: &'a mut E,
    fs: &'a F,
}

impl<'a, E: Env, F: FileSystem> Evaluator<'a, E, F> {
    pub fn new(env: &'a mut E, fs: &'a F) -> Self {
        Self { env, fs }
    }

    pub fn eval(&mut self, expr: &Expr) -> Result<bool, EvalError> {
        match expr {
            Expr::And(l, r) => {
                if self.eval(l)? {
                    self.eval(r)
                } else {
                    Ok(false)
                }
            }
            Expr::Or(l, r) => {
                if self.eval(l)? {
                    Ok(true)
                } else {
                    self.eval(r)
                }
            }
            Expr::Not(inner) => Ok(!self.eval(inner)?),
            Expr::Primary(p) => self.eval_primary(p),
        }
    }

    fn eval_primary(&mut self, p: &Primary) -> Result<bool, EvalError> {
        match p {
            Primary::Unary { op, arg } => {
                let s = self.expand(arg)?;
                self.eval_unary(*op, &s)
            }
            Primary::Binary { op, lhs, rhs } => self.eval_binary(*op, lhs, rhs),
            Primary::StringNonEmpty(w) => {
                let s = self.expand(w)?;
                Ok(!s.is_empty())
            }
        }
    }

    fn eval_unary(&self, op: UnaryOp, arg: &str) -> Result<bool, EvalError> {
        use UnaryOp::*;
        Ok(match op {
            // Existence / type
            FileExists => self.stat_path(arg).is_ok(),
            FileRegular => stat_kind_is(&self.stat_path(arg), FileKind::Regular),
            FileDir => stat_kind_is(&self.stat_path(arg), FileKind::Directory),
            FileBlock => stat_kind_is(&self.stat_path(arg), FileKind::BlockDevice),
            FileChar => stat_kind_is(&self.stat_path(arg), FileKind::CharDevice),
            FileSymlink => stat_kind_is(&self.lstat_path(arg), FileKind::Symlink),
            FileNamedPipe => stat_kind_is(&self.stat_path(arg), FileKind::NamedPipe),
            FileSocket => stat_kind_is(&self.stat_path(arg), FileKind::Socket),

            // Permissions / attributes
            FileReadable => self.access_path(arg, AccessMode::Read),
            FileWritable => self.access_path(arg, AccessMode::Write),
            FileExecutable => self.access_path(arg, AccessMode::Execute),
            FileNonEmpty => self.stat_path(arg).map(|s| s.size > 0).unwrap_or(false),
            FileSetUid => stat_mode_bit(&self.stat_path(arg), 0o4000),
            FileSetGid => stat_mode_bit(&self.stat_path(arg), 0o2000),
            FileSticky => stat_mode_bit(&self.stat_path(arg), 0o1000),
            FileOwnedByUid => self
                .stat_path(arg)
                .map(|s| s.uid == self.fs.effective_uid())
                .unwrap_or(false),
            FileOwnedByGid => self
                .stat_path(arg)
                .map(|s| s.gid == self.fs.effective_gid())
                .unwrap_or(false),
            FileNewerThanAccess => self
                .stat_path(arg)
                .map(|s| stat_after(s.mtime, s.atime))
                .unwrap_or(false),

            // Misc
            FdIsTty => {
                let fd: i32 = arg
                    .parse()
                    .map_err(|_| EvalError::InvalidFd(arg.to_string()))?;
                self.fs.is_tty(fd)
            }
            StringEmpty => arg.is_empty(),
            StringNonEmpty => !arg.is_empty(),
            VarSet => self.eval_var_set(arg),
            VarIsNameRef => self.env.is_nameref(arg),
            ShellOptSet => self.shell_option(arg),
        })
    }

    fn stat_path(&self, arg: &str) -> std::io::Result<FileStat> {
        if arg.is_empty() {
            return Err(std::io::ErrorKind::NotFound.into());
        }
        self.fs.stat(Path::new(arg))
    }

    fn lstat_path(&self, arg: &str) -> std::io::Result<FileStat> {
        if arg.is_empty() {
            return Err(std::io::ErrorKind::NotFound.into());
        }
        self.fs.lstat(Path::new(arg))
    }

    fn access_path(&self, arg: &str, mode: AccessMode) -> bool {
        !arg.is_empty() && self.fs.access(Path::new(arg), mode)
    }

    fn eval_var_set(&self, raw: &str) -> bool {
        // Split optional `name[subscript]` suffix.
        if let Some(open) = raw.find('[')
            && raw.ends_with(']')
        {
            let name = &raw[..open];
            let sub = &raw[open + 1..raw.len() - 1];
            return self.env.array_element_set(name, sub);
        }
        // Bare name → either a scalar set, or the [0] element of an array.
        self.env.var(raw).is_some() || self.env.array_element_set(raw, "0")
    }

    fn eval_binary(&mut self, op: BinaryOp, lhs: &Word, rhs: &Word) -> Result<bool, EvalError> {
        use BinaryOp::*;
        match op {
            // File comparisons
            FileSameInode => {
                let l = self.expand(lhs)?;
                let r = self.expand(rhs)?;
                Ok(match (self.stat_path(&l), self.stat_path(&r)) {
                    (Ok(a), Ok(b)) => a.dev == b.dev && a.ino == b.ino,
                    _ => false,
                })
            }
            FileNewer => {
                let l = self.expand(lhs)?;
                let r = self.expand(rhs)?;
                let a = self.stat_path(&l);
                let b = self.stat_path(&r);
                Ok(match (a, b) {
                    (Ok(_), Err(_)) => true,
                    (Ok(a), Ok(b)) => stat_after(a.mtime, b.mtime),
                    _ => false,
                })
            }
            FileOlder => {
                let l = self.expand(lhs)?;
                let r = self.expand(rhs)?;
                let a = self.stat_path(&l);
                let b = self.stat_path(&r);
                Ok(match (a, b) {
                    (Err(_), Ok(_)) => true,
                    (Ok(a), Ok(b)) => stat_after(b.mtime, a.mtime),
                    _ => false,
                })
            }
            // Lexicographic
            StrLt => {
                let l = self.expand(lhs)?;
                let r = self.expand(rhs)?;
                Ok(l.as_bytes() < r.as_bytes())
            }
            StrGt => {
                let l = self.expand(lhs)?;
                let r = self.expand(rhs)?;
                Ok(l.as_bytes() > r.as_bytes())
            }
            // Conditional glob matching always recognizes extglobs, regardless
            // of the shell's `extglob` option.
            GlobMatch | GlobNotMatch => {
                let l = self.expand(lhs)?;
                let rhs = self.expand_preserving_quotes(rhs)?;
                let re = pattern::compile_glob(
                    &rhs,
                    pattern::GlobOptions {
                        case_insensitive: self.shell_option("nocasematch"),
                        extglob: true,
                    },
                    |_| String::new(),
                )?;
                let m = pattern::matches_glob(&re, &l);
                Ok(if matches!(op, GlobMatch) { m } else { !m })
            }
            RegexMatch => {
                let l = self.expand(lhs)?;
                let rhs = self.expand_preserving_quotes(rhs)?;
                let re = pattern::compile_regex(&rhs, self.shell_option("nocasematch"), |_| {
                    String::new()
                })
                .map_err(|error| match error {
                    pattern::PatternError::Regex(error) => EvalError::BadRegex(error),
                    error => EvalError::BadPattern(error),
                })?;
                if let Some(caps) = re.captures(&l) {
                    let groups: Vec<Option<String>> = caps
                        .iter()
                        .map(|m| m.map(|m| m.as_str().to_string()))
                        .collect();
                    self.env.set_bash_rematch(&groups);
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            // Arithmetic
            ArithEq | ArithNe | ArithLt | ArithLe | ArithGt | ArithGe => {
                let l = self.expand(lhs)?;
                let a = crate::arith::eval(&l, self.env)?;
                let r = self.expand(rhs)?;
                let b = crate::arith::eval(&r, self.env)?;
                Ok(match op {
                    ArithEq => a == b,
                    ArithNe => a != b,
                    ArithLt => a < b,
                    ArithLe => a <= b,
                    ArithGt => a > b,
                    ArithGe => a >= b,
                    _ => unreachable!(),
                })
            }
        }
    }

    pub(crate) fn expand(&mut self, w: &Word) -> Result<String, EvalError> {
        let mut out = String::new();
        for p in &w.parts {
            match p {
                WordPart::Literal(s) | WordPart::Quoted(s) => out.push_str(s),
                WordPart::Var(name) | WordPart::QuotedVar(name) => {
                    if let Some(v) = self.env.var(name) {
                        out.push_str(v);
                    }
                }
                WordPart::Expansion { expansion, .. } => {
                    out.push_str(&self.eval_parameter_expansion(expansion)?);
                }
            }
        }
        Ok(out)
    }

    fn shell_option(&self, name: &str) -> bool {
        self.env
            .shell_opt(name)
            .unwrap_or(matches!(name, "patsub_replacement"))
    }

    /// Expand values while retaining whether their resulting bytes are active
    /// pattern syntax or quoted literals.
    fn expand_preserving_quotes(&mut self, word: &Word) -> Result<Word, EvalError> {
        let mut expanded = Word::default();
        for part in &word.parts {
            match part {
                WordPart::Literal(_) | WordPart::Quoted(_) => expanded.push(part.clone()),
                WordPart::Var(name) => expanded.push(WordPart::Literal(
                    self.env.var(name).unwrap_or("").to_owned(),
                )),
                WordPart::QuotedVar(name) => expanded.push(WordPart::Quoted(
                    self.env.var(name).unwrap_or("").to_owned(),
                )),
                WordPart::Expansion { expansion, quoted } => {
                    let value = self.eval_parameter_expansion_preserving_quotes(expansion)?;
                    if *quoted {
                        expanded.push(WordPart::Quoted(flatten_expanded_word(&value)));
                    } else {
                        expanded.parts.extend(value.parts);
                    }
                }
            }
        }
        Ok(expanded)
    }

    fn eval_parameter_expansion_preserving_quotes(
        &mut self,
        expansion: &ParameterExpansion,
    ) -> Result<Word, EvalError> {
        let value = self.env.var(&expansion.name).map(str::to_owned);
        match &expansion.op {
            ParameterOp::DefaultValue { test_null, word }
                if value.is_none() || (*test_null && value.as_deref() == Some("")) =>
            {
                self.expand_preserving_quotes(word)
            }
            ParameterOp::AlternateValue { test_null, word }
                if value.is_some() && (!*test_null || value.as_deref() != Some("")) =>
            {
                self.expand_preserving_quotes(word)
            }
            ParameterOp::AlternateValue { .. } => Ok(Word::default()),
            ParameterOp::DefaultValue { .. } => Ok(Word::literal(value.unwrap_or_default())),
            _ => Ok(Word::literal(self.eval_parameter_expansion(expansion)?)),
        }
    }

    fn eval_parameter_expansion(
        &mut self,
        expansion: &ParameterExpansion,
    ) -> Result<String, EvalError> {
        let value = self.env.var(&expansion.name).map(str::to_owned);
        match &expansion.op {
            ParameterOp::Length => Ok(value.unwrap_or_default().chars().count().to_string()),
            ParameterOp::DefaultValue { test_null, word } => {
                if value.is_none() || (*test_null && value.as_deref() == Some("")) {
                    self.expand(word)
                } else {
                    Ok(value.unwrap_or_default())
                }
            }
            ParameterOp::AlternateValue { test_null, word } => {
                if value.is_some() && (!*test_null || value.as_deref() != Some("")) {
                    self.expand(word)
                } else {
                    Ok(String::new())
                }
            }
            ParameterOp::Remove { kind, pattern } => {
                self.remove_pattern(&value.unwrap_or_default(), *kind, pattern)
            }
            ParameterOp::Replace {
                kind,
                pattern,
                replacement,
            } => self.replace_pattern(&value.unwrap_or_default(), *kind, pattern, replacement),
            ParameterOp::Substring { offset, length } => {
                self.substring(&value.unwrap_or_default(), offset, length.as_ref())
            }
            ParameterOp::CaseModify { kind, pattern } => {
                self.modify_case(&value.unwrap_or_default(), *kind, pattern.as_ref())
            }
        }
    }

    fn parameter_glob(
        &mut self,
        word: &Word,
        extglob_always: bool,
        nocase: bool,
    ) -> Result<pattern::CompiledGlob, EvalError> {
        let word = self.expand_preserving_quotes(word)?;
        Ok(pattern::compile_glob(
            &word,
            pattern::GlobOptions {
                case_insensitive: nocase,
                extglob: extglob_always || self.shell_option("extglob"),
            },
            |_| String::new(),
        )?)
    }

    fn remove_pattern(
        &mut self,
        value: &str,
        kind: RemoveKind,
        pattern_word: &Word,
    ) -> Result<String, EvalError> {
        let pattern = self.parameter_glob(pattern_word, false, false)?;
        let boundaries = char_boundaries(value);
        let indices: Box<dyn Iterator<Item = usize>> = match kind {
            RemoveKind::ShortestPrefix | RemoveKind::LongestSuffix => {
                Box::new(boundaries.iter().copied())
            }
            RemoveKind::LongestPrefix | RemoveKind::ShortestSuffix => {
                Box::new(boundaries.iter().rev().copied())
            }
        };
        for index in indices {
            let candidate = match kind {
                RemoveKind::ShortestPrefix | RemoveKind::LongestPrefix => &value[..index],
                RemoveKind::ShortestSuffix | RemoveKind::LongestSuffix => &value[index..],
            };
            if pattern.is_match(candidate) {
                return Ok(match kind {
                    RemoveKind::ShortestPrefix | RemoveKind::LongestPrefix => value[index..].into(),
                    RemoveKind::ShortestSuffix | RemoveKind::LongestSuffix => value[..index].into(),
                });
            }
        }
        Ok(value.to_owned())
    }

    fn replace_pattern(
        &mut self,
        value: &str,
        kind: ReplaceKind,
        pattern_word: &Word,
        replacement_word: &Word,
    ) -> Result<String, EvalError> {
        let expanded_pattern = self.expand_preserving_quotes(pattern_word)?;
        let replacement = self.expand_preserving_quotes(replacement_word)?;
        let patsub = self.shell_option("patsub_replacement");
        // An empty pattern is disabled for ordinary first/all replacement. The
        // anchored forms still insert their replacement at the selected edge.
        // A non-empty pattern such as `?(x)` can independently match empty.
        if flatten_expanded_word(&expanded_pattern).is_empty() {
            return Ok(match kind {
                ReplaceKind::Prefix => {
                    format!("{}{}", render_replacement(&replacement, "", patsub), value)
                }
                ReplaceKind::Suffix => {
                    format!("{}{}", value, render_replacement(&replacement, "", patsub))
                }
                ReplaceKind::First | ReplaceKind::All => value.to_owned(),
            });
        }
        let pattern = pattern::compile_glob(
            &expanded_pattern,
            pattern::GlobOptions {
                case_insensitive: self.shell_option("nocasematch"),
                extglob: self.shell_option("extglob"),
            },
            |_| String::new(),
        )?;
        let boundaries = char_boundaries(value);

        match kind {
            ReplaceKind::Prefix => {
                if value.is_empty() {
                    return Ok(value.to_owned());
                }
                if let Some(end) = longest_match_from(&pattern, value, 0, &boundaries) {
                    return Ok(format!(
                        "{}{}",
                        render_replacement(&replacement, &value[..end], patsub),
                        &value[end..]
                    ));
                }
            }
            ReplaceKind::Suffix => {
                for &start in &boundaries {
                    if pattern.is_match(&value[start..]) {
                        return Ok(format!(
                            "{}{}",
                            &value[..start],
                            render_replacement(&replacement, &value[start..], patsub)
                        ));
                    }
                }
            }
            ReplaceKind::First => {
                if let Some((start, end)) = leftmost_longest(&pattern, value, &boundaries, 0) {
                    return Ok(format!(
                        "{}{}{}",
                        &value[..start],
                        render_replacement(&replacement, &value[start..end], patsub),
                        &value[end..]
                    ));
                }
            }
            ReplaceKind::All => {
                let mut output = String::new();
                let mut position = 0usize;
                while position < value.len() {
                    let Some((start, end)) =
                        leftmost_longest(&pattern, value, &boundaries, position)
                    else {
                        output.push_str(&value[position..]);
                        break;
                    };
                    output.push_str(&value[position..start]);
                    output.push_str(&render_replacement(
                        &replacement,
                        &value[start..end],
                        patsub,
                    ));
                    if start == end {
                        let next = next_char_boundary(value, end);
                        output.push_str(&value[end..next]);
                        position = next;
                    } else {
                        position = end;
                    }
                }
                return Ok(output);
            }
        }
        Ok(value.to_owned())
    }

    fn substring(
        &mut self,
        value: &str,
        offset_word: &Word,
        length_word: Option<&Word>,
    ) -> Result<String, EvalError> {
        let offset_expression = self.expand(offset_word)?;
        let offset = crate::arith::eval(&offset_expression, self.env)?;
        let length = if let Some(word) = length_word {
            let expression = self.expand(word)?;
            Some(crate::arith::eval(&expression, self.env)?)
        } else {
            None
        };
        let chars: Vec<char> = value.chars().collect();
        let count = chars.len() as i128;
        let start = if offset < 0 {
            count + i128::from(offset)
        } else {
            i128::from(offset)
        };
        if start < 0 || start > count {
            return Ok(String::new());
        }
        let end = match length {
            None => count,
            Some(length) if length >= 0 => (start + i128::from(length)).min(count),
            Some(length) => count + i128::from(length),
        };
        if end < start {
            return Err(EvalError::InvalidArith("substring expression < 0".into()));
        }
        Ok(chars[start as usize..end.min(count) as usize]
            .iter()
            .collect())
    }

    fn modify_case(
        &mut self,
        value: &str,
        kind: CaseModifyKind,
        pattern_word: Option<&Word>,
    ) -> Result<String, EvalError> {
        let default_pattern = Word::literal("?");
        let pattern = if let Some(pattern_word) = pattern_word {
            let expanded = self.expand_preserving_quotes(pattern_word)?;
            let expanded_is_unquoted_empty =
                flatten_expanded_word(&expanded).is_empty() && !expanded.any_quoted();
            let pattern_word = if expanded_is_unquoted_empty {
                &default_pattern
            } else {
                &expanded
            };
            pattern::compile_glob(
                pattern_word,
                pattern::GlobOptions {
                    case_insensitive: false,
                    extglob: true,
                },
                |_| String::new(),
            )?
        } else {
            self.parameter_glob(&default_pattern, true, false)?
        };
        let modify_all = matches!(kind, CaseModifyKind::UpperAll | CaseModifyKind::LowerAll);
        let uppercase = matches!(kind, CaseModifyKind::UpperFirst | CaseModifyKind::UpperAll);
        let mut output = String::new();
        for (index, ch) in value.chars().enumerate() {
            if (modify_all || index == 0) && pattern.is_match(&ch.to_string()) {
                if uppercase {
                    output.extend(ch.to_uppercase());
                } else {
                    output.extend(ch.to_lowercase());
                }
            } else {
                output.push(ch);
            }
        }
        Ok(output)
    }
}

fn stat_kind_is(stat: &std::io::Result<FileStat>, want: FileKind) -> bool {
    matches!(stat, Ok(s) if s.kind == want)
}

fn stat_mode_bit(stat: &std::io::Result<FileStat>, mask: u32) -> bool {
    matches!(stat, Ok(s) if s.mode & mask != 0)
}

fn stat_after(a: (i64, i64), b: (i64, i64)) -> bool {
    a > b
}

fn char_boundaries(value: &str) -> Vec<usize> {
    value
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(value.len()))
        .collect()
}

fn next_char_boundary(value: &str, index: usize) -> usize {
    value[index..]
        .char_indices()
        .nth(1)
        .map_or(value.len(), |(next, _)| index + next)
}

fn longest_match_from(
    pattern: &pattern::CompiledGlob,
    value: &str,
    start: usize,
    boundaries: &[usize],
) -> Option<usize> {
    boundaries
        .iter()
        .rev()
        .copied()
        .take_while(|end| *end >= start)
        .find(|end| pattern.is_match(&value[start..*end]))
}

fn leftmost_longest(
    pattern: &pattern::CompiledGlob,
    value: &str,
    boundaries: &[usize],
    minimum_start: usize,
) -> Option<(usize, usize)> {
    boundaries
        .iter()
        .copied()
        .filter(|start| *start >= minimum_start && *start < value.len())
        .find_map(|start| {
            longest_match_from(pattern, value, start, boundaries).map(|end| (start, end))
        })
}

fn render_replacement(replacement: &Word, matched: &str, patsub: bool) -> String {
    let mut output = String::new();
    for part in &replacement.parts {
        match part {
            WordPart::Literal(text) if patsub => {
                for ch in text.chars() {
                    if ch == '&' {
                        output.push_str(matched);
                    } else {
                        output.push(ch);
                    }
                }
            }
            WordPart::Literal(text) | WordPart::Quoted(text) => output.push_str(text),
            WordPart::Var(_) | WordPart::QuotedVar(_) | WordPart::Expansion { .. } => {
                unreachable!("replacement words are fully expanded")
            }
        }
    }
    output
}

fn flatten_expanded_word(word: &Word) -> String {
    let mut output = String::new();
    for part in &word.parts {
        match part {
            WordPart::Literal(text) | WordPart::Quoted(text) => output.push_str(text),
            WordPart::Var(_) | WordPart::QuotedVar(_) | WordPart::Expansion { .. } => {
                unreachable!("word is fully expanded")
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::MapEnv;
    use crate::fs_abs::StdFs;
    use crate::parse::{ParseOptions, parse, parse_with_options};
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn run(input: &str, env: &mut MapEnv) -> bool {
        let expr = parse(input).expect("parse");
        let fs = StdFs;
        Evaluator::new(env, &fs).eval(&expr).expect("eval")
    }

    fn run_default(input: &str) -> bool {
        run(input, &mut MapEnv::new())
    }

    fn run_with_punctuation_vars(input: &str, env: &mut MapEnv) -> bool {
        let options = ParseOptions::default().punctuation_variables(true);
        let expr = parse_with_options(input, options).expect("parse");
        let fs = StdFs;
        Evaluator::new(env, &fs).eval(&expr).expect("eval")
    }

    #[test]
    fn string_nonempty_default() {
        let mut env = MapEnv::new().with_var("x", "hi");
        assert!(run("$x", &mut env));
        assert!(run("-n $x", &mut env));
        assert!(!run("-z $x", &mut env));
    }

    #[test]
    fn string_empty_for_unset() {
        let mut env = MapEnv::new();
        assert!(!run("$x", &mut env));
        assert!(run("-z $x", &mut env));
    }

    #[test]
    fn custom_braced_vars_expand() {
        let mut env = MapEnv::new()
            .with_var("", "path")
            .with_var("/", "basename")
            .with_var("//", "parent")
            .with_var(".", "path-no-ext")
            .with_var("/.", "basename-no-ext");
        assert!(run_with_punctuation_vars("${} == path", &mut env));
        assert!(run_with_punctuation_vars("${/} == basename", &mut env));
        assert!(run_with_punctuation_vars("${//} == parent", &mut env));
        assert!(run_with_punctuation_vars("${.} == path-no-ext", &mut env));
        assert!(run_with_punctuation_vars(
            "${/.} == basename-no-ext",
            &mut env
        ));
    }

    #[test]
    fn lex_compare_lt_gt() {
        assert!(run_default("apple < banana"));
        assert!(!run_default("banana < apple"));
        assert!(run_default("banana > apple"));
    }

    #[test]
    fn arith_lt_le_gt_ge_eq_ne() {
        let mut env = MapEnv::new().with_var("x", "5");
        assert!(run("$x -lt 10", &mut env));
        assert!(run("$x -le 5", &mut env));
        assert!(run("$x -gt 4", &mut env));
        assert!(run("$x -ge 5", &mut env));
        assert!(run("$x -eq 5", &mut env));
        assert!(run("$x -ne 6", &mut env));
    }

    #[test]
    fn arith_empty_is_zero() {
        let mut env = MapEnv::new();
        assert!(run("$x -eq 0", &mut env));
    }

    #[test]
    fn arith_bare_name_lookup() {
        // `[[ x -lt 10 ]]` — x is a variable name in arithmetic context.
        let mut env = MapEnv::new().with_var("x", "3");
        assert!(run("x -lt 10", &mut env));
    }

    #[test]
    fn file_test_regular_and_dir() {
        let f = NamedTempFile::new().unwrap();
        let path = f.path().to_str().unwrap().to_string();
        let mut env = MapEnv::new().with_var("p", &path);
        assert!(run("-f $p", &mut env));
        assert!(run("-e $p", &mut env));
        assert!(!run("-d $p", &mut env));
    }

    #[test]
    fn file_test_nonexistent() {
        let mut env = MapEnv::new();
        assert!(!run("-e /this/path/does/not/exist/abc123", &mut env));
        assert!(!run("-f /this/path/does/not/exist/abc123", &mut env));
    }

    #[test]
    fn file_test_empty_path() {
        let mut env = MapEnv::new();
        assert!(!run("-e ''", &mut env));
        assert!(!run("-f ''", &mut env));
        assert!(!run("-d ''", &mut env));
        assert!(!run("-r ''", &mut env));
        assert!(!run("-w ''", &mut env));
        assert!(!run("-x ''", &mut env));
    }

    #[test]
    fn file_comparison_empty_path() {
        let f = NamedTempFile::new().unwrap();
        let path = f.path().to_str().unwrap().to_string();
        let mut env = MapEnv::new().with_var("p", &path);
        assert!(!run("$p -ef ''", &mut env));
        assert!(run("$p -nt ''", &mut env));
        assert!(run("'' -ot $p", &mut env));
    }

    #[test]
    fn file_size_nonempty() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "hello").unwrap();
        let path = f.path().to_str().unwrap().to_string();
        let mut env = MapEnv::new().with_var("p", &path);
        assert!(run("-s $p", &mut env));
    }

    #[test]
    fn v_unary_var_set() {
        let mut env = MapEnv::new().with_var("HOME", "/x");
        assert!(run("-v HOME", &mut env));
        assert!(!run("-v NOPE", &mut env));
    }

    #[test]
    fn shell_opt() {
        let mut env = MapEnv::new().with_option("nocasematch", true);
        assert!(run("-o nocasematch", &mut env));
        assert!(!run("-o noclobber", &mut env));
    }

    #[test]
    fn glob_match() {
        let mut env = MapEnv::new().with_var("f", "report.txt");
        assert!(run("$f == *.txt", &mut env));
        assert!(!run("$f == *.md", &mut env));
        assert!(run("$f != *.md", &mut env));
    }

    #[test]
    fn glob_quoted_metas_are_literal() {
        // Quoted "*.txt" pattern should match only the literal "*.txt".
        let mut env = MapEnv::new();
        assert!(run(r#"'*.txt' == "*.txt""#, &mut env));
        assert!(!run(r#"foo.txt == "*.txt""#, &mut env));
    }

    #[test]
    fn glob_nocasematch() {
        let mut env = MapEnv::new()
            .with_var("f", "Report.TXT")
            .with_option("nocasematch", true);
        assert!(run("$f == *.txt", &mut env));
    }

    #[test]
    fn regex_basic() {
        let mut env = MapEnv::new().with_var("line", "  ab cd");
        assert!(run(r"$line =~ ^[[:space:]]*(a)?b", &mut env));
    }

    #[test]
    fn regex_populates_rematch() {
        let mut env = MapEnv::new().with_var("v", "user-42");
        let r = run(r"$v =~ ^([a-z]+)-([0-9]+)$", &mut env);
        assert!(r);
        assert_eq!(env.last_rematch.len(), 3);
        assert_eq!(env.last_rematch[0].as_deref(), Some("user-42"));
        assert_eq!(env.last_rematch[1].as_deref(), Some("user"));
        assert_eq!(env.last_rematch[2].as_deref(), Some("42"));
    }
}
