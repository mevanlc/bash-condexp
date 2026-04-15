//! Cases ported from bash's own `tests/cond.tests` and
//! `tests/cond-regexp{1,2,3}.sub` (GPL-3.0). Each case runs through both
//! our evaluator and `bash -c '[[ … ]]'` and the booleans must agree.
//!
//! Out-of-scope cases from the upstream files are skipped (and noted at
//! the bottom of this file): extglob (`+([0-9])` etc.), `( … )` grouping,
//! `$'…'` ANSI-C quoting, command substitution, POSIX bracket
//! equivalence/collation classes (`[[=d=]]`, `[[.d.]]`), and tests of
//! deliberately-invalid regex error reporting.
//!
//! Run with: `cargo test --features bash-conformance --test cond_ported`.

#![cfg(feature = "bash-conformance")]

use bash_condexp::{Evaluator, MapEnv, StdFs, parse};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

fn which_bash() -> PathBuf {
    let path = std::env::var("PATH").unwrap_or_default();
    for dir in path.split(':') {
        let p = PathBuf::from(dir).join("bash");
        if p.is_file() {
            return p;
        }
    }
    PathBuf::from("bash")
}

fn vars_to_map(pairs: &[(&str, &str)]) -> MapEnv {
    let mut e = MapEnv::new();
    for (k, v) in pairs {
        e.vars.insert((*k).to_string(), (*v).to_string());
    }
    e
}

fn ours(expr: &str, env: &mut MapEnv) -> bool {
    let parsed = parse(expr).unwrap_or_else(|e| panic!("parse {expr:?}: {e}"));
    let fs = StdFs;
    Evaluator::new(env, &fs)
        .eval(&parsed)
        .unwrap_or_else(|e| panic!("eval {expr:?}: {e}"))
}

fn bashes(expr: &str, vars: &BTreeMap<String, String>) -> bool {
    let body = expr.trim();
    let body = body
        .strip_prefix("[[")
        .and_then(|s| s.strip_suffix("]]"))
        .map(str::trim)
        .unwrap_or(body);
    let script = format!("[[ {body} ]]");
    // Resolve bash via the inherited PATH (matters on macOS where /bin/bash
    // is 3.2 and /usr/local/bin/bash is 5.x — they disagree on `! !`).
    let bash_path = which_bash();
    let mut cmd = Command::new(&bash_path);
    cmd.arg("-c").arg(&script).env_clear();
    cmd.env(
        "PATH",
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into()),
    );
    for (k, v) in vars {
        cmd.env(k, v);
    }
    let output = cmd.output().expect("invoke bash");
    match output.status.code() {
        Some(0) => true,
        Some(1) => false,
        Some(other) => panic!(
            "bash returned {other} for `{script}`\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
        None => panic!("bash killed by signal for `{script}`"),
    }
}

fn check(expr: &str, vars: &[(&str, &str)]) {
    let mut env = vars_to_map(vars);
    let map: BTreeMap<String, String> = vars
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    let ours = ours(expr, &mut env);
    let bash = bashes(expr, &map);
    assert_eq!(
        ours, bash,
        "mismatch on `{expr}` with vars {vars:?}: ours={ours}, bash={bash}"
    );
}

// =============================================================================
// Ported from cond.tests
// =============================================================================

#[test]
fn cond_bare_word_means_nonempty() {
    // [[ x ]] ≡ [[ -n x ]]
    check("x", &[]);
    check("-n x", &[]);
    check("a", &[]);
    check("-n a", &[]);
}

#[test]
fn cond_bang_negation() {
    // [[ ! x ]] is false; ! toggles, doesn't just set a flag
    check("! x", &[]);
    check("! 1 -eq 1", &[]);
    check("! ! 1 -eq 1", &[]);
    check("! ! ! 1 -eq 1", &[]);
    check("! ! ! ! 1 -eq 1", &[]);
}

#[test]
fn cond_bang_binds_to_term_not_expression() {
    // [[ ! x || x ]] -> (!x) || x -> false || true -> true
    check("! x || x", &[]);
}

#[test]
fn cond_unset_var_is_empty_string() {
    check("-n $UNSET", &[]);
    check("-z $UNSET", &[]);
}

