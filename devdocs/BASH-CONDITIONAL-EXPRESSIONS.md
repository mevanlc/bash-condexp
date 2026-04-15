# Bash Conditional Expressions

Conditional expressions are used by the `[[` compound command
and the `test` and `[` builtin commands.
The `test` and `[` commands determine their behavior based on the number
of arguments; see the descriptions of those commands for any other
command-specific actions.

Expressions may be unary or binary,
and are formed from the primaries listed below.
Unary expressions are often used to examine the status of a file
or shell variable.
Binary operators are used for string, numeric, and file attribute
comparisons.

Bash handles several filenames specially when they are used in
expressions.
If the operating system on which Bash is running provides these
special files, Bash uses them; otherwise it emulates them
internally with this behavior:
if the *file* argument to one of the primaries is of the form
`/dev/fd/N`, then Bash checks file descriptor *N*.
If the *file* argument to one of the primaries is one of
`/dev/stdin`, `/dev/stdout`, or `/dev/stderr`,
Bash checks file descriptor 0, 1, or 2, respectively.

When used with `[[`, the `<` and `>` operators sort
lexicographically using the current locale.
The `test` command uses ASCII ordering.

Unless otherwise specified, primaries that operate on files follow symbolic
links and operate on the target of the link, rather than the link itself.

## File Test Operators

| Operator | True if... |
|----------|------------|
| `-a` *file* | *file* exists. |
| `-b` *file* | *file* exists and is a block special file. |
| `-c` *file* | *file* exists and is a character special file. |
| `-d` *file* | *file* exists and is a directory. |
| `-e` *file* | *file* exists. |
| `-f` *file* | *file* exists and is a regular file. |
| `-g` *file* | *file* exists and its set-group-id bit is set. |
| `-h` *file* | *file* exists and is a symbolic link. |
| `-k` *file* | *file* exists and its "sticky" bit is set. |
| `-p` *file* | *file* exists and is a named pipe (FIFO). |
| `-r` *file* | *file* exists and is readable. |
| `-s` *file* | *file* exists and has a size greater than zero. |
| `-t` *fd* | File descriptor *fd* is open and refers to a terminal. |
| `-u` *file* | *file* exists and its set-user-id bit is set. |
| `-w` *file* | *file* exists and is writable. |
| `-x` *file* | *file* exists and is executable. |
| `-G` *file* | *file* exists and is owned by the effective group id. |
| `-L` *file* | *file* exists and is a symbolic link. |
| `-N` *file* | *file* exists and has been modified since it was last accessed. |
| `-O` *file* | *file* exists and is owned by the effective user id. |
| `-S` *file* | *file* exists and is a socket. |

## File Comparison Operators

| Operator | True if... |
|----------|------------|
| *file1* `-ef` *file2* | *file1* and *file2* refer to the same device and inode numbers. |
| *file1* `-nt` *file2* | *file1* is newer (according to modification date) than *file2*, or if *file1* exists and *file2* does not. |
| *file1* `-ot` *file2* | *file1* is older than *file2*, or if *file2* exists and *file1* does not. |

## Shell Option and Variable Operators

| Operator | True if... |
|----------|------------|
| `-o` *optname* | The shell option *optname* is enabled. The list of options appears in the description of the `-o` option to the `set` builtin. |
| `-v` *varname* | The shell variable *varname* is set (has been assigned a value). If *varname* is an indexed array variable name subscripted by `@` or `*`, this returns true if the array has any set elements. If *varname* is an associative array variable name subscripted by `@` or `*`, this returns true if an element with that key is set. |
| `-R` *varname* | The shell variable *varname* is set and is a name reference. |

## String Operators

| Operator | True if... |
|----------|------------|
| `-z` *string* | The length of *string* is zero. |
| `-n` *string* | The length of *string* is non-zero. |
| *string* | (same as `-n` *string*) The length of *string* is non-zero. |
| *string1* `==` *string2* | The strings are equal. When used with `[[`, this performs pattern matching (see Pattern Matching with `[[` below). |
| *string1* `=` *string2* | Same as `==`. `=` should be used with `test` for POSIX conformance. |
| *string1* `!=` *string2* | The strings are not equal. |
| *string1* `<` *string2* | *string1* sorts before *string2* lexicographically. |
| *string1* `>` *string2* | *string1* sorts after *string2* lexicographically. |

## Arithmetic Binary Operators

*arg1* **OP** *arg2*

where **OP** is one of `-eq`, `-ne`, `-lt`, `-le`, `-gt`, or `-ge`.

These return true if *arg1* is equal to, not equal to, less than, less
than or equal to, greater than, or greater than or equal to *arg2*,
respectively. *arg1* and *arg2* may be positive or negative integers.
When used with `[[`, *arg1* and *arg2* are evaluated as arithmetic
expressions. Since the expansions `[[` performs on *arg1* and *arg2*
can potentially result in empty strings, arithmetic expression evaluation
treats those as expressions that evaluate to 0.

## The `[[` Compound Command

