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
//!   (operands: integer literal, `$var`, or empty → 0)
//! - **Regex**: `=~` (POSIX-ERE-ish via the `regex` crate; populates
//!   `BASH_REMATCH` through [`Env::set_bash_rematch`])
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
//! - No command substitution `$(...)`, arithmetic expansion `$((...))`,
//!   or process substitution. Only `$var` / `${var}` are expanded.
//! - Arithmetic operands are integer literals, `$var`, or empty (= 0).
//!   The full `$((…))` grammar (operators, ternary, hex/octal, ...) is
//!   not yet supported.
//! - Extglob (`?(...)`, `*(...)`, `+(...)`, `@(...)`, `!(...)`) is not
//!   yet supported in `==` / `!=` patterns.
//! - `<` and `>` use byte comparison, not locale-aware `strcoll`.
//!
//! ## Hosting your own environment / filesystem
//!
//! [`Env`] and [`FileSystem`] are traits — implement them yourself to
//! sandbox lookups, mock files, or interpose. [`MapEnv`] is a convenient
//! in-memory test double; [`StdEnv`] snapshots `std::env`; [`StdFs`] uses
//! `std::fs` plus `libc` on unix targets.

pub mod ast;
pub mod env;
pub mod error;
pub mod eval;
pub mod fs_abs;
pub mod lex;
pub mod parse;
pub mod pattern;

pub use ast::{BinaryOp, Expr, Primary, UnaryOp, Word, WordPart};
pub use env::{Env, MapEnv, StdEnv};
pub use error::{EvalError, ParseError};
pub use eval::Evaluator;
pub use fs_abs::{AccessMode, FileKind, FileStat, FileSystem, StdFs};
pub use parse::parse;
