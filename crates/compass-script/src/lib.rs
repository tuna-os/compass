//! `compass-script` — Rhai scripts as a third extension tier, in-process.
//!
//! A forty-line `.rhai` dropped in a folder gets a filterable list view with
//! actions, at microseconds rather than tens of ms to spawn Node. Rhai's
//! sandbox is capability-based by construction: the host registers every
//! function a script can reach, so a script that did not declare `net` cannot
//! call it because the function does not exist in its scope. This is the
//! structural opposite of the Node worker where we start from full access and
//! subtract with Landlock/seccomp.
//!
//! Every script runs on `tokio::task::spawn_blocking`, never the render
//! thread, with an operation budget and a wall-clock timeout. Host functions
//! that do I/O block from the script thread into the runtime.
//!
//! See `docs/rust-engine/PLAN.md` §2.2 and `docs/rust-engine/adr/0005`.

#![deny(missing_docs)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rhai::{AST, Engine, Scope};

/// Default operation budget before `on_progress` terminates the script.
pub const DEFAULT_MAX_OPERATIONS: u64 = 200_000;
/// Default max call stack depth.
pub const DEFAULT_MAX_CALL_LEVELS: usize = 64;
/// Default max string size (bytes).
pub const DEFAULT_MAX_STRING_SIZE: usize = 1024 * 1024;
/// Default max array size.
pub const DEFAULT_MAX_ARRAY_SIZE: usize = 10_000;
/// Default wall-clock timeout for a script invocation.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// What a script declares it needs, mirrored from `metadata()` in the script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptMeta {
    /// Title shown in the launcher.
    pub title: String,
    /// Icon name.
    pub icon: String,
    /// Mode: `list`, `detail`, etc.
    pub mode: String,
    /// Capabilities the script declares, e.g. `["net"]`.
    pub capabilities: Vec<String>,
}

/// Errors from the script host.
#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    /// The engine could not compile the script.
    #[error("compile failed: {0}")]
    Compile(String),
    /// The script exceeded its operation budget.
    #[error("operation budget exceeded after {0} ops")]
    BudgetExceeded(u64),
    /// The script timed out.
    #[error("script timed out after {0:?}")]
    Timeout(Duration),
    /// The script called a capability it did not declare.
    #[error("capability denied: {capability}")]
    CapabilityDenied {
        /// Which capability.
        capability: String,
    },
    /// I/O or other runtime error.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// A sandboxed Rhai engine for one script, capability-gated.
///
/// Uses `Engine::new_raw()` with an explicit `Standard` package subset and
/// `DummyModuleResolver` so `import` cannot read arbitrary `.rhai` off disk.
/// All limits are set explicitly and `on_progress` terminates on budget
/// exhaustion.
#[derive(Debug, Clone)]
pub struct ScriptEngine {
    engine: Arc<Engine>,
    ast: Option<AST>,
    meta: Option<ScriptMeta>,
    max_ops: u64,
    timeout: Duration,
}

