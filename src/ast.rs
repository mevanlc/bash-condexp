//! AST types for bash conditional expressions.

use std::fmt;

/// A conditional expression tree. Combinators short-circuit during evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Primary(Primary),
}

/// A single primary test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Primary {
    Unary {
        op: UnaryOp,
        arg: Word,
    },
    Binary {
        op: BinaryOp,
        lhs: Word,
        rhs: Word,
    },
    /// Bare word: `[[ $x ]]` is equivalent to `-n $x`.
    StringNonEmpty(Word),
}

/// Unary operators. The operator token uniquely determines the kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    // File existence / type
    FileExists,    // -a, -e
    FileRegular,   // -f
    FileDir,       // -d
    FileBlock,     // -b
    FileChar,      // -c
    FileSymlink,   // -h, -L
    FileNamedPipe, // -p
    FileSocket,    // -S
    // File permissions / attributes
    FileReadable,        // -r
    FileWritable,        // -w
    FileExecutable,      // -x
    FileNonEmpty,        // -s
    FileSetUid,          // -u
    FileSetGid,          // -g
    FileSticky,          // -k
    FileOwnedByUid,      // -O
    FileOwnedByGid,      // -G
    FileNewerThanAccess, // -N
    // Misc
    FdIsTty,        // -t
    StringEmpty,    // -z
    StringNonEmpty, // -n
    VarSet,         // -v
    VarIsNameRef,   // -R
    ShellOptSet,    // -o
}

impl UnaryOp {
    pub fn token(self) -> &'static str {
        use UnaryOp::*;
        match self {
            FileExists => "-e",
            FileRegular => "-f",
            FileDir => "-d",
            FileBlock => "-b",
            FileChar => "-c",
            FileSymlink => "-h",
            FileNamedPipe => "-p",
            FileSocket => "-S",
            FileReadable => "-r",
            FileWritable => "-w",
            FileExecutable => "-x",
            FileNonEmpty => "-s",
            FileSetUid => "-u",
            FileSetGid => "-g",
            FileSticky => "-k",
            FileOwnedByUid => "-O",
            FileOwnedByGid => "-G",
            FileNewerThanAccess => "-N",
            FdIsTty => "-t",
            StringEmpty => "-z",
            StringNonEmpty => "-n",
            VarSet => "-v",
            VarIsNameRef => "-R",
            ShellOptSet => "-o",
        }
    }

    /// Parse a token string (without quoting) into an operator.
    pub fn from_token(s: &str) -> Option<Self> {
        use UnaryOp::*;
        Some(match s {
            "-a" | "-e" => FileExists,
            "-f" => FileRegular,
            "-d" => FileDir,
            "-b" => FileBlock,
            "-c" => FileChar,
            "-h" | "-L" => FileSymlink,
            "-p" => FileNamedPipe,
            "-S" => FileSocket,
            "-r" => FileReadable,
            "-w" => FileWritable,
            "-x" => FileExecutable,
            "-s" => FileNonEmpty,
            "-u" => FileSetUid,
            "-g" => FileSetGid,
            "-k" => FileSticky,
            "-O" => FileOwnedByUid,
            "-G" => FileOwnedByGid,
            "-N" => FileNewerThanAccess,
            "-t" => FdIsTty,
            "-z" => StringEmpty,
            "-n" => StringNonEmpty,
            "-v" => VarSet,
            "-R" => VarIsNameRef,
            "-o" => ShellOptSet,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    // File comparisons
    FileSameInode, // -ef
    FileNewer,     // -nt
    FileOlder,     // -ot
    // String
    StrLt,        // < (lexicographic)
    StrGt,        // > (lexicographic)
    GlobMatch,    // == / = (pattern, per [[)
    GlobNotMatch, // !=     (pattern, per [[)
    RegexMatch,   // =~
    // Arithmetic
    ArithEq, // -eq
    ArithNe, // -ne
    ArithLt, // -lt
    ArithLe, // -le
    ArithGt, // -gt
    ArithGe, // -ge
}

impl BinaryOp {
    pub fn token(self) -> &'static str {
        use BinaryOp::*;
        match self {
            FileSameInode => "-ef",
            FileNewer => "-nt",
            FileOlder => "-ot",
            StrLt => "<",
            StrGt => ">",
            GlobMatch => "==",
            GlobNotMatch => "!=",
            RegexMatch => "=~",
            ArithEq => "-eq",
            ArithNe => "-ne",
            ArithLt => "-lt",
            ArithLe => "-le",
            ArithGt => "-gt",
            ArithGe => "-ge",
        }
    }

    pub fn from_token(s: &str) -> Option<Self> {
        use BinaryOp::*;
        Some(match s {
            "-ef" => FileSameInode,
            "-nt" => FileNewer,
            "-ot" => FileOlder,
            "==" | "=" => GlobMatch,
            "!=" => GlobNotMatch,
            "<" => StrLt,
            ">" => StrGt,
            "=~" => RegexMatch,
            "-eq" => ArithEq,
            "-ne" => ArithNe,
            "-lt" => ArithLt,
            "-le" => ArithLe,
            "-gt" => ArithGt,
            "-ge" => ArithGe,
            _ => return None,
        })
    }
}

/// A parsed word — a sequence of parts that, at evaluation time, expand into
/// a single string. Per-part quoting is preserved so the evaluator can honor
/// bash's rules for pattern/regex literalness on the RHS of `==` / `=~`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Word {
    pub parts: Vec<WordPart>,
}

