//! Conformance tests against real bash.
//!
//! Run with `cargo test --features bash-conformance`.
//!
//! Each case:
//!   1. Evaluates the expression with our crate.
//!   2. Runs `bash -c '[[ … ]]'; echo $?` with the same variables in the
//!      child's environment.
//!   3. Asserts our `bool` matches bash's exit status (0 ↔ true, 1 ↔ false).
//!
//! The matrix focuses on each operator family. It's not exhaustive — that's
//! what the unit tests are for — but it catches grammar/semantic drift from
//! bash itself.

#![cfg(feature = "bash-conformance")]

use bash_condexp::{Evaluator, MapEnv, StdFs, parse};
use std::collections::BTreeMap;
use std::io::Write;
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

fn bashes(expr: &str, vars: &BTreeMap<String, String>, options: &[(&str, bool)]) -> bool {
    // We always wrap with explicit [[ ]] for bash; our parser accepts both
    // forms, so we can test either way. We strip surrounding [[ ]] from our
    // expr if present, then add bash's brackets.
    let body = expr.trim();
    let body = body
        .strip_prefix("[[")
        .and_then(|s| s.strip_suffix("]]"))
        .map(str::trim)
        .unwrap_or(body);
    let option_setup: String = options
        .iter()
        .map(|(name, enabled)| format!("shopt -{} {name}; ", if *enabled { "s" } else { "u" }))
        .collect();
    let script = format!("{option_setup}[[ {body} ]]");
    let bash_path = which_bash();
    let mut cmd = Command::new(&bash_path);
    cmd.arg("-c").arg(&script);
    cmd.env_clear();
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
    check_with_options(expr, vars, &[]);
}

fn check_with_options(expr: &str, vars: &[(&str, &str)], options: &[(&str, bool)]) {
    let mut env = vars_to_map(vars);
    for (name, enabled) in options {
        env.options.insert((*name).to_owned(), *enabled);
    }
    let map: BTreeMap<String, String> = vars
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    let our_result = ours(expr, &mut env);
    let bash_result = bashes(expr, &map, options);
    assert_eq!(
        our_result, bash_result,
        "mismatch on `{expr}` with vars {vars:?}: ours={our_result}, bash={bash_result}"
    );
}

#[test]
fn string_ops_match_bash() {
    check("-z $empty", &[("empty", "")]);
    check("-n $name", &[("name", "alice")]);
    check("$name == alice", &[("name", "alice")]);
    check("$name != bob", &[("name", "alice")]);
    check("apple < banana", &[]);
    check("banana > apple", &[]);
}

#[test]
fn arith_ops_match_bash() {
    check("$x -eq 5", &[("x", "5")]);
    check("$x -ne 5", &[("x", "4")]);
    check("$x -lt 10", &[("x", "5")]);
    check("$x -le 5", &[("x", "5")]);
    check("$x -gt 4", &[("x", "5")]);
    check("$x -ge 5", &[("x", "5")]);
    check("'2**3**2' -eq 512", &[]);
    check("'010 + 0x10 + 2#10 + 64#_' -eq 89", &[]);
    check("recursive -eq 3", &[("recursive", "x+1"), ("x", "2")]);
}

#[test]
fn glob_match_match_bash() {
    check("$f == *.txt", &[("f", "report.txt")]);
    check("$f == *.txt", &[("f", "report.md")]);
    check("$f == report.[!a-z]*", &[("f", "report.1234")]);
    check("$f == [[:digit:]][[:digit:]]", &[("f", "42")]);
    check("$f == +([[:digit:]])", &[("f", "42")]);
}

#[test]
fn parameter_transformations_match_bash() {
    let vars = &[("path", "a/b/c"), ("value", "abcabc")];
    check("${path#*/} == b/c", vars);
    check("${path##*/} == c", vars);
    check("${path%/*} == a/b", vars);
    check("${path%%/*} == a", vars);
    check("${value/a/X} == Xbcabc", vars);
    check("${value//a/X} == XbcXbc", vars);
    check("${value/#a/X} == Xbcabc", vars);
    check("${value/%c/X} == abcabX", vars);
    check("${value//?/[&]} == '[a][b][c][a][b][c]'", vars);
    check(r"${value//?/\&} == '&&&&&&'", vars);
}

#[test]
fn parameter_length_substring_case_and_defaults_match_bash() {
    let vars = &[
        ("value", "abcdef"),
        ("offset", "1"),
        ("letters", "abCab"),
        ("empty", ""),
        ("set", "value"),
    ];
    check("${#value} -eq 6", vars);
    check("${value:2:3} == cde", vars);
    check("${value: -2} == ef", vars);
    check("${value:offset+1:2} == cd", vars);
    check("${letters^} == AbCab", vars);
    check("${letters^^@(a|b)} == ABCAB", vars);
    check("${letters,,C} == abcab", vars);
    check("${missing-fallback} == fallback", vars);
    check("${empty-fallback} == ''", vars);
    check("${empty:-fallback} == fallback", vars);
    check("${set+alternate} == alternate", vars);
    check("${empty:+alternate} == ''", vars);
    check("foo != ${missing-'*'}", vars);
}

#[test]
fn parameter_pattern_options_match_bash() {
    let vars = &[("value", "aaab"), ("upper", "Aaa")];
    check("${value##+(a)} == aaab", vars);
    check_with_options("${value##+(a)} == b", vars, &[("extglob", true)]);
    check_with_options("${upper/a/X} == Xaa", vars, &[("nocasematch", true)]);
    check_with_options("${upper#a*} == Aaa", vars, &[("nocasematch", true)]);
    check_with_options(
        "${value/a/[&]} == '[&]aab'",
        vars,
        &[("patsub_replacement", false)],
    );
}

#[test]
fn regex_match_match_bash() {
    check("$line =~ ^[[:space:]]*(a)?b", &[("line", "  ab cd")]);
    check("$v =~ ^([a-z]+)-([0-9]+)$", &[("v", "user-42")]);
    check("$line =~ ^foo$", &[("line", "foo")]);
    check("$line =~ ^foo$", &[("line", "foobar")]);
}

#[test]
fn combinators_match_bash() {
    check("$x -gt 0 && $x -lt 10", &[("x", "5")]);
    check("$x -lt 0 || $x -gt 100", &[("x", "200")]);
    check("! $x -eq 5", &[("x", "5")]);
    check(
        "$x -gt 0 && $x -lt 10 && $name == alice",
        &[("x", "5"), ("name", "alice")],
    );
}

#[test]
fn file_tests_match_bash() {
    use tempfile::NamedTempFile;
    let mut f = NamedTempFile::new().unwrap();
    writeln!(f, "hello").unwrap();
    let path = f.path().to_str().unwrap().to_string();
    check("-e $p", &[("p", &path)]);
    check("-f $p", &[("p", &path)]);
    check("-s $p", &[("p", &path)]);
    check("-d $p", &[("p", &path)]);
    check("-e $p", &[("p", "/no/such/path/abc123")]);
    check("-e $p", &[("p", "")]);
    check("-r $p", &[("p", "")]);
    check("$p -nt $empty", &[("p", &path), ("empty", "")]);
    check("$empty -ot $p", &[("p", &path), ("empty", "")]);
}

#[test]
fn nested_brackets_match_bash() {
    check(
        "[[ $x -eq 5 ]] && [[ $name =~ ^a ]]",
        &[("x", "5"), ("name", "alice")],
    );
}
