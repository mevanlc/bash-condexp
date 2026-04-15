//! Parse and evaluate bash `[[ ... ]]` conditional expressions.
//!
//! See `aidocs/PLAN.md` for the design and
//! `devdocs/BASH-CONDITIONAL-EXPRESSIONS.md` for the reference grammar.

pub mod ast;
pub mod env;
pub mod error;
pub mod fs_abs;
pub mod lex;
pub mod parse;

pub use env::{Env, MapEnv, StdEnv};
pub use fs_abs::{AccessMode, FileKind, FileStat, FileSystem, StdFs};
pub use parse::parse;

pub use ast::{BinaryOp, Expr, Primary, UnaryOp, Word, WordPart};
pub use error::{EvalError, ParseError};