```
[[ expression ]]
```

Evaluate the conditional expression *expression* and return a status of
zero (true) or non-zero (false). Expressions are composed of the
primaries described above.

The words between `[[` and `]]` do not undergo word splitting and
filename expansion. The shell performs tilde expansion, parameter and
variable expansion, arithmetic expansion, command substitution, process
substitution, and quote removal on those words. Conditional operators
such as `-f` must be unquoted to be recognized as primaries.

When used with `[[`, the `<` and `>` operators sort lexicographically
using the current locale.

### Pattern Matching with `[[`

When the `==` and `!=` operators are used, the string to the right of
the operator is considered a pattern and matched according to the rules
of Pattern Matching, as if the `extglob` shell option were enabled.
The `=` operator is identical to `==`.
If the `nocasematch` shell option is enabled, the match is performed
without regard to the case of alphabetic characters.
The return value is 0 if the string matches (`==`) or does not match
(`!=`) the pattern, and 1 otherwise.

If you quote any part of the pattern, using any of the shell's quoting
mechanisms, the quoted portion is matched literally. This means every
character in the quoted portion matches itself, instead of having any
special pattern matching meaning.

### Regular Expression Matching with `=~`

An additional binary operator, `=~`, is available, with the same
precedence as `==` and `!=`. When you use `=~`, the string to the right
of the operator is considered a POSIX extended regular expression pattern
and matched accordingly (using the POSIX `regcomp` and `regexec`
interfaces usually described in *regex*(3)).

The return value is 0 if the string matches the pattern, and 1 if it
does not. If the regular expression is syntactically incorrect, the
conditional expression returns 2. If the `nocasematch` shell option is
enabled, the match is performed without regard to the case of alphabetic
characters.

You can quote any part of the pattern to force the quoted portion to be
matched literally instead of as a regular expression.
If the pattern is stored in a shell variable, quoting the variable
expansion forces the entire pattern to be matched literally.

The match succeeds if the pattern matches any part of the string.
If you want to force the pattern to match the entire string, anchor the
pattern using the `^` and `$` regular expression operators.

For example, the following will match a line (stored in the shell
variable `line`) if there is a sequence of characters anywhere in the
value consisting of any number, including zero, of characters in the
`space` character class, immediately followed by zero or one instances
of `a`, then a `b`:

```bash
[[ $line =~ [[:space:]]*(a)?b ]]
```

That means values for `line` like `aab`, `  aaaaaab`, `xaby`, and ` ab`
will all match, as will a line containing a `b` anywhere in its value.

If you want to match a character that's special to the regular expression
grammar (`^$|[]()\.*+?`), it has to be quoted to remove its special
meaning.

Likewise, if you want to include a character in your pattern that has a
special meaning to the regular expression grammar, you must make sure
it's not quoted. If you want to anchor a pattern at the beginning or end
of the string, for instance, you cannot quote the `^` or `$` characters
using any form of shell quoting.

If you want to match `initial string` at the start of a line, the
following will work:

```bash
[[ $line =~ ^"initial string" ]]
```

but this will not:

```bash
[[ $line =~ "^initial string" ]]
```

because in the second example the `^` is quoted and doesn't have its
usual special meaning.

It is sometimes difficult to specify a regular expression properly
without using quotes, or to keep track of the quoting used by regular
expressions while paying attention to shell quoting and the shell's
quote removal. Storing the regular expression in a shell variable is
often a useful way to avoid problems with quoting characters that are
special to the shell. For example, the following is equivalent to the
pattern used above:

```bash
pattern='[[:space:]]*(a)?b'
[[ $line =~ $pattern ]]
```

Shell programmers should take special care with backslashes, since
backslashes are used by both the shell and regular expressions to remove
the special meaning from the following character. This means that after
the shell's word expansions complete, any backslashes remaining in parts
of the pattern that were originally not quoted can remove the special
meaning of pattern characters. If any part of the pattern is quoted, the
shell does its best to ensure that the regular expression treats those
remaining backslashes as literal, if they appeared in a quoted portion.

The following two sets of commands are *not* equivalent:

```bash
pattern='\.'

[[ . =~ $pattern ]]
[[ . =~ \. ]]

[[ . =~ "$pattern" ]]
[[ . =~ '\.' ]]
```

The first two matches will succeed, but the second two will not, because
in the second two the backslash will be part of the pattern to be
matched. In the first two examples, the pattern passed to the regular
expression parser is `\.`. The backslash removes the special meaning
from `.`, so the literal `.` matches. In the second two examples, the
pattern passed to the regular expression parser has the backslash quoted
(e.g., `\\.`), which will not match the string, since it does not
contain a backslash. If the string in the first examples were anything
other than `.`, say `a`, the pattern would not match, because the quoted
`.` in the pattern loses its special meaning of matching any single
character.

Bracket expressions in regular expressions can be sources of errors as
well, since characters that are normally special in regular expressions
lose their special meanings between brackets. However, you can use
bracket expressions to match special pattern characters without quoting
them, so they are sometimes useful for this purpose.

