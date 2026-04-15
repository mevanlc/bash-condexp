//! Environment abstraction — variable lookup, shell options, and the
//! `BASH_REMATCH` write-back hook used after a successful `=~` match.

use std::collections::HashMap;

/// The host environment for evaluation.
pub trait Env {
    /// Look up `$name`. Return `None` for unset, `Some("")` for empty.
    fn var(&self, name: &str) -> Option<&str>;

    /// Whether the variable is a name reference (`declare -n`). Used by `-R`.
    fn is_nameref(&self, _name: &str) -> bool {
        false
    }

    /// Whether the shell option `name` (e.g. `nocasematch`, `extglob`) is on.
    fn shell_opt(&self, _name: &str) -> bool {
        false
    }

    /// Whether `name[subscript]` is set. Used by `-v` with array subscripts.
    /// Default just checks the bare variable when `subscript == "0"` or when
    /// the host doesn't track arrays.
    fn array_element_set(&self, name: &str, subscript: &str) -> bool {
        if subscript == "0" {
            self.var(name).is_some()
        } else {
            false
        }
    }

    /// Called after a successful `=~` match. `groups[0]` is the full match,
    /// `groups[1..]` are capture groups (with `None` for groups that didn't
    /// participate in the match).
    fn set_bash_rematch(&mut self, _groups: &[Option<String>]) {}
}

/// In-memory `Env` for testing. Captures variables and (optionally) options.
#[derive(Debug, Default, Clone)]
pub struct MapEnv {
    pub vars: HashMap<String, String>,
    pub namerefs: HashMap<String, bool>,
    pub options: HashMap<String, bool>,
    pub last_rematch: Vec<Option<String>>,
}

impl MapEnv {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_var(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.vars.insert(name.into(), value.into());
        self
    }

    pub fn with_option(mut self, name: impl Into<String>, on: bool) -> Self {
        self.options.insert(name.into(), on);
        self
    }
}

impl Env for MapEnv {
    fn var(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(String::as_str)
    }
    fn is_nameref(&self, name: &str) -> bool {
        self.namerefs.get(name).copied().unwrap_or(false)
    }
    fn shell_opt(&self, name: &str) -> bool {
        self.options.get(name).copied().unwrap_or(false)
    }
    fn set_bash_rematch(&mut self, groups: &[Option<String>]) {
        self.last_rematch = groups.to_vec();
    }
}

/// Process environment: variables read from `std::env`, no array/nameref/
/// option tracking, `BASH_REMATCH` writes are dropped.
#[derive(Debug, Default)]
pub struct StdEnv {
    snapshot: HashMap<String, String>,
}

impl StdEnv {
    /// Capture the current process environment.
    pub fn capture() -> Self {
        Self {
            snapshot: std::env::vars().collect(),
        }
    }
}

impl Env for StdEnv {
    fn var(&self, name: &str) -> Option<&str> {
        self.snapshot.get(name).map(String::as_str)
    }
}
