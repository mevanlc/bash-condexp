# bash-condexp

A Rust library for parsing and evaluating
[bash conditional expressions](./devdocs/BASH-CONDITIONAL-EXPRESSIONS.md) —
the grammar accepted by `[[ ... ]]`.

## Scope

- `[[`-style grammar only. Outer `[[ ]]` are optional (`-f foo` is valid
  input on its own).
- Combinators: `!`, `&&`, `||`. No `( )` grouping; no `-a` / `-o`.
- Full primary coverage: file tests, file comparison, string, arithmetic,
  `-v` / `-R` / `-o`, `=~` regex, `==` / `!=` extglob.

## Status

Work in progress. See [`aidocs/PLAN.md`](./aidocs/PLAN.md).

## Example

```rust
use bash_condexp::{parse, Evaluator, StdEnv, StdFs};

let expr = parse("-f Cargo.toml && -d src")?;
let outcome = Evaluator::new(&StdEnv::capture(), &StdFs).eval(&expr)?;
assert!(outcome.is_true());
```