impl Word {
    pub fn literal(s: impl Into<String>) -> Self {
        Word {
            parts: vec![WordPart::Literal(s.into())],
        }
    }

    /// Was any part of the word quoted? Used by `==` / `=~` to decide
    /// whether the pattern should be treated literally.
    pub fn any_quoted(&self) -> bool {
        self.parts.iter().any(|part| {
            matches!(
                part,
                WordPart::Quoted(_)
                    | WordPart::QuotedVar(_)
                    | WordPart::Expansion { quoted: true, .. }
            )
        })
    }

    pub fn push(&mut self, part: WordPart) {
        self.parts.push(part);
    }
}

impl fmt::Display for Word {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for p in &self.parts {
            match p {
                WordPart::Literal(s) | WordPart::Quoted(s) => f.write_str(s)?,
                WordPart::Var(name) | WordPart::QuotedVar(name) => write!(f, "${{{}}}", name)?,
                WordPart::Expansion { expansion, .. } => write!(f, "{expansion}")?,
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WordPart {
    /// Unquoted literal text.
    Literal(String),
    /// Quoted literal text (single- or double-quoted). Quoting affects how
    /// the bytes are treated inside pattern/regex RHS but not their values.
    Quoted(String),
    /// An unquoted `$var` / `${var}` reference. The expanded value retains
    /// pattern/regex metacharacter specialness on the RHS of `==` / `=~`.
    Var(String),
    /// A `$var` / `${var}` reference that appeared inside double quotes.
    /// The expanded value is matched **literally** on the RHS of `==` /
    /// `=~`, per bash's "quoted variable expansion is literal" rule.
    QuotedVar(String),
    /// A braced parameter expansion that transforms or conditionally replaces
    /// the parameter value.
    Expansion {
        expansion: ParameterExpansion,
        /// Whether the complete expansion appeared in double quotes. This
        /// controls the literalness of its result when the containing word is
        /// later used as a glob or regular-expression pattern.
        quoted: bool,
    },
}

/// A structured `${parameter...}` expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterExpansion {
    pub name: String,
    pub op: ParameterOp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParameterOp {
    Length,
    Remove {
        kind: RemoveKind,
        pattern: Word,
    },
    Replace {
        kind: ReplaceKind,
        pattern: Word,
        replacement: Word,
    },
    Substring {
        offset: Word,
        length: Option<Word>,
    },
    CaseModify {
        kind: CaseModifyKind,
        pattern: Option<Word>,
    },
    DefaultValue {
        test_null: bool,
        word: Word,
    },
    AlternateValue {
        test_null: bool,
        word: Word,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemoveKind {
    ShortestPrefix,
    LongestPrefix,
    ShortestSuffix,
    LongestSuffix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReplaceKind {
    First,
    All,
    Prefix,
    Suffix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaseModifyKind {
    UpperFirst,
    UpperAll,
    LowerFirst,
    LowerAll,
}

impl fmt::Display for ParameterExpansion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if matches!(self.op, ParameterOp::Length) {
            return write!(f, "${{#{}}}", self.name);
        }
        write!(f, "${{{}", self.name)?;
        match &self.op {
            ParameterOp::Length => unreachable!(),
            ParameterOp::Remove { kind, pattern } => {
                let op = match kind {
                    RemoveKind::ShortestPrefix => "#",
                    RemoveKind::LongestPrefix => "##",
                    RemoveKind::ShortestSuffix => "%",
                    RemoveKind::LongestSuffix => "%%",
                };
                write!(f, "{op}{pattern}")?;
            }
            ParameterOp::Replace {
                kind,
                pattern,
                replacement,
            } => {
                let op = match kind {
                    ReplaceKind::First => "/",
                    ReplaceKind::All => "//",
                    ReplaceKind::Prefix => "/#",
                    ReplaceKind::Suffix => "/%",
                };
                write!(f, "{op}{pattern}/{replacement}")?;
            }
            ParameterOp::Substring { offset, length } => {
                write!(f, ":{offset}")?;
                if let Some(length) = length {
                    write!(f, ":{length}")?;
                }
            }
            ParameterOp::CaseModify { kind, pattern } => {
                let op = match kind {
                    CaseModifyKind::UpperFirst => "^",
                    CaseModifyKind::UpperAll => "^^",
                    CaseModifyKind::LowerFirst => ",",
                    CaseModifyKind::LowerAll => ",,",
                };
                write!(f, "{op}")?;
                if let Some(pattern) = pattern {
                    write!(f, "{pattern}")?;
                }
            }
            ParameterOp::DefaultValue { test_null, word } => {
                write!(f, "{}{}", if *test_null { ":-" } else { "-" }, word)?;
            }
            ParameterOp::AlternateValue { test_null, word } => {
                write!(f, "{}{}", if *test_null { ":+" } else { "+" }, word)?;
            }
        }
        write!(f, "}}")
    }
}
