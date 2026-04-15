//! Tiny CLI: parse one expression from argv, print the AST and the result.
//!
//! Usage:
//!   cargo run --example condexp -- '$x -lt 10 && -e Cargo.toml'
//!
//! Variables are read from the process environment via [`StdEnv`].

use bash_condexp::{Evaluator, StdEnv, StdFs, parse};
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(input) = args.next() else {
        eprintln!("usage: condexp '<expression>'");
        return ExitCode::from(2);
    };

    let expr = match parse(&input) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("parse error: {e}");
            return ExitCode::from(2);
        }
    };

    println!("AST: {expr:#?}");

    let mut env = StdEnv::capture();
    let fs = StdFs;
    match Evaluator::new(&mut env, &fs).eval(&expr) {
        Ok(true) => {
            println!("=> true");
            ExitCode::from(0)
        }
        Ok(false) => {
            println!("=> false");
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("eval error: {e}");
            ExitCode::from(2)
        }
    }
}
