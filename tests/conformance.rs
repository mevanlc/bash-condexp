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

fn bashes(expr: &str, vars: &BTreeMap<String, String>) -> bool {
    // We always wrap with explicit [[ ]] for bash; our parser accepts both
    // forms, so we can test either way. We strip surrounding [[ ]] from our
    // expr if present, then add bash's brackets.
    let body = expr.trim();
    let body = body
        .strip_prefix("[[")
        .and_then(|s| s.strip_suffix("]]"))
        .map(str::trim)
        .unwrap_or(body);
    let script = format!("[[ {body} ]]");
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
    let mut env = vars_to_map(vars);
    let map: BTreeMap<String, String> = vars
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    let our_result = ours(expr, &mut env);
    let bash_result = bashes(expr, &map);
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
}

#[test]
fn glob_match_match_bash() {
    check("$f == *.txt", &[("f", "report.txt")]);
    check("$f == *.txt", &[("f", "report.md")]);
    check("$f == report.[!a-z]*", &[("f", "report.1234")]);
    check("$f == [[:digit:]][[:digit:]]", &[("f", "42")]);
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
}

#[test]
fn nested_brackets_match_bash() {
    check(
        "[[ $x -eq 5 ]] && [[ $name =~ ^a ]]",
        &[("x", "5"), ("name", "alice")],
    );
}

