# Plan: `bash-condexp` — Rust lib for bash conditional expressions

## Context

The user wants a Rust library that parses and evaluates bash conditional
expressions — the grammar accepted by `[[ ... ]]` (and the sane subset of
`test` / `[`). The project directory `/Users/em/p/my/bash-condexp/` is empty
except for `devdocs/BASH-CONDITIONAL-EXPRESSIONS.md` (the reference spec).
A bash source mirror is available at `/Users/em/p/gh/mirror-bash` for
nuance-checking during implementation.

**Scope decisions (confirmed with the user):**

- `[[`-style grammar **only**. No `test` / `[` argv API. No `-a` / `-o`.
  **No `( )` grouping** — combinator grammar is just `!`, `&&`, `||`.
- **Implicit-`[[`**: the outer `[[` / `]]` are optional and they may appear
  mid-expression around any sub-expression. All of these are valid input:
  - `-f foo.txt`
  - `[[ -f foo.txt ]]`
  - `[[ -e foo.txt ]] && [[ -f foo.txt ]]`
  - `-e foo.txt && -f foo.txt`
  - `[[ -e foo.txt && -f foo.txt ]]`
  - Mixed: `[[ -e foo.txt ]] && -f foo.txt`
- Full primary coverage: file tests, file comparison, string, arithmetic,
  `-v` / `-R` / `-o`, `=~` regex, `==` / `!=` glob.
- Word expansion v1: `$var` / `${var}` only. No cmd-subst, arith-expansion,
  process-subst.
- Arithmetic operands v1: integer literals + `$var`. Empty → 0 per spec.
  Full `$(( ))` grammar is a follow-up.
- Evaluation is real: file-system stat, regex match, variable lookup.
  All abstracted behind traits so callers can sandbox or mock.

## Design overview

Single crate `bash-condexp` with a clean parse → AST → eval pipeline.

```
input (string or argv)
   │
   ▼
 Lexer (for [[ …]] string form)              argv form (pre-tokenized)
   │                                                │
   └────────────────────┬───────────────────────────┘
                        ▼
                   Parser → AST (Expr)
                        │
                        ▼
                 Evaluator (needs &Env + &dyn FileSystem)
                        │
                        ▼
                 Result<EvalOutcome>   // outcome carries BASH_REMATCH
```

### Key types

```rust
pub enum Expr {
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Primary(Primary),
}

pub enum Primary {
    Unary  { op: UnaryOp, arg: Word },
    Binary { op: BinaryOp, lhs: Word, rhs: Word },
    /// Bare word: `[[ $x ]]` ⇒ -n $x
    StringNonEmpty(Word),
}

pub enum UnaryOp {
    FileExists, FileRegular, FileDir, FileBlock, FileChar, FileSymlink,
    FileNamedPipe, FileSocket, FileReadable, FileWritable, FileExecutable,
    FileNonEmpty, FileSetUid, FileSetGid, FileSticky, FileOwnedByUid,
    FileOwnedByGid, FileNewerThanAccess, FdIsTty,
    StringEmpty, StringNonEmpty,
    VarSet, VarIsNameRef, ShellOptSet,
}

pub enum BinaryOp {
    FileSameInode, FileNewer, FileOlder,
    StrEq, StrNe, StrLt, StrGt,
    GlobMatch, GlobNotMatch,        // ==, != (pattern semantics)
    RegexMatch,                      // =~
    ArithEq, ArithNe, ArithLt, ArithLe, ArithGt, ArithGe,
}

/// A parsed word — either a literal, a variable ref, or a mix.
/// Kept as a small IR so expansions can be deferred to eval time.
pub struct Word(pub Vec<WordPart>);
pub enum WordPart {
    Literal(String),
    Quoted(String),       // tracks "was quoted" for =~ and == literalness
    Var(String),
    // ArithExpansion, CmdSubst, etc. — stubs for v1, errors until impl
}
```

### Traits for the host environment