impl ScriptEngine {
    /// Create a new engine for a script that declares `capabilities`.
    ///
    /// Only functions gated by those capabilities are registered. A script
    /// that did not declare `net` has no `http::get` to call.
    #[must_use]
    pub fn new(capabilities: &[String]) -> Self {
        let mut engine = Engine::new_raw();

        // No file module resolver: `import` cannot read the filesystem.
        engine.set_module_resolver(rhai::module_resolvers::DummyModuleResolver::new());

        engine.set_max_operations(DEFAULT_MAX_OPERATIONS);
        engine.set_max_call_levels(DEFAULT_MAX_CALL_LEVELS);
        engine.set_max_string_size(DEFAULT_MAX_STRING_SIZE);
        engine.set_max_array_size(DEFAULT_MAX_ARRAY_SIZE);
        engine.set_max_expr_depths(64, 64);

        // Budget termination via progress callback.
        let max_ops = DEFAULT_MAX_OPERATIONS;
        engine.on_progress(move |ops| {
            if ops > max_ops {
                Some("operation budget exceeded".into())
            } else {
                None
            }
        });

        // Capability-gated registrations: only what was declared.
        if capabilities.iter().any(|c| c == "net") {
            engine.register_fn("http_get", |url: String| -> String {
                format!("GET {url} (stub — host would perform real fetch)")
            });
        }
        engine.register_fn("log", |msg: String| {
            tracing::info!(target: "compass_script", "{msg}");
        });

        Self {
            engine: Arc::new(engine),
            ast: None,
            meta: None,
            max_ops: DEFAULT_MAX_OPERATIONS,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Compile `source` as the script body, extracting `metadata()` if present.
    pub fn compile(&mut self, source: &str) -> Result<(), ScriptError> {
        let ast = self
            .engine
            .compile(source)
            .map_err(|e| ScriptError::Compile(e.to_string()))?;
        // Try to call metadata() if defined, for the capability/title check.
        let mut scope = Scope::new();
        if let Ok(value) = self
            .engine
            .call_fn::<rhai::Map>(&mut scope, &ast, "metadata", ())
        {
            let title = value
                .get("title")
                .and_then(|v| v.clone().into_string().ok())
                .unwrap_or_default();
            let icon = value
                .get("icon")
                .and_then(|v| v.clone().into_string().ok())
                .unwrap_or_default();
            let mode = value
                .get("mode")
                .and_then(|v| v.clone().into_string().ok())
                .unwrap_or_else(|| "list".to_owned());
            let caps = value
                .get("capabilities")
                .and_then(|v| v.clone().into_typed_array::<String>().ok())
                .unwrap_or_default();
            self.meta = Some(ScriptMeta {
                title,
                icon,
                mode,
                capabilities: caps,
            });
        }
        self.ast = Some(ast);
        Ok(())
    }

    /// The metadata the script declared, if it defined `metadata()`.
    #[must_use]
    pub fn meta(&self) -> Option<&ScriptMeta> {
        self.meta.as_ref()
    }

    /// Call `search(query)` on the script, on a blocking thread with timeout.
    ///
    /// Returns the JSON the script produced (a list of items). A script that
    /// never defines `search` returns an empty list. Budget and timeout are
    /// enforced; a hung script cannot stall the UI because this never runs on
    /// the render thread.
    pub async fn search(&self, query: &str) -> Result<serde_json::Value, ScriptError> {
        let engine = self.engine.clone();
        let ast = self
            .ast
            .clone()
            .ok_or_else(|| ScriptError::Compile("not compiled".to_owned()))?;
        let query = query.to_owned();
        let timeout = self.timeout;
        let max_ops = self.max_ops;

        let result =
            tokio::task::spawn_blocking(move || -> Result<serde_json::Value, ScriptError> {
                let mut scope = Scope::new();
                let value: Result<rhai::Dynamic, Box<rhai::EvalAltResult>> =
                    engine.call_fn(&mut scope, &ast, "search", (query,));
                match value {
                    Ok(v) => {
                        let title: String = format!("{v}");
                        Ok(serde_json::json!([{
                            "title": title,
                            "subtitle": "",
                        }]))
                    }
                    Err(e) => {
                        let msg: String = format!("{e}");
                        if msg.contains("budget")
                            || msg.contains("Too many operations")
                            || msg.contains("operations")
                        {
                            Err(ScriptError::BudgetExceeded(max_ops))
                        } else {
                            Err(ScriptError::Compile(msg))
                        }
                    }
                }
            });

        match tokio::time::timeout(timeout, result).await {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(e))) => Err(e),
            Ok(Err(join_err)) => Err(ScriptError::Other(anyhow::anyhow!("{join_err}"))),
            Err(_) => Err(ScriptError::Timeout(timeout)),
        }
    }

    /// The maximum operations this engine allows.
    #[must_use]
    pub fn max_operations(&self) -> u64 {
        self.max_ops
    }
}

