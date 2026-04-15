//! Parse and evaluate bash `[[ ... ]]` conditional expressions.
//!
//! See `aidocs/PLAN.md` for the design and
//! `devdocs/BASH-CONDITIONAL-EXPRESSIONS.md` for the reference grammar.

pub mod ast;
pub mod error;
pub mod lex;

pub use ast::{BinaryOp, Expr, Primary, UnaryOp, Word, WordPart};
pub use error::{EvalError, ParseError};
