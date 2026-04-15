//! Evaluator for parsed conditional expressions.
//!
//! Combinators (`&&`, `||`, `!`) live in this module too, but the actual
//! glob and regex matching for `==`/`!=`/`=~` are split into
//! [`crate::pattern`] and applied here once those modules are in place.

use std::path::Path;

use crate::ast::{BinaryOp, Expr, Primary, UnaryOp, Word, WordPart};
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
                let s = self.expand(arg);
                self.eval_unary(*op, &s)
            }
            Primary::Binary { op, lhs, rhs } => {
                self.eval_binary(*op, lhs, rhs)
            }
            Primary::StringNonEmpty(w) => {
                let s = self.expand(w);
                Ok(!s.is_empty())
            }
        }
    }

    fn eval_unary(&self, op: UnaryOp, arg: &str) -> Result<bool, EvalError> {
        use UnaryOp::*;
        Ok(match op {
            // Existence / type
            FileExists => self.fs.stat(Path::new(arg)).is_ok(),
            FileRegular => stat_kind_is(&self.fs.stat(Path::new(arg)), FileKind::Regular),
            FileDir => stat_kind_is(&self.fs.stat(Path::new(arg)), FileKind::Directory),
            FileBlock => stat_kind_is(&self.fs.stat(Path::new(arg)), FileKind::BlockDevice),
            FileChar => stat_kind_is(&self.fs.stat(Path::new(arg)), FileKind::CharDevice),
            FileSymlink => stat_kind_is(&self.fs.lstat(Path::new(arg)), FileKind::Symlink),
            FileNamedPipe => stat_kind_is(&self.fs.stat(Path::new(arg)), FileKind::NamedPipe),
            FileSocket => stat_kind_is(&self.fs.stat(Path::new(arg)), FileKind::Socket),

            // Permissions / attributes
            FileReadable => self.fs.access(Path::new(arg), AccessMode::Read),
            FileWritable => self.fs.access(Path::new(arg), AccessMode::Write),
            FileExecutable => self.fs.access(Path::new(arg), AccessMode::Execute),
            FileNonEmpty => self.fs.stat(Path::new(arg)).map(|s| s.size > 0).unwrap_or(false),
            FileSetUid => stat_mode_bit(&self.fs.stat(Path::new(arg)), 0o4000),
            FileSetGid => stat_mode_bit(&self.fs.stat(Path::new(arg)), 0o2000),
            FileSticky => stat_mode_bit(&self.fs.stat(Path::new(arg)), 0o1000),
            FileOwnedByUid => self
                .fs
                .stat(Path::new(arg))
                .map(|s| s.uid == self.fs.effective_uid())
                .unwrap_or(false),
            FileOwnedByGid => self
                .fs
                .stat(Path::new(arg))
                .map(|s| s.gid == self.fs.effective_gid())
                .unwrap_or(false),
            FileNewerThanAccess => self
                .fs
                .stat(Path::new(arg))
                .map(|s| stat_after(s.mtime, s.atime))
                .unwrap_or(false),

            // Misc
            FdIsTty => {
                let fd: i32 = arg.parse().map_err(|_| EvalError::InvalidFd(arg.to_string()))?;
                self.fs.is_tty(fd)
            }
            StringEmpty => arg.is_empty(),
            StringNonEmpty => !arg.is_empty(),
            VarSet => self.eval_var_set(arg),
            VarIsNameRef => self.env.is_nameref(arg),
            ShellOptSet => self.env.shell_opt(arg),
        })
    }

    fn eval_var_set(&self, raw: &str) -> bool {
        // Split optional `name[subscript]` suffix.
        if let Some(open) = raw.find('[') {
            if raw.ends_with(']') {
                let name = &raw[..open];
                let sub = &raw[open + 1..raw.len() - 1];
                return self.env.array_element_set(name, sub);
            }
        }
        // Bare name → either a scalar set, or the [0] element of an array.
        self.env.var(raw).is_some() || self.env.array_element_set(raw, "0")
    }

    fn eval_binary(&mut self, op: BinaryOp, lhs: &Word, rhs: &Word) -> Result<bool, EvalError> {
        use BinaryOp::*;
        match op {
            // File comparisons
            FileSameInode => {
                let l = self.expand(lhs);
                let r = self.expand(rhs);
                Ok(match (self.fs.stat(Path::new(&l)), self.fs.stat(Path::new(&r))) {
                    (Ok(a), Ok(b)) => a.dev == b.dev && a.ino == b.ino,
                    _ => false,
                })
            }
            FileNewer => {
                let l = self.expand(lhs);
                let r = self.expand(rhs);
                let a = self.fs.stat(Path::new(&l));
                let b = self.fs.stat(Path::new(&r));
                Ok(match (a, b) {
                    (Ok(_), Err(_)) => true,
                    (Ok(a), Ok(b)) => stat_after(a.mtime, b.mtime),
                    _ => false,
                })
            }
            FileOlder => {
                let l = self.expand(lhs);
                let r = self.expand(rhs);
                let a = self.fs.stat(Path::new(&l));
                let b = self.fs.stat(Path::new(&r));
                Ok(match (a, b) {
                    (Err(_), Ok(_)) => true,
                    (Ok(a), Ok(b)) => stat_after(b.mtime, a.mtime),
                    _ => false,
                })
            }
            // Lexicographic
            StrLt => {
                let l = self.expand(lhs);
                let r = self.expand(rhs);
                Ok(l.as_bytes() < r.as_bytes())
            }
            StrGt => {
                let l = self.expand(lhs);
                let r = self.expand(rhs);
                Ok(l.as_bytes() > r.as_bytes())
            }
            // Pattern (extglob-lite) and regex matching.
            GlobMatch | GlobNotMatch => {
                let l = self.expand(lhs);
                let nocase = self.env.shell_opt("nocasematch");
                let env = &*self.env;
                let re = pattern::compile_glob(rhs, nocase, |name| {
                    env.var(name).unwrap_or("").to_string()
                })?;
                let m = pattern::matches_glob(&re, &l);
                Ok(if matches!(op, GlobMatch) { m } else { !m })
            }
            RegexMatch => {
                let l = self.expand(lhs);
                let nocase = self.env.shell_opt("nocasematch");
                // Borrow env immutably to compile, then drop the borrow
                // before potentially calling set_bash_rematch.
                let re = {
                    let env = &*self.env;
                    pattern::compile_regex(rhs, nocase, |name| {
                        env.var(name).unwrap_or("").to_string()
                    })?
                };
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
                let l = self.expand(lhs);
                let r = self.expand(rhs);
                let a = self.parse_arith(&l)?;
                let b = self.parse_arith(&r)?;
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

    /// v1 arithmetic operand: empty → 0; integer literal (signed decimal);
    /// or a bare variable name whose value is itself a literal int.
    fn parse_arith(&self, s: &str) -> Result<i64, EvalError> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Ok(0);
        }
        if let Ok(n) = trimmed.parse::<i64>() {
            return Ok(n);
        }
        // Treat as variable name and recurse once.
        if is_ident(trimmed) {
            let value = self.env.var(trimmed).unwrap_or("");
            if value.trim().is_empty() {
                return Ok(0);
            }
            return value
                .trim()
                .parse::<i64>()
                .map_err(|_| EvalError::InvalidArith(value.to_string()));
        }
        Err(EvalError::InvalidArith(s.to_string()))
    }

    pub(crate) fn expand(&self, w: &Word) -> String {
        let mut out = String::new();
        for p in &w.parts {
            match p {
                WordPart::Literal(s) | WordPart::Quoted(s) => out.push_str(s),
                WordPart::Var(name) => {
                    if let Some(v) = self.env.var(name) {
                        out.push_str(v);
                    }
                }
            }
        }
        out
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

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::MapEnv;
    use crate::fs_abs::StdFs;
    use crate::parse::parse;
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