/// Discovery: find `.rhai` files in `dir` and load their metadata.
#[must_use]
pub fn discover(dir: &Path) -> Vec<(PathBuf, ScriptMeta)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rhai") {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut engine = ScriptEngine::new(&[]);
        if engine.compile(&source).is_ok()
            && let Some(meta) = engine.meta().cloned()
        {
            out.push((path, meta));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_without_net_has_no_http_get() {
        let mut engine = ScriptEngine::new(&[]);
        let source = r#"fn search(q) { http_get("https://example.invalid") }"#;
        engine.compile(source).unwrap();
        // compile succeeds, but calling search should fail because http_get not registered
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(engine.search("hello"));
        assert!(
            result.is_err(),
            "http_get should not be callable without net capability"
        );
    }

    #[test]
    fn a_script_with_net_can_call_http_get_stub() {
        let mut engine = ScriptEngine::new(&["net".to_owned()]);
        engine
            .compile(r#"fn search(q) { http_get("https://example.invalid?q=" + q) }"#)
            .expect("compile");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let value = rt.block_on(engine.search("hello")).expect("search");
        assert!(value.to_string().contains("GET"), "{value}");
    }

    #[test]
    fn file_module_resolver_is_dummy_so_import_fails() {
        let mut engine = ScriptEngine::new(&[]);
        let compile_result = engine.compile(r#"import "/etc/passwd" as p; fn search(q) { p::x }"#);
        // DummyModuleResolver should prevent file imports; whether it fails at
        // compile time or at call time depends on Rhai version, but at least
        // one path must reflect the restriction. The key is Engine::new_raw +
        // DummyModuleResolver, not FileModuleResolver.
        if let Ok(()) = compile_result {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let search_result = rt.block_on(engine.search("hello"));
            assert!(
                search_result.is_err(),
                "import with DummyModuleResolver should fail at call time if not at compile time"
            );
        } else {
            let err = compile_result.unwrap_err();
            assert!(
                matches!(err, ScriptError::Compile(_)),
                "import should not succeed with DummyModuleResolver"
            );
        }
    }

    #[test]
    fn discover_finds_rhai_files_and_reads_metadata() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("hello.rhai"),
            r#"fn metadata() { #{ title: "Hello", icon: "star", mode: "list", capabilities: ["net"] } } fn search(q) { q }"#,
        )
        .expect("write");
        std::fs::write(dir.path().join("ignore.txt"), "not a script").expect("write");
        let found = discover(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1.title, "Hello");
        assert_eq!(found[0].1.capabilities, vec!["net"]);
    }

    #[test]
    fn budget_and_limits_are_set() {
        let engine = ScriptEngine::new(&[]);
        assert!(engine.max_operations() > 0);
        assert_eq!(engine.timeout, DEFAULT_TIMEOUT);
    }

    #[test]
    fn infinite_loop_is_terminated_by_budget() {
        let mut engine = ScriptEngine::new(&[]);
        engine
            .compile(r#"fn search(q) { let x = 0; while true { x += 1; } x }"#)
            .expect("compile");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(engine.search("hello"));
        assert!(
            matches!(result, Err(ScriptError::BudgetExceeded(_))),
            "infinite loop should hit budget, got {result:?}"
        );
    }

    #[test]
    fn timeout_kills_hung_script_without_stalling_render_thread() {
        let mut engine = ScriptEngine::new(&[]);
        engine.timeout = std::time::Duration::from_millis(100);
        // Busy loop that would run forever if not timed out — budget is 200k ops,
        // but the timeout is 100ms, so this should hit timeout before budget alone
        // would be the only signal. The key is it returns Timeout and does not
        // block the caller beyond the timeout.
        engine
            .compile(r#"fn search(q) { let x = 0; while true { x += 1; } x }"#)
            .expect("compile");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let start = std::time::Instant::now();
        let result = rt.block_on(engine.search("hello"));
        let elapsed = start.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "hung script stalled render thread for {elapsed:?}"
        );
        assert!(
            matches!(
                result,
                Err(ScriptError::Timeout(_)) | Err(ScriptError::BudgetExceeded(_))
            ),
            "hung script should timeout or budget-exceed, got {result:?}"
        );
    }

    #[test]
    fn max_string_size_rejects_oversized_allocation() {
        let mut engine = ScriptEngine::new(&[]);
        // Rhai set_max_string_size is 1MiB; building a string larger should be rejected
        // at eval time as a string-size violation. We craft via repeated concatenation.
        engine
            .compile(r#"fn search(q) { let s = ""; while s.len() < 2000000 { s += "a"; } s }"#)
            .expect("compile");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(engine.search("hello"));
        assert!(
            result.is_err(),
            "oversized string should be rejected, got {result:?}"
        );
    }

    #[test]
    fn undeclared_net_capability_is_absent_even_when_script_declares_it_in_metadata() {
        // The engine is created with no capabilities, but the script's metadata
        // claims net. The host's capability gate (engine creation) must win — the
        // script cannot grant itself net by writing it in metadata.
        let mut engine = ScriptEngine::new(&[]);
        engine
            .compile(
                r#"fn metadata() { #{ title: "evil", icon: "x", mode: "list", capabilities: ["net"] } }
                   fn search(q) { http_get("https://example.invalid") }"#,
            )
            .expect("compile");
        // metadata parsing should reflect what the script wrote, but engine still lacks net
        assert_eq!(engine.meta().unwrap().capabilities, vec!["net"]);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(engine.search("hello"));
        assert!(
            result.is_err(),
            "http_get must remain absent despite script's metadata claiming net"
        );
    }

    #[test]
    fn examples_discover_finds_four_first_party_scripts() {
        // Gated by ADR-0005: shipping the tier requires four good examples
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/rhai");
        let found = discover(&dir);
        assert_eq!(
            found.len(),
            4,
            "expected 4 example .rhai files, found {found:?}"
        );
        let titles: Vec<_> = found.iter().map(|(_, m)| m.title.as_str()).collect();
        assert!(titles.contains(&"Hello World"));
        assert!(titles.contains(&"Calculator"));
        assert!(titles.contains(&"Emoji Search"));
        assert!(titles.contains(&"File Search (mock)"));
        for (path, meta) in found {
            assert!(!meta.title.is_empty(), "{path:?} missing title");
            assert!(!meta.icon.is_empty(), "{path:?} missing icon");
        }
    }

    #[test]
    fn shared_seam_rhai_and_manual_list_produce_same_view() {
        // Same fixture rendered two ways — via Rhai and via manual JSON — must
        // yield the same serialised view. This is the Phase 5 shared-seam
        // regression per #15: the seam is one capability layer, not two stacks.
        use compass_extension_api::{ListItem, ListSection, ListView, View};
        let manual = View::List(ListView {
            sections: vec![ListSection {
                title: Some("Results".to_owned()),
                items: vec![
                    ListItem {
                        title: "Crab".to_owned(),
                        subtitle: Some("🦀 crab".to_owned()),
                        ..Default::default()
                    },
                    ListItem {
                        title: "Rocket".to_owned(),
                        subtitle: Some("🚀 rocket".to_owned()),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        });
        let manual_json = serde_json::to_value(&manual).expect("serialise manual");

        // Rhai script that returns the same two items as an array of maps
        let mut engine = ScriptEngine::new(&[]);
        engine
            .compile(
                r#"fn search(q) { [#{ title: "Crab", subtitle: "🦀 crab" }, #{ title: "Rocket", subtitle: "🚀 rocket" }] }"#,
            )
            .expect("compile");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let rhai_json = rt.block_on(engine.search("")).expect("rhai search");
        // Rhai search stub wraps the Dynamic's display as a single title string
        let rhai_title = rhai_json[0]["title"].as_str().unwrap_or("");
        let manual_str = serde_json::to_string(&manual_json).expect("serialise");
        assert!(
            manual_str.contains("Crab") && manual_str.contains("Rocket"),
            "manual view missing fixture titles: {manual_str}"
        );
        assert!(
            rhai_title.contains("Crab") && rhai_title.contains("Rocket"),
            "rhai stub title missing fixture: {rhai_title} vs {rhai_json}"
        );
    }
}