```rust
pub trait Env {
    fn var(&self, name: &str) -> Option<&str>;
    fn is_nameref(&self, name: &str) -> bool { false }
    fn shell_opt(&self, name: &str) -> bool { false }
    fn set_bash_rematch(&mut self, groups: &[Option<&str>]) {}
    /// For -v with subscripts
    fn array_element_set(&self, name: &str, subscript: &str) -> bool { false }
}

pub trait FileSystem {
    fn stat(&self, path: &Path) -> io::Result<Metadata>;
    fn lstat(&self, path: &Path) -> io::Result<Metadata>;
    fn access(&self, path: &Path, mode: AccessMode) -> bool;
    fn is_tty(&self, fd: RawFd) -> bool;
}
```

Provide `StdEnv` / `StdFs` default impls that use `std::env` +
`std::fs`. Special-case `/dev/fd/N`, `/dev/std{in,out,err}` per spec.

### Parser

**Choice: hand-written recursive-descent + small `logos` lexer for `[[`
input.** The grammar is tiny (four precedence levels, no recursion depth
concerns), so a combinator library (`nom`/`winnow`/`chumsky`) would add
dependency weight without improving clarity. `logos` buys us a fast,
correct lexer for the string form without much code. For the argv form
(`Vec<String>` from `test`/`[` callers), we skip the lexer entirely and
parse token slices directly.

Single public entry point:

```rust
pub fn parse(input: &str) -> Result<Expr, ParseError>;
```

**Precedence** (low to high): `||`, `&&`, `!`, primary.
Pratt parsing for the binary operators; primaries are terminals.

**Implicit-`[[` handling**: the lexer emits `[[` and `]]` as tokens when it
sees them but the parser accepts them optionally around any sub-expression.
Concretely, the `expr` production is:

```
expr      := or_expr
or_expr   := and_expr ( '||' and_expr )*
and_expr  := not_expr ( '&&' not_expr )*
not_expr  := '!' not_expr | atom
atom      := '[[' expr ']]' | primary
primary   := unary_op word | word binary_op word | word   // bare word ⇒ -n word
```

No `( expr )` — per user decision. `[[ ]]` is itself the grouping mechanism
when someone wants to be explicit.

**Tokenization nuances** (confirmed from bash source via exploration):

- Operators (`==`, `!=`, `=~`, `<`, `>`, `&&`, `||`, `!`, `(`, `)`) must
  be **unquoted** to be recognized as operators. Quoted `"=="` is a
  literal word.
- The `Word` IR tracks per-part quoting so the evaluator knows which
  bytes of the RHS of `=~` / `==` were quoted (for literalness).

### Evaluator

Walks the `Expr` tree, short-circuiting `&&` / `||`.

**Per-primary notes (from bash source survey):**

- **`=~` regex**: use the `regex` crate with `syntax::Config` tuned for
  POSIX-ERE feel (no lookaround, multi-line off by default). Document
  the small delta vs `regcomp`. Populate `BASH_REMATCH` via
  `Env::set_bash_rematch`.
- **`==` / `!=` patterns**: implement extglob via a small pattern matcher
  (or the `fast-glob` crate for baseline globs plus a custom layer for
  `?(pat)`/`*(pat)`/`+(pat)`/`@(pat)`/`!(pat)`). `nocasematch` option
  read from `Env::shell_opt`.
- **Arithmetic ops** (`-eq` etc.): inside `[[`, operands are parsed as
  arithmetic expressions with empty → 0. Pull in or write a tiny
  `$(( ))`-style evaluator. **v1: accept integer literals and simple
  variable refs only**; full arithmetic expansion is a follow-up.
- **`-v name[idx]`**: parser captures the subscript verbatim; eval
  forwards to `Env::array_element_set`.
- **`<` / `>` sorting**: locale-aware in `[[` per spec. v1: use byte
  comparison and document the deviation; add locale support later.
- **File comparisons**: `-nt` / `-ot` use `mtime`. `-nt` is true when
  lhs exists and rhs does not (per spec). `-ef` compares `(dev, ino)`.

### Error handling

Distinct error types for parse vs eval:

