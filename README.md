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
- Bash scalar parameter transformations: length, substring, prefix/suffix
  removal, pattern replacement, case modification, and the pure default and
  alternate-value forms.
- Full scalar arithmetic expressions in arithmetic comparison operands and
  substring indices, including recursive variables and mutation. Arrays and
  general `$((...))` word expansion remain out of scope.
- All five extglob operators. `[[ ... == pattern ]]` and case modification
  always enable them; parameter removal/replacement follows the host's
  `extglob` option, like bash.

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

`Env::shell_opt` returns `Option<bool>`: `Some` is an explicit host value and
`None` asks the evaluator to use bash's default (`patsub_replacement` on;
`extglob` and `nocasematch` off). Arithmetic assignment and increment/decrement
call `Env::set_var`; returning `false` makes those expressions fail with
`EvalError::ArithmeticAssignmentUnsupported`.

## Parameter transformations

```rust
# use bash_condexp::{Evaluator, MapEnv, StdFs, parse};
let mut env = MapEnv::new()
    .with_var("path", "src/lib.rs")
    .with_var("name", "readme.md");
let fs = StdFs;

let expr = parse("${path##*/} == lib.rs && ${name^^[[:lower:]]} == README.MD").unwrap();
assert!(Evaluator::new(&mut env, &fs).eval(&expr).unwrap());
```

Supported forms are `${#name}`, `${name:offset[:length]}`, `${name#pattern}` /
`${name##pattern}`, `${name%pattern}` / `${name%%pattern}`, all four
`${name/pattern/replacement}` modes, `${name^pattern}` / `${name^^pattern}` /
their lowercase counterparts, and `${name-word}`, `${name:-word}`,
`${name+word}`, `${name:+word}`. Nested parameter expansions are supported in
their word, pattern, replacement, offset, and length fields.

By default, parameter names follow bash syntax: identifiers, numeric positional
parameters, and the special parameters `@`, `*`, `#`, `?`, `-`, `$`, and `!`.
Their values still come from the host's `Env`; bash-condexp does not synthesize
positional or special-parameter state.

Empty and ASCII-punctuation-only names used for fd-style placeholders are an
opt-in syntax extension:

```rust
# use bash_condexp::{Evaluator, MapEnv, ParseOptions, StdFs, parse_with_options};
let options = ParseOptions::default().punctuation_variables(true);
let expression = parse_with_options("${/.} == README", options).unwrap();
let mut env = MapEnv::new().with_var("/.", "README");
assert!(Evaluator::new(&mut env, &StdFs).eval(&expression).unwrap());
```

The opt-in accepts `${}`, `${/}`, `${//}`, `${.}`, `${/.}`, and other
punctuation-only host keys. Bash syntax takes precedence where the grammars
overlap, so `${#}` always names bash's `#` parameter and `${##}` takes its
length. An unambiguous custom operation such as `${/#._}` removes `._` from the
start of the variable named `/`. Slash replacement syntax cannot unambiguously
separate a punctuation-only name from its operator; expose a normal identifier
alias when replacement is needed for such a value.

## Testing

```bash
cargo nextest run
cargo nextest run --features bash-conformance  # also diffs against `bash -c`
```

## Limitations

- No command substitution, general arithmetic expansion, process substitution,
  arrays, indirect expansion, or assignment/error parameter forms (`:=`, `:?`,
  `=`, `?`).
- `<` and `>` use byte comparison, not locale-aware `strcoll`.

## Design

See [`devdocs/PLAN.md`](./devdocs/PLAN.md) for the original design and
[`devdocs/LIMITATIONS.md`](./devdocs/LIMITATIONS.md) for compatibility details.
