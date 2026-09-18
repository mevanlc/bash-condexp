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

    #[error("unterminated parameter expansion starting at byte {pos}")]
    UnterminatedParameterExpansion { pos: usize },

    #[error("invalid parameter expansion `{raw}` at byte {pos}: {reason}")]
    InvalidParameterExpansion {
        raw: String,
        pos: usize,
        reason: String,
    },

    #[error("unsupported parameter expansion `{operator}` at byte {pos}")]
    UnsupportedParameterExpansion { operator: String, pos: usize },

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

    #[error("arithmetic division by zero")]
    DivisionByZero,

    #[error("arithmetic exponent is negative")]
    NegativeExponent,

    #[error("arithmetic variable recursion limit exceeded while expanding `{0}`")]
    ArithmeticRecursion(String),

    #[error("the environment does not support arithmetic assignment to `{0}`")]
    ArithmeticAssignmentUnsupported(String),

    #[error("invalid glob pattern: {0}")]
    BadPattern(#[from] crate::pattern::PatternError),

    #[error("invalid regular expression: {0}")]
    BadRegex(#[from] regex::Error),

    #[error("file I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("variable `{0}` refers to unsupported expansion (cmd/arith subst not implemented)")]
    UnsupportedExpansion(String),

    #[error("invalid file descriptor for `-t`: `{0}`")]
    InvalidFd(String),
}
