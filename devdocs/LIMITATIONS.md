# Limitations

This crate implements the `[[ ... ]]` conditional-expression grammar, with
optional outer `[[` / `]]`, but it is not a complete bash interpreter. The
important boundary is: it parses and evaluates conditional expressions against
an `Env` and a `FileSystem`; it does not run bash's full expansion pipeline
before evaluating the expression.

The examples below assume intermediate familiarity with bash conditionals and
focus on places where real bash behavior is wider or subtly different.

## Input Form

`bash-condexp` accepts a string expression:

```rust
parse("-f Cargo.toml && $name == al*")?;
parse("[[ -f Cargo.toml && $name == al* ]]")?;
parse("[[ -f Cargo.toml ]] && [[ $name == al* ]]")?;
```

It does not implement the argv-oriented `test` / `[` command interface.

```bash
# Real bash supports this command form:
[ -f Cargo.toml -a -d src ]

# This crate does not parse `[` / `]` command arguments.
```

The grammar intentionally does not support `( ... )` grouping. Use nested
`[[ ... ]]` if you need explicit grouping.

```bash
# Real bash:
[[ ( -f Cargo.toml || -f Cargo.lock ) && -d src ]]

# This crate:
[[ [[ -f Cargo.toml || -f Cargo.lock ]] && -d src ]]
```

The legacy `-a` and `-o` combinators are not supported as boolean operators.
Unary `-a path` (file exists) and unary `-o optname` (shell option set) are
supported.

```bash
# Real bash accepts this legacy form:
[[ -f Cargo.toml -a -d src ]]

# This crate:
[[ -f Cargo.toml && -d src ]]
```

## Word Expansion

`$name` and plain `${name}` references are expanded. The following scalar
parameter transformations are also supported, including nested parameter
expansions in their subsidiary words:

- length: `${#name}`
- substring: `${name:offset}` and `${name:offset:length}`
- shortest/longest prefix and suffix removal: `#`, `##`, `%`, `%%`
- first/all/prefix/suffix pattern replacement: `/`, `//`, `/#`, `/%`
- first/all uppercase and lowercase modification: `^`, `^^`, `,`, `,,`
- pure default and alternate values: `-`, `:-`, `+`, `:+`

Quoting is tracked per word part so the evaluator can preserve bash's
literalness rules for patterns, regular expressions, and replacement `&`.

Supported:

```bash
[[ $name == al* ]]
[[ "${name}" == "alice" ]]
[[ ${line} =~ ^user-([0-9]+)$ ]]
[[ ${path##*/} == Cargo.toml ]]
[[ ${name:-anonymous} == alice ]]
```

Not implemented:

```bash
[[ $(uname) == Darwin ]]
[[ $((1 + 2)) -eq 3 ]]
[[ -e <(printf '%s\n' data) ]]
[[ ${array[0]} == value ]]
[[ ${name:=fallback} == alice ]]
[[ $'a\nb' =~ $'\n' ]]
```

Assignment/error parameter forms (`:=`, `:?`, `=`, `?`) are rejected rather
than mutating the host or producing shell diagnostics. Empty and
ASCII-punctuation-only host names are rejected by the default bash-compatible
parser and require `ParseOptions::punctuation_variables(true)`. Bash syntax has
precedence in that mode, including the `${#}` special parameter. Slash
replacement syntax is inherently ambiguous for punctuation-only host names such
as `/`; expose a normal identifier alias for those values when replacement is
needed. Other unambiguous transformations work, for example `${/#._}` removes
`._` from the start of the value named `/`.

## Arithmetic Operands

Bash evaluates arithmetic comparison operands as arithmetic expressions:

```bash
[[ x + 1 -eq 6 ]]
[[ 0x10 -eq 16 ]]
[[ 010 -eq 8 ]]
[[ n++ -gt 3 ]]
```

This crate evaluates full scalar integer expressions in arithmetic comparison
operands and substring indices. This includes recursive scalar variable lookup,
decimal/octal/hexadecimal and `base#number` literals, unary/binary/logical
operators, power, ternary and comma operators, assignment, and pre/post
increment/decrement. Examples:

```bash
[[ "" -eq 0 ]]
[[ -5 -lt 10 ]]
[[ $port -lt 9000 ]]
[[ port -lt 9000 ]]    # arithmetic context looks up variable `port`
[[ '2**8' -eq 256 ]]
[[ 'n++, n' -eq 8 ]]
```

The expression still has to occupy one conditional word, as it does in bash;
quote expressions that contain spaces. Arrays and subscripts are not supported:

```bash
[[ arr[0] -eq 7 ]]
```

Mutation calls `Env::set_var`. A read-only host receives
`EvalError::ArithmeticAssignmentUnsupported`; `MapEnv` and `StdEnv` implement
writes to their in-memory maps. This does not mutate the process environment.

## Glob Patterns

For `==`, `=`, and `!=`, this crate implements the common `[[ ... ]]` pattern
rules:

- `*`
- `?`
- bracket expressions such as `[abc]`, `[!abc]`, and `[a-z]`
- POSIX character classes such as `[[:digit:]]`
- backslash escaping outside brackets
- quoted parts matching literally
- `nocasematch` through `Env::shell_opt("nocasematch")`

Examples:

