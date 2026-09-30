//! What can go wrong running a script, as values a host can act on.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use compass_extension_api::Denial;
use rhai::{EvalAltResult, ParseErrorType};

/// Which engine limit a script ran into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitKind {
    /// Too-deep function calls, usually unbounded recursion.
    CallDepth,
    /// Too-deep expression nesting, rejected when compiling.
    ExpressionDepth,
    /// A string longer than the limit.
    StringSize,
    /// An array with more elements than the limit.
    ArraySize,
    /// An object map with more properties than the limit.
    MapSize,
    /// Too many variables in scope.
    Variables,
    /// An attempt to load a module.
    Modules,
}

impl fmt::Display for LimitKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LimitKind::CallDepth => "call depth",
            LimitKind::ExpressionDepth => "expression depth",
            LimitKind::StringSize => "string size",
            LimitKind::ArraySize => "array size",
            LimitKind::MapSize => "map size",
            LimitKind::Variables => "variable count",
            LimitKind::Modules => "module count",
        })
    }
}

/// A script's manifest could not be used.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// The manifest file could not be read.
    #[error("cannot read {path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// Why.
        source: std::io::Error,
    },
    /// The manifest is not valid TOML of the expected shape.
    #[error("invalid manifest {path}: {message}")]
    Invalid {
        /// The file.
        path: PathBuf,
        /// What the parser said.
        message: String,
    },
    /// The entry point escapes the script's own directory.
    #[error("entry `{entry}` must be a file inside the script directory")]
    EntryOutsideDirectory {
        /// The entry as written.
        entry: String,
    },
}

/// Everything that can stop a script call. A script failure is always one of
/// these values, never a panic in the host and never a stalled caller.
#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    /// The manifest could not be used.
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    /// The source could not be read.
    #[error("cannot read script {path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// Why.
        source: std::io::Error,
    },
    /// The source does not compile. Expression-depth violations land here,
    /// because the engine rejects them before anything runs.
    #[error("compile failed: {0}")]
    Compile(String),
    /// A limit other than the operation budget was hit.
    #[error("{limit} limit exceeded: {detail}")]
    LimitExceeded {
        /// Which one.
        limit: LimitKind,
        /// The engine's description.
        detail: String,
    },
    /// The operation budget ran out and `on_progress` terminated the call.
    #[error("operation budget of {budget} exceeded")]
    BudgetExceeded {
        /// The budget that was exhausted.
        budget: u64,
    },
    /// The call ran past its wall-clock limit.
    #[error("script timed out after {0:?}")]
    Timeout(Duration),
    /// A function or module the script named does not exist in its scope.
    ///
    /// This is how an undeclared capability looks from inside a script: not a
    /// refusal, an absence.
    #[error("`{0}` is not available to this script")]
    Undefined(String),
    /// The script does not define an entry point the host called.
    #[error("the script does not define `fn {0}`")]
    MissingEntryPoint(String),
    /// A view asked for something that needs a capability the script does not
    /// hold, such as a `copy` action without `clipboard.write`.
    #[error("{0}")]
    CapabilityDenied(Denial),
    /// The value a script returned is not a view.
    #[error("invalid view at {path}: {message}")]
    InvalidView {
        /// Where in the returned value, e.g. `items[2].actions[0]`.
        path: String,
        /// What was wrong.
        message: String,
    },
    /// The script raised an error of its own, or a host function failed.
    #[error("script error: {0}")]
    Runtime(String),
    /// The blocking task died without returning.
    #[error("script task failed: {0}")]
    Crashed(String),
}

/// Why `on_progress` stopped a call. Carried as the termination token so the
/// classification below never has to parse a message.
pub(crate) const TERMINATE_BUDGET: &str = "compass:budget";
pub(crate) const TERMINATE_TIMEOUT: &str = "compass:timeout";

impl ScriptError {
    /// Maps an engine error to a host error, looking through the frames Rhai
    /// wraps around errors raised inside functions.
    pub(crate) fn from_eval(error: &EvalAltResult, budget: u64, timeout: Duration) -> Self {
        match error {
            EvalAltResult::ErrorInFunctionCall(name, _, inner, _) => {
                match Self::from_eval(inner, budget, timeout) {
                    // A script function that is itself missing is reported by
                    // Rhai as a failure *inside* the caller; keep the inner
                    // name, which is the one the author wrote.
                    ScriptError::Runtime(msg) => {
                        ScriptError::Runtime(format!("in `{name}`: {msg}"))
                    }
                    other => other,
                }
            }
            EvalAltResult::ErrorInModule(_, inner, _) => Self::from_eval(inner, budget, timeout),
            EvalAltResult::ErrorTerminated(token, _) => {
                match token.clone().into_immutable_string().as_deref() {
                    Ok(TERMINATE_TIMEOUT) => ScriptError::Timeout(timeout),
                    _ => ScriptError::BudgetExceeded { budget },
                }
            }
            EvalAltResult::ErrorTooManyOperations(_) => ScriptError::BudgetExceeded { budget },
            EvalAltResult::ErrorStackOverflow(_) => ScriptError::LimitExceeded {
                limit: LimitKind::CallDepth,
                detail: error.to_string(),
            },
            EvalAltResult::ErrorTooManyVariables(_) => ScriptError::LimitExceeded {
                limit: LimitKind::Variables,
                detail: error.to_string(),
            },
            EvalAltResult::ErrorTooManyModules(_) => ScriptError::LimitExceeded {
                limit: LimitKind::Modules,
                detail: error.to_string(),
            },
            EvalAltResult::ErrorDataTooLarge(what, _) => {
                let limit = if what.contains("array") {
                    LimitKind::ArraySize
                } else if what.contains("map") {
                    LimitKind::MapSize
                } else {
                    LimitKind::StringSize
                };
                ScriptError::LimitExceeded {
                    limit,
                    detail: error.to_string(),
                }
            }
            EvalAltResult::ErrorFunctionNotFound(signature, _) => {
                ScriptError::Undefined(signature.clone())
            }
            EvalAltResult::ErrorModuleNotFound(name, _) => ScriptError::Undefined(name.clone()),
            EvalAltResult::ErrorParsing(parse, _) => Self::from_parse(parse),
            other => ScriptError::Runtime(other.to_string()),
        }
    }

    /// Maps a compile error, keeping the two that are sandbox outcomes rather
    /// than mistakes distinct.
    pub(crate) fn from_parse(error: &ParseErrorType) -> Self {
        match error {
            ParseErrorType::ExprTooDeep => ScriptError::LimitExceeded {
                limit: LimitKind::ExpressionDepth,
                detail: error.to_string(),
            },
            ParseErrorType::ModuleUndefined(name) => ScriptError::Undefined(name.clone()),
            other => ScriptError::Compile(other.to_string()),
        }
    }
}