#[test]
fn cond_pattern_match_glob() {
    // TDIR=/usr/homes/chet
    check("$TDIR == /usr/homes/*", &[("TDIR", "/usr/homes/chet")]);
    // Quoted star is literal — should not match the path
    check(r"$TDIR == /usr/homes/\*", &[("TDIR", "/usr/homes/chet")]);
    check(
        "$TDIR == '/usr/homes/*'",
        &[("TDIR", "/usr/homes/chet")],
    );
}

#[test]
fn cond_short_circuit_and_or() {
    // First part of && false → second not evaluated
    check("-n $UNSET && $UNSET == foo", &[]);
    // Both empty → false
    check("-z $UNSET && $UNSET == foo", &[]);
    // First part of || true → second not evaluated
    check("-z $UNSET || -d /tmp", &[]);
}

#[test]
fn cond_and_higher_precedence_than_or() {
    // -n $TDIR && -n $UNSET || $TDIR -ef .
    // = (-n $TDIR && -n $UNSET) || ($TDIR -ef .)
    // = (true && false) || (false)        when TDIR=/usr/homes/chet (doesn't exist)
    // = false
    // Use a path that does exist for the -ef branch to make the test
    // exercise both branches deterministically.
    check(
        "-n $TDIR && -n $UNSET || $TDIR -ef /tmp",
        &[("TDIR", "/tmp")],
    );
    // -n $TDIR || -n $UNSET && $PWD -ef xyz
    // = -n $TDIR || (-n $UNSET && $PWD -ef xyz)
    // = true || (...) = true
    check(
        "-n $TDIR || -n $UNSET && /nonexistent -ef xyz",
        &[("TDIR", "/tmp")],
    );
}

#[test]
fn cond_arith_with_unset_and_set() {
    // unset IVAR; [[ 7 -gt $IVAR ]] -> true (unset → 0)
    check("7 -gt $IVAR", &[]);
    // [[ $IVAR -gt 7 ]] with IVAR unset -> false
    check("$IVAR -gt 7", &[]);
    // IVAR=4; [[ $IVAR -gt 7 ]] -> false
    check("$IVAR -gt 7", &[("IVAR", "4")]);
}

