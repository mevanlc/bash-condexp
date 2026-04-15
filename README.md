# bash-condexp

A Rust library for parsing and evaluating
[bash conditional expressions](./devdocs/BASH-CONDITIONAL-EXPRESSIONS.md) —
the grammar accepted by `[[ ... ]]`.

## Scope

- `[[`-style grammar. Outer `[[ ]]` are optional (`-f foo` is valid input
  on its own) and may also wrap any sub-expression.
- Combinators: `!`, `&&`, `||`. No `( ... )` grouping; no `-a` / `-o`.
- Full primary coverage: file tests, file comparison, string ops, arithmetic
  comparisons, `-v` / `-R` / `-o`, `=~` regex (with `BASH_REMATCH`),
  `==` / `!=` glob.

## Example

```rust
use bash_condexp::{Evaluator, MapEnv, StdFs, parse};

let mut env = MapEnv::new()
    .with_var("name", "alice")
    .with_var("port", "8080");
let fs = StdFs;

let expr = parse("$name == al* && $port -lt 9000 && -e Cargo.toml").unwrap();
let truth = Evaluator::new(&mut env, &fs).eval(&expr).unwrap();
assert!(truth);
```

Try the CLI example:

```bash
cargo run --example condexp -- '$HOME != "" && -d $HOME'
```

## Hosting your own environment

`Env` and `FileSystem` are traits — implement them to sandbox lookups, mock
files, or interpose. Defaults provided:

- `MapEnv` — in-memory test double (good for unit tests)
- `StdEnv` — snapshots `std::env`
- `StdFs` — `std::fs` + `libc` on unix targets

After a successful `=~` match, the evaluator calls
`Env::set_bash_rematch(&groups)` with the full match in `groups[0]` and
capture groups in `groups[1..]` — your `Env` impl can store these
however you like.

## Testing

```bash
cargo test                                  # unit + integration tests
cargo test --features bash-conformance      # also diffs against `bash -c`
```

## v1 limitations

- Word expansion: `$var` and `${var}` only — no `$(...)`, `$((...))`,
  process substitution.
- Arithmetic operands: integer literal, `$var`, or empty (= 0). Full
  `$((...))` grammar is a follow-up.
- Glob: `*`, `?`, `[...]`, POSIX character classes, `\X` escapes. Extglob
  (`?(...)` / `*(...)` / etc.) is a follow-up.
- `<` and `>` use byte comparison, not locale-aware `strcoll`.

## Design

See [`aidocs/PLAN.md`](./aidocs/PLAN.md) for the full design and
implementation notes.
