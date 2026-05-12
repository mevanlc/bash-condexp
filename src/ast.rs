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
        self.parts.iter().any(|p| matches!(p, WordPart::Quoted(_)))
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
}