#[test]
fn cond_arith_quoted_operand() {
    // [[ "$IVAR" -eq "7" ]] with IVAR=4 -> false
    check(r#""$IVAR" -eq "7""#, &[("IVAR", "4")]);
}

#[test]
fn cond_pattern_filename_unset_vs_set() {
    // [[ $filename == *.c ]] (filename unset) -> false
    check("$filename == *.c", &[]);
    // ...with filename=patmatch.c -> true
    check("$filename == *.c", &[("filename", "patmatch.c")]);
}

#[test]
fn cond_null_pattern_only_matches_null_string() {
    // STR=file.c PAT=; [[ $STR = $PAT ]] -> false
    check("$STR = $PAT", &[("STR", "file.c"), ("PAT", "")]);
    // STR= PAT=; [[ $STR = $PAT ]] -> true
    check("$STR = $PAT", &[("STR", ""), ("PAT", "")]);
}

#[test]
fn cond_regex_match_with_groups() {
    // [[ jbig2dec-0.9-i586-001.tgz =~ ([^-]+)-([^-]+)-([^-]+)-0*([1-9][0-9]*)\.tgz ]]
    check(
        r"jbig2dec-0.9-i586-001.tgz =~ ([^-]+)-([^-]+)-([^-]+)-0*([1-9][0-9]*)\.tgz",
        &[],
    );
}

#[test]
fn cond_regex_quoted_metacharacters_are_literal() {
    // [[ jbig2dec-0.9-i586-001.tgz =~ \([^-]+\)-... ]] should NOT match,
    // since the parens are literalized.
    check(
        r"jbig2dec-0.9-i586-001.tgz =~ \([^-]+\)-\([^-]+\)-\([^-]+\)-0*\([1-9][0-9]*\)\.tgz",
        &[],
    );
}

#[test]
fn cond_regex_quoted_string_substring() {
    // [[ "$LDD_BASH" =~ "libc" ]] -> true (substring match against literal)
    check(
        r#""$LDD_BASH" =~ "libc""#,
        &[(
            "LDD_BASH",
            "libreadline.so.5 => /lib/libreadline.so.5\nlibc.so.6 => /lib/libc.so.6",
        )],
    );
    check(
        r#""$LDD_BASH" =~ libc"#,
        &[(
            "LDD_BASH",
            "libreadline.so.5 => /lib/libreadline.so.5\nlibc.so.6 => /lib/libc.so.6",
        )],
    );
}

// =============================================================================
// Ported from cond-regexp1.sub
// =============================================================================

#[test]
fn cond_regexp1_var_with_quoted_class() {
    // VAR='[[:alpha:]]'
    // [[ $VAR =~ '[[:alpha:]]' ]] -> true (literal "[[:alpha:]]" matches)
    check(
        r"$VAR =~ '[[:alpha:]]'",
        &[("VAR", "[[:alpha:]]")],
    );
    // [[ a =~ '[[:alpha:]]' ]] -> false (literal pattern doesn't match "a")
    check(r"a =~ '[[:alpha:]]'", &[]);
    // [[ a =~ [[:alpha:]] ]] -> true (real char class)
    check("a =~ [[:alpha:]]", &[]);
    // [[ a =~ $VAR ]] -> true (unquoted var keeps regex specialness)
    check("a =~ $VAR", &[("VAR", "[[:alpha:]]")]);
    // [[ a =~ "$VAR" ]] -> false (quoted var becomes literal)
    check(r#"a =~ "$VAR""#, &[("VAR", "[[:alpha:]]")]);
}

#[test]
fn cond_regexp1_aab_optional_group() {
    // line=aab; [[ $line =~ [[:space:]]*(a)?b ]] -> true
    check(
        "$line =~ [[:space:]]*(a)?b",
        &[("line", "aab")],
    );
}

#[test]
fn cond_regexp1_alphabet_eq_and_match() {
    // V="alphabet"
    let v = &[("V", "alphabet")];
    check("$V == alphabet", v);
    check(r#"$V == "alphabet""#, v);
    check("$V == 'alphabet'", v);
    check("$V =~ alphabet", v);
    check(r#"$V =~ "alphabet""#, v);
    check("$V =~ 'alphabet'", v);
}

#[test]
fn cond_regexp1_unquoted_dot_vs_quoted_dot() {
    // pattern="xxx.yyy"; string=xxxAyyy
    // [[ $string =~ $pattern ]] -> true (unquoted, . is regex meta)
    // [[ $string =~ "$pattern" ]] -> false (quoted, literal dot)
    let v = &[("pattern", "xxx.yyy"), ("string", "xxxAyyy")];
    check("$string =~ $pattern", v);
    check(r#"$string =~ "$pattern""#, v);
}

#[test]
fn cond_regexp1_substring_anchors() {
    let v = &[("v", "helloworld")];
    check(r#""helloworld" =~ llo"#, &[]);
    check(r#""helloworld" =~ world"#, &[]);
    check(r#""helloworld" =~ world$"#, &[]);
    check(r#""helloworld" =~ oworld$"#, &[]);
    let _ = v; // also runnable via $v if we wanted
}

// =============================================================================
// Ported from cond-regexp3.sub  (the literal-only cases that don't use $'…')
// =============================================================================

#[test]
fn cond_regexp3_var_holding_backslash_dash() {
    // v='a\-b'; [[ a-b =~ ${v} ]] -> true, since \- in ERE bracket-less
    // context just means literal '-'. (Verified by running bash directly.)
    check("a-b =~ $v", &[("v", r"a\-b")]);
}

// NOT ported from cond-regexp3.sub: `[[ x =~ \x ]]`. Bash's POSIX ERE
// engine treats \x as literal x; Rust's `regex` crate rejects it as an
// "incomplete escape sequence" because it reserves \x for hex escapes.
// Documented as a v1 limitation in lib.rs.

// =============================================================================
// Things deliberately NOT ported (out of scope)
// -----------------------------------------------------------------------------
// - extglob: `[[ $arg == -+([0-9]) ]]`, `*?(a)bc` — we don't implement
//   extglob in v1 (documented in lib.rs).
// - `( … )` grouping inside `[[ … ]]` — explicitly excluded by user.
// - `$'…'` ANSI-C quoting / `$'\001'` etc. — we don't implement it; bash
//   would expand at parse time before our parser ever sees the input.
// - POSIX bracket equivalence/collation classes `[[=d=]]`, `[[.d.]]` — the
//   `regex` crate doesn't support them.
// - cond-regexp2.sub's `cond_invalid` cases — they assert the SHAPE of an
//   error; ours/bashes only diff the boolean.
// - cond-error1.sub / cond-xtrace1.sub — error-output and trace tests, not
//   value tests.
// =============================================================================