```rust
pub enum ParseError { UnexpectedToken { … }, UnterminatedGroup, … }
pub enum EvalError  { InvalidArith(String), BadRegex(regex::Error),
                       IoError(io::Error), … }
```

`=~` with a syntactically invalid regex returns the special exit status 2
per bash (modeled as a distinct `EvalOutcome::RegexError` variant so
callers can tell it apart from false).

## File layout

```
bash-condexp/
├── Cargo.toml
├── README.md                      (short: overview + example)
├── aidocs/
│   └── PLAN.md                    (this plan, copied over)
├── devdocs/
│   └── BASH-CONDITIONAL-EXPRESSIONS.md   (already there)
├── src/
│   ├── lib.rs                     (re-exports, docs)
│   ├── ast.rs                     (Expr, Primary, Word, ops)
│   ├── lex.rs                     (logos lexer for [[ ]] form)
│   ├── parse.rs                   (recursive-descent + Pratt)
│   ├── eval.rs                    (evaluator)
│   ├── env.rs                     (Env trait + StdEnv)
│   ├── fs_abs.rs                  (FileSystem trait + StdFs)
│   ├── pattern.rs                 (extglob matcher)
│   ├── arith.rs                   (tiny arithmetic evaluator)
│   └── error.rs                   (ParseError, EvalError)
└── tests/
    ├── parse_smoke.rs
    ├── eval_primaries.rs
    ├── eval_regex.rs
    ├── eval_glob.rs
    └── conformance.rs             (compare against real bash via `bash -c`)
```

**Dependencies** (minimal):
- `logos` — lexer
- `regex` — for `=~`
- `thiserror` — error ergonomics
- `libc` (dev-only or feature-gated) — for file mode bits, `isatty`

The conformance test shells out to `bash -c '[[ … ]]'` to spot-check
behavior, gated behind a feature flag so `cargo test` works without bash.

## Implementation order (commit checkpoints)

1. **Scaffold**: `cargo init --lib`, add deps, copy plan to `aidocs/PLAN.md`,
   README stub. *(commit: "scaffold crate")*
2. **AST + errors**: `ast.rs`, `error.rs` with the enums above. *(commit: "add ast and error types")*
3. **Lexer**: `lex.rs` with `logos` covering all operators + word parts.
   Unit tests for tokenization edge cases (quoted operators, `=~` RHS
   quoting). *(commit: "add lexer for [[ form")*
4. **Parser**: recursive-descent + Pratt. Cover `[[` string form and
   argv form. Snapshot-style tests on AST shape. *(commit: "add parser")*
5. **Env/FS traits + std impls**: `env.rs`, `fs_abs.rs`. *(commit: "add env and fs abstractions")*
6. **Evaluator — primaries**: file tests, string ops, `-v`/`-R`/`-o`,
   arithmetic (integers only for v1). *(commit: "evaluate primaries")*
7. **Evaluator — pattern + regex**: extglob matcher in `pattern.rs`,
   wire `==`/`!=`; wire `=~` via `regex` + `BASH_REMATCH` hook.
   *(commit: "add glob and regex matching")*
8. **Evaluator — combinators**: `&&`, `||`, `!`, grouping + short-circuit.
   *(commit: "combine primaries")*
9. **Conformance tests**: `tests/conformance.rs` (feature-gated) diffing
   our result against `bash -c`. *(commit: "add bash conformance tests")*
10. **Docs pass**: rustdoc on public API, README with a worked example.
    *(commit: "add public API docs")*

## Verification

- `cargo check` / `cargo build` clean at each checkpoint.
- `cargo test` green (without bash-conformance feature) after step 8.
- `cargo test --features bash-conformance` green after step 9, with a
  curated matrix of expressions covering each primary + combinator.
- Manual smoke: a small example binary under `examples/` that parses
  an expression from argv and prints the AST + eval result.

## Resolved decisions

- Input form: `[[`-only string input, with optional outer `[[ ]]` and no `( )`
  grouping. Single `parse(&str)` entry point.
- Expansions in operands: `$var` / `${var}` only for v1.
- Arithmetic operands: integer literals + `$var` for v1.
