//! Error types for parsing and evaluation.

use std::io;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("unexpected token `{token}` at byte {pos}")]
    UnexpectedToken { token: String, pos: usize },

    #[error("unexpected end of input")]
    UnexpectedEof,

    #[error("unterminated quoted string starting at byte {pos}")]
    UnterminatedString { pos: usize },

    #[error("expected `]]` to close `[[` opened at byte {pos}")]
    UnterminatedDoubleBracket { pos: usize },

    #[error("expected a word after `{op}`")]
    ExpectedWord { op: String },

    #[error("the `=~` operator requires a right-hand side")]
    RegexMissingRhs,

    #[error("unrecognized conditional operator `{token}`")]
    UnknownOperator { token: String },

    #[error("invalid `-v` variable subscript: `{raw}`")]
    InvalidSubscript { raw: String },

    #[error("parentheses are not supported; use `[[ ... ]]` to group")]
    ParensNotSupported,

    #[error("`-a` / `-o` combinators are not supported; use `&&` / `||`")]
    LegacyCombinatorNotSupported,
}

#[derive(Debug, Error)]
pub enum EvalError {
    #[error("invalid arithmetic operand: `{0}`")]
    InvalidArith(String),

    #[error("invalid regular expression: {0}")]
    BadRegex(#[from] regex::Error),

    #[error("file I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("variable `{0}` refers to unsupported expansion (cmd/arith subst not implemented)")]
    UnsupportedExpansion(String),

    #[error("invalid file descriptor for `-t`: `{0}`")]
    InvalidFd(String),
}
