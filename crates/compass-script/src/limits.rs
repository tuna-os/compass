//! The resource ceiling every script runs under.

use std::time::Duration;

/// Hard limits for one script. Every field is enforced by the engine or by the
/// call wrapper; none of them is advisory.
///
/// The defaults are sized for a launcher: a search runs on every keystroke, so
/// a script that needs more than a few hundred thousand operations or more than
/// a couple of seconds is not a launcher script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Operations per call before `on_progress` terminates it.
    pub max_operations: u64,
    /// Maximum function call depth, which bounds recursion.
    pub max_call_levels: usize,
    /// Maximum expression nesting at the top level, checked when compiling.
    pub max_expr_depth: usize,
    /// Maximum expression nesting inside functions, checked when compiling.
    pub max_function_expr_depth: usize,
    /// Maximum length of any string, in bytes.
    pub max_string_size: usize,
    /// Maximum number of elements in any array.
    pub max_array_size: usize,
    /// Maximum number of properties in any object map.
    pub max_map_size: usize,
    /// Maximum number of variables in scope at once.
    pub max_variables: usize,
    /// Wall-clock ceiling for a single call, including the time spent in host
    /// functions.
    pub timeout: Duration,
}

impl Limits {
    /// The defaults, as a constant.
    pub const DEFAULT: Limits = Limits {
        max_operations: 500_000,
        max_call_levels: 48,
        max_expr_depth: 64,
        max_function_expr_depth: 32,
        max_string_size: 1024 * 1024,
        max_array_size: 10_000,
        max_map_size: 10_000,
        max_variables: 1_000,
        timeout: Duration::from_secs(2),
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}
