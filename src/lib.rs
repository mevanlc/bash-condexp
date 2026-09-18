//! Parse and evaluate bash conditional expressions — the grammar accepted
//! by `[[ ... ]]`.
//!
//! ## Quick start
//!
//! ```
//! use bash_condexp::{Evaluator, MapEnv, StdFs, parse};
//!
//! let mut env = MapEnv::new().with_var("name", "alice");
//! let fs = StdFs;
//!
//! let expr = parse("$name == al* && -e Cargo.toml").unwrap();
//! let truth = Evaluator::new(&mut env, &fs).eval(&expr).unwrap();
//! assert!(truth);
//! ```
//!
//! ## Input forms
//!
//! The outer `[[ ... ]]` are optional, and may also wrap any subexpression:
//!
//! ```
//! # use bash_condexp::parse;
//! parse("-f Cargo.toml").unwrap();                            // implicit
//! parse("[[ -f Cargo.toml ]]").unwrap();                      // explicit
//! parse("[[ -f Cargo.toml ]] && [[ -d src ]]").unwrap();      // composed
//! parse("-f Cargo.toml && -d src").unwrap();                  // implicit
//! parse("[[ -f Cargo.toml && -d src ]]").unwrap();            // grouped
//! ```
//!
//! [`parse`] uses the bash-compatible parameter-name grammar. Empty and
//! ASCII-punctuation-only names used by fd-style placeholders can be enabled
//! per parse:
//!
//! ```
//! # use bash_condexp::{ParseOptions, parse_with_options};
//! let options = ParseOptions::default().punctuation_variables(true);
//! parse_with_options("${/.} == README", options).unwrap();
//! ```
//!
//! There is no `( ... )` grouping (use `[[ ... ]]` instead) and no
//! `-a` / `-o` legacy combinators (use `&&` / `||`).
//!
//! ## Supported primaries
//!
//! - **Existence / type**: `-a` `-e` `-f` `-d` `-b` `-c` `-h` `-L` `-p` `-S`
//! - **Permissions / attrs**: `-r` `-w` `-x` `-s` `-u` `-g` `-k` `-O` `-G` `-N`
//! - **Other unary**: `-t` `-z` `-n` `-v` `-R` `-o`
//! - **File comparisons**: `-ef` `-nt` `-ot`
//! - **Strings**: `==` `=` `!=` `<` `>` (`==` / `!=` are pattern-matching
//!   per bash; `<` / `>` are byte-wise lexicographic in v1)
//! - **Arithmetic**: `-eq` `-ne` `-lt` `-le` `-gt` `-ge`
//!   (full scalar integer expressions, including mutation through
//!   [`Env::set_var`])
//! - **Regex**: `=~` (POSIX-ERE-ish via the `regex` crate; populates
//!   `BASH_REMATCH` through [`Env::set_bash_rematch`])
//! - **Parameter transformations**: length, substring, prefix/suffix removal,
//!   replacement, case modification, and pure default/alternate values
//!
//! ## Combinators
//!
//! - `!` (highest)
//! - `&&`
//! - `||` (lowest)
//!
//! Both `&&` and `||` short-circuit.
//!
//! ## Limitations (v1)
//!
//! - No command substitution `$(...)`, general arithmetic expansion
//!   `$((...))`, process substitution, arrays, or indirect parameter expansion.
//! - Assignment/error parameter forms (`:=`, `:?`, `=`, `?`) are rejected;
//!   the pure `-`, `:-`, `+`, and `:+` forms are supported.
//! - `<` and `>` use byte comparison, not locale-aware `strcoll`.
//! - `=~` uses the Rust `regex` crate, which is close to POSIX ERE but
//!   differs in a few edge cases — notably it rejects lone `\x` (it
//!   reserves `\x..` for hex escapes), where bash treats `\x` as literal
//!   `x`. POSIX bracket equivalence/collation classes (`[[=d=]]`,
//!   `[[.d.]]`) are also not supported.
//!
//! ## Hosting your own environment / filesystem
//!
//! [`Env`] and [`FileSystem`] are traits — implement them yourself to
//! sandbox lookups, mock files, or interpose. [`MapEnv`] is a convenient
//! in-memory test double; [`StdEnv`] snapshots `std::env`; [`StdFs`] uses
//! `std::fs` plus `libc` on unix targets.

mod arith;
pub mod ast;
pub mod env;
pub mod error;
pub mod eval;
pub mod fs_abs;
pub mod lex;
pub mod parse;
pub mod pattern;

pub use ast::{
    BinaryOp, CaseModifyKind, Expr, ParameterExpansion, ParameterOp, Primary, RemoveKind,
    ReplaceKind, UnaryOp, Word, WordPart,
};
pub use env::{Env, MapEnv, StdEnv};
pub use error::{EvalError, ParseError};
pub use eval::Evaluator;
pub use fs_abs::{AccessMode, FileKind, FileStat, FileSystem, StdFs};
pub use parse::{ParseOptions, parse, parse_with_options};
pub use pattern::{CompiledGlob, GlobOptions, PatternError};
