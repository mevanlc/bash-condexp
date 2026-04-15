//! End-to-end tests for combinators (`&&`, `||`, `!`) — including
//! short-circuit behavior and mixing implicit and explicit `[[ ]]`.

use bash_condexp::{Evaluator, MapEnv, StdFs, parse};
use std::cell::Cell;

/// Env that counts variable lookups so we can verify short-circuiting.
struct CountingEnv {
    inner: MapEnv,
    lookups: Cell<usize>,
}

impl CountingEnv {
    fn new(inner: MapEnv) -> Self {
        Self {
            inner,
            lookups: Cell::new(0),
        }
    }
}

impl bash_condexp::Env for CountingEnv {
    fn var(&self, name: &str) -> Option<&str> {
        self.lookups.set(self.lookups.get() + 1);
        self.inner.var(name)
    }
    fn shell_opt(&self, name: &str) -> bool {
        self.inner.shell_opt(name)
    }
}

fn run(input: &str) -> bool {
    let expr = parse(input).expect("parse");
    let mut env = MapEnv::new().with_var("x", "5").with_var("name", "alice");
    let fs = StdFs;
    Evaluator::new(&mut env, &fs).eval(&expr).expect("eval")
}

#[test]
fn and_true_true() {
    assert!(run("$x -eq 5 && $name == alice"));
}

#[test]
fn and_short_circuits_on_false_lhs() {
    let mut env = CountingEnv::new(MapEnv::new().with_var("y", "2"));
    let expr = parse("$y -gt 100 && $missing -lt 5").unwrap();
    let fs = StdFs;
    let r = Evaluator::new(&mut env, &fs).eval(&expr).expect("eval");
    assert!(!r);
    // Only LHS should have been evaluated. `$y` is one var lookup;
    // `$missing` would be a second if RHS ran.
    assert_eq!(env.lookups.get(), 1, "RHS should not be evaluated");
}

#[test]
fn or_short_circuits_on_true_lhs() {
    let mut env = CountingEnv::new(MapEnv::new().with_var("y", "200"));
    let expr = parse("$y -gt 100 || $missing -lt 5").unwrap();
    let fs = StdFs;
    let r = Evaluator::new(&mut env, &fs).eval(&expr).expect("eval");
    assert!(r);
    assert_eq!(env.lookups.get(), 1, "RHS should not be evaluated");
}

#[test]
fn not_negates() {
    assert!(run("! -e /this/does/not/exist/abc123"));
    assert!(!run("! $x -eq 5"));
}

#[test]
fn precedence_and_binds_tighter() {
    // a || b && c  =>  a || (b && c)
    assert!(run("$x -eq 99 || $x -eq 5 && $name == alice"));
    // If && bound looser this would parse as (a || b) && c, and would be
    // false since $name != bob.
    assert!(!run("$x -eq 99 || $x -eq 5 && $name == bob"));
}

#[test]
fn nested_brackets_compose() {
    assert!(run("[[ $x -eq 5 ]] && [[ $name =~ ^a ]]"));
    assert!(!run("[[ $x -eq 5 && $name == bob ]]"));
}

#[test]
fn implicit_and_explicit_mix() {
    assert!(run("[[ $x -eq 5 ]] && $name == alice"));
    assert!(run("$x -eq 5 && [[ $name == alice ]]"));
}

#[test]
fn chained_or() {
    assert!(run("$x -eq 1 || $x -eq 2 || $x -eq 5"));
    assert!(!run("$x -eq 1 || $x -eq 2 || $x -eq 3"));
}

#[test]
fn chained_and() {
    assert!(run("$x -gt 0 && $x -lt 10 && $name == alice"));
    assert!(!run("$x -gt 0 && $x -lt 10 && $name == bob"));
}

#[test]
fn double_negation() {
    assert!(run("! ! $x -eq 5"));
}