```bash
[[ $file == *.txt ]]
[[ $file == report.[!a-z]* ]]
[[ $value == [[:digit:]][[:digit:]] ]]
[[ $file != "*.txt" ]]   # quoted star is literal
```

All five extglob operators are supported. Like bash, conditional matching acts
as if `extglob` were enabled:

```bash
[[ $arg == -+([0-9]) ]]
[[ $name == @(alice|bob) ]]
[[ $path == !(tmp)/* ]]
```

Case modification also always recognizes extglob. Parameter prefix/suffix
removal and replacement recognize it only when
`Env::shell_opt("extglob") == Some(true)`. `nocasematch` affects conditional
matching and replacement, but not removal or case modification.

## Regex Matching

`=~` is implemented with Rust's `regex` crate. That is close enough for many
POSIX ERE-style patterns:

```bash
[[ $line =~ ^[[:space:]]*(a)?b ]]
[[ $v =~ ^([a-z]+)-([0-9]+)$ ]]
[[ $text =~ needle ]]       # substring match, like bash
```

A successful match calls `Env::set_bash_rematch(&groups)`, with the full match
at index 0 and capture groups after that. This mirrors the useful shape of
`BASH_REMATCH`, but storage and lifetime are controlled by the caller's `Env`
implementation.

Known differences from bash's regex engine:

- Rust `regex` rejects some escape sequences that bash accepts literally.
- A lone `\x` is a known example: bash treats it as matching literal `x`, while
  Rust `regex` reserves `\x..` for hex escapes and returns an invalid-regex
  error.
- POSIX bracket equivalence classes (`[[=d=]]`) are not supported.
- POSIX bracket collation symbols (`[[.d.]]`) are not supported.
- Rust `regex` has its own Unicode-aware semantics in some contexts; bash's
  behavior is tied to the platform regex implementation and locale.

Example divergence:

```bash
# Real bash: true, because \x is treated as literal x by its regex engine.
[[ x =~ \x ]]

# This crate: EvalError::BadRegex, because Rust regex rejects incomplete \x.
```

Use simpler ERE-compatible syntax when portability between bash and this crate
matters.

## String Ordering

Bash compares `<` and `>` lexicographically using the current locale inside
`[[ ... ]]`.

This crate compares the expanded strings byte-wise.

```bash
[[ $left > z ]]   # where `left` contains a non-ASCII string
```

The result of that expression can differ depending on locale in bash. In this
crate, the result follows UTF-8 byte order. For ASCII-only strings this usually
matches the ordering people expect; for locale-sensitive text it should not be
treated as bash-equivalent.

## Shell State Is Supplied by `Env`

Several bash primaries depend on shell state that does not naturally exist in a
standalone Rust library. This crate exposes that state through `Env`.

`-v name` checks `Env::var(name)` and, for subscripted forms, calls
`Env::array_element_set(name, subscript)`.

```bash
[[ -v name ]]
[[ -v arr[0] ]]
```

The default `MapEnv` is useful for tests, but it does not model full bash array
semantics. If you care about indexed or associative arrays, provide an `Env`
implementation that answers `array_element_set` accurately.

`-R name` calls `Env::is_nameref(name)`. Real bash asks whether a variable is a
name reference created with `declare -n`; this crate can only know that if your
`Env` says so.

`-o optname` calls `Env::shell_opt(optname)`. The return type is `Option<bool>`:
`Some` is an explicit host value, while `None` uses evaluator defaults.
`patsub_replacement` defaults on, and `extglob` and `nocasematch` default off.

`StdEnv` snapshots process environment variables. It does not know bash
namerefs, arrays, shell options, or where to persist `BASH_REMATCH`.

## Filesystem Semantics

File tests are evaluated through `FileSystem`. `StdFs` uses `std::fs` and, on
Unix, `libc` for mode bits, ownership, access checks, effective uid/gid, and
TTY detection.

The usual file tests are implemented:

```bash
[[ -e path ]]
[[ -f path ]]
[[ -d path ]]
[[ path1 -nt path2 ]]
[[ path1 -ef path2 ]]
```

The exact behavior can still differ from bash when:

- the caller provides a custom `FileSystem`
- the platform is not Unix
- permission checks depend on OS-specific access-control details
- special bash path handling is expected outside what `std::fs` / `libc`
  expose

For deterministic tests, prefer a mock `FileSystem` or temporary files whose
metadata you control.

## Error And Exit-Status Modeling

The library returns `Result<bool, EvalError>` for evaluation. Bash commands
report status codes instead:

- `0` for true
- `1` for false
- `2` for many syntax or evaluation errors, including invalid regex

The example CLI maps `true` to exit code 0, `false` to 1, and parse/eval errors
to 2. Library callers should decide whether an `EvalError` should be surfaced,
mapped to a shell-style status code, or treated as false for their use case.

## Deliberately Skipped Bash Test Areas

The conformance tests include cases ported from bash's conditional-expression
tests, but skip areas that are outside v1 scope:

- extglob pattern operators
- `( ... )` grouping
- ANSI-C `$'...'` quoting
- command substitution
- process substitution
- full arithmetic-expression parsing
- POSIX bracket equivalence and collation classes
- exact invalid-regex diagnostic shape
- xtrace and shell error-output formatting

These are not accidental omissions in tests; they are boundaries of the current
implementation.