Though it might seem like a strange way to write it, the following
pattern will match a `.` in the string:

```bash
[[ . =~ [.] ]]
```

The shell performs any word expansions before passing the pattern to the
regular expression functions, so you can assume that the shell's quoting
takes precedence. As noted above, the regular expression parser will
interpret any unquoted backslashes remaining in the pattern after shell
expansion according to its own rules. The intention is to avoid making
shell programmers quote things twice as much as possible, so shell
quoting should be sufficient to quote special pattern characters where
that's necessary.

#### `BASH_REMATCH`

The array variable `BASH_REMATCH` records which parts of the string
matched the pattern. The element of `BASH_REMATCH` with index 0 contains
the portion of the string matching the entire regular expression.
Substrings matched by parenthesized subexpressions within the regular
expression are saved in the remaining `BASH_REMATCH` indices. The
element of `BASH_REMATCH` with index *n* is the portion of the string
matching the *n*th parenthesized subexpression.

Bash sets `BASH_REMATCH` in the global scope; declaring it as a local
variable will lead to unexpected results.

### Combining Expressions in `[[`

Expressions may be combined using the following operators, listed in
decreasing order of precedence:

| Operator | Meaning |
|----------|---------|
| `(` *expression* `)` | Returns the value of *expression*. May be used to override normal precedence of operators. |
| `!` *expression* | True if *expression* is false. |
| *expression1* `&&` *expression2* | True if both *expression1* and *expression2* are true. |
| *expression1* `\|\|` *expression2* | True if either *expression1* or *expression2* is true. |

The `&&` and `||` operators do not evaluate *expression2* if the value
of *expression1* is sufficient to determine the return value of the
entire conditional expression.

## The `test` and `[` Builtins

```
test expr
[ expr ]
```

Evaluate a conditional expression *expr* and return a status of 0 (true)
or 1 (false). Each operator and operand must be a separate argument.
Expressions are composed of the primaries described above. `test` does
not accept any options, nor does it accept and ignore an argument of
`--` as signifying the end of options. When using the `[` form, the last
argument to the command must be a `]`.

### Combining Expressions in `test` / `[`

Expressions may be combined using the following operators, listed in
decreasing order of precedence. The evaluation depends on the number of
arguments; see below. `test` uses operator precedence when there are
five or more arguments.

| Operator | Meaning |
|----------|---------|
| `!` *expr* | True if *expr* is false. |
| `(` *expr* `)` | Returns the value of *expr*. May be used to override normal operator precedence. |
| *expr1* `-a` *expr2* | True if both *expr1* and *expr2* are true. |
| *expr1* `-o` *expr2* | True if either *expr1* or *expr2* is true. |

### Argument-Count Rules for `test` / `[`

The `test` and `[` builtins evaluate conditional expressions using a set
of rules based on the number of arguments.

**0 arguments:** The expression is false.

**1 argument:** The expression is true if, and only if, the argument is
not null.

**2 arguments:** If the first argument is `!`, the expression is true if
and only if the second argument is null. If the first argument is one of
the unary conditional operators, the expression is true if the unary
test is true. If the first argument is not a valid unary operator, the
expression is false.

**3 arguments:** The following conditions are applied in the order
listed:

1. If the second argument is one of the binary conditional operators,
   the result of the expression is the result of the binary test using
   the first and third arguments as operands. The `-a` and `-o` operators
   are considered binary operators when there are three arguments.
2. If the first argument is `!`, the value is the negation of the
   two-argument test using the second and third arguments.
3. If the first argument is exactly `(` and the third argument is
   exactly `)`, the result is the one-argument test of the second
   argument.
4. Otherwise, the expression is false.

**4 arguments:** The following conditions are applied in the order
listed:

1. If the first argument is `!`, the result is the negation of the
   three-argument expression composed of the remaining arguments.
2. If the first argument is exactly `(` and the fourth argument is
   exactly `)`, the result is the two-argument test of the second and
   third arguments.
3. Otherwise, the expression is parsed and evaluated according to
   precedence using the rules listed above.

**5 or more arguments:** The expression is parsed and evaluated
according to precedence using the rules listed above.

### Locale and Sorting in `test` / `[`

If the shell is in POSIX mode, or if the expression is part of the `[[`
command, the `<` and `>` operators sort using the current locale. If the
shell is not in POSIX mode, the `test` and `[` commands sort
lexicographically using ASCII ordering.

### Deprecation Note

The historical operator-precedence parsing with 4 or more arguments can
lead to ambiguities when it encounters strings that look like primaries.
The POSIX standard has deprecated the `-a` and `-o` primaries and
enclosing expressions within parentheses. Scripts should no longer use
them. It's much more reliable to restrict test invocations to a single
primary, and to replace uses of `-a` and `-o` with the shell's `&&` and
`||` list operators. For example, use

```bash
test -n string1 && test -n string2
```

instead of

```bash
test -n string1 -a -n string2
```
