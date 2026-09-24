//! One loaded script: compile once, then search and dispatch actions.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use compass_extension_api::{
    ActionEffect, ActionRequest, ActionResponse, Capability, CapabilityRegistry, DispatchError,
    HandlerId, ToastStyle, ViewTree,
};
use rhai::{AST, Dynamic, Engine, Scope};

use crate::discovery::DiscoveredScript;
use crate::engine::{self, Deadline, Grants};
use crate::error::ScriptError;
use crate::host::ScriptHost;
use crate::limits::Limits;
use crate::manifest::ScriptManifest;
use crate::render::{Handler, Render};

/// The entry point every script defines.
pub const SEARCH_FN: &str = "search";

#[derive(Debug, Default)]
struct Handlers {
    render: u64,
    table: BTreeMap<HandlerId, Handler>,
}

#[derive(Debug)]
struct Inner {
    engine: Engine,
    ast: AST,
    deadline: Arc<Deadline>,
    limits: Limits,
    grants: Grants,
    host: Arc<dyn ScriptHost>,
    /// Serialises calls: a script is single-threaded, and the deadline slot
    /// belongs to whichever call holds this.
    running: Mutex<()>,
    renders: AtomicU64,
    handlers: Mutex<Handlers>,
}

/// A compiled script, ready to be searched.
///
/// Every call runs on Tokio's blocking pool, never on the caller's thread, and
/// returns within [`Limits::timeout`] whatever the script does. The instance
/// must therefore be used from inside a Tokio runtime.
///
/// Capabilities are resolved once, when the instance is built. After changing
/// a grant, build a new instance; the old one keeps exactly the functions it
/// was built with.
#[derive(Debug, Clone)]
pub struct ScriptInstance {
    manifest: ScriptManifest,
    inner: Arc<Inner>,
}

impl ScriptInstance {
    /// Compiles a discovered script.
    ///
    /// `registry` must already hold this script's declarations (see
    /// [`ScriptManifest::declare_into`]) and whatever the host has granted.
    ///
    /// # Errors
    ///
    /// When the source does not compile, trips a compile-time limit, names a
    /// capability module it was not granted, or does not define
    /// `fn search(query)`.
    pub fn load(
        script: &DiscoveredScript,
        registry: &CapabilityRegistry,
        host: Arc<dyn ScriptHost>,
        limits: Limits,
    ) -> Result<Self, ScriptError> {
        Self::compile(
            script.manifest.clone(),
            &script.source,
            registry,
            host,
            limits,
        )
    }

    /// Compiles `source` for `manifest`.
    ///
    /// # Errors
    ///
    /// As [`load`](Self::load).
    pub fn compile(
        manifest: ScriptManifest,
        source: &str,
        registry: &CapabilityRegistry,
        host: Arc<dyn ScriptHost>,
        limits: Limits,
    ) -> Result<Self, ScriptError> {
        let grants = Grants::check(registry, &manifest.id);
        let deadline = engine::deadline();
        let engine = engine::build(&limits, deadline.clone(), &grants, &host);
        let ast = engine
            .compile(source)
            .map_err(|error| ScriptError::from_parse(error.err_type()))?;
        if !ast
            .iter_functions()
            .any(|f| f.name == SEARCH_FN && f.params.len() == 1)
        {
            return Err(ScriptError::MissingEntryPoint(format!(
                "{SEARCH_FN}(query)"
            )));
        }
        Ok(Self {
            manifest,
            inner: Arc::new(Inner {
                engine,
                ast,
                deadline,
                limits,
                grants,
                host,
                running: Mutex::new(()),
                renders: AtomicU64::new(0),
                handlers: Mutex::new(Handlers::default()),
            }),
        })
    }

    /// The manifest this instance was built from.
    #[must_use]
    pub fn manifest(&self) -> &ScriptManifest {
        &self.manifest
    }

    /// The limits this instance runs under.
    #[must_use]
    pub fn limits(&self) -> &Limits {
        &self.inner.limits
    }

    /// Runs `search(query)` and returns the view it describes.
    ///
    /// The actions in the returned tree are live until the next successful
    /// search; invoking one from an older tree is an unknown action.
    ///
    /// # Errors
    ///
    /// Any [`ScriptError`]: the budget, a limit, the timeout, a script error,
    /// or a returned value that is not a view.
    pub async fn search(&self, query: &str) -> Result<ViewTree, ScriptError> {
        let query = query.to_owned();
        let render = self.inner.renders.fetch_add(1, Ordering::Relaxed) + 1;
        let (tree, handlers) = self
            .run(move |inner| {
                let value: Dynamic = inner
                    .engine
                    .call_fn(&mut Scope::new(), &inner.ast, SEARCH_FN, (query,))
                    .map_err(|error| inner.error(&error))?;
                let mut ctx = Render {
                    render,
                    ast: &inner.ast,
                    grants: &inner.grants,
                    handlers: Vec::new(),
                };
                let view = ctx.view(&value)?;
                Ok((ViewTree::new(view), ctx.handlers))
            })
            .await?;

        let mut current = self.lock_handlers();
        if render > current.render {
            current.render = render;
            current.table = handlers.into_iter().collect();
        }
        Ok(tree)
    }

    /// Runs the action `request` names and says what the host should do next.
    ///
    /// Never fails in the `Result` sense: every outcome, including an unknown
    /// or stale token, a denied capability and a script error, is an
    /// [`ActionResponse`] the UI can show.
    pub async fn invoke(&self, request: &ActionRequest) -> ActionResponse {
        let invocation = request.invocation;
        let handler = self.lock_handlers().table.get(&request.handler).cloned();
        let Some(handler) = handler else {
            return ActionResponse::Failed {
                invocation,
                error: DispatchError::UnknownAction {
                    handler: request.handler.clone(),
                },
            };
        };

        match self.run(move |inner| inner.dispatch(handler)).await {
            Ok(effects) => ActionResponse::Completed {
                invocation,
                effects,
            },
            Err(ScriptError::CapabilityDenied(denial)) => ActionResponse::Failed {
                invocation,
                error: DispatchError::CapabilityDenied { denial },
            },
            Err(error) => ActionResponse::Failed {
                invocation,
                error: DispatchError::Failed {
                    extension: self.manifest.id.clone(),
                    message: error.to_string(),
                },
            },
        }
    }

    fn lock_handlers(&self) -> std::sync::MutexGuard<'_, Handlers> {
        self.inner
            .handlers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `f` on the blocking pool under the deadline, and gives up waiting
    /// at the timeout. Giving up is not the only stop: the same deadline is
    /// armed for `on_progress`, so the script itself is terminated and the
    /// thread comes back to the pool.
    async fn run<T, F>(&self, f: F) -> Result<T, ScriptError>
    where
        T: Send + 'static,
        F: FnOnce(&Inner) -> Result<T, ScriptError> + Send + 'static,
    {
        let inner = self.inner.clone();
        let timeout = inner.limits.timeout;
        let submitted = Instant::now();
        let task = tokio::task::spawn_blocking(move || {
            let _running = inner.running.lock().unwrap_or_else(PoisonError::into_inner);
            // The clock starts when the caller asked, not when this call got
            // its turn: a call queued behind a hung one must not then run for
            // a full timeout of its own after its caller has given up.
            let remaining = timeout.saturating_sub(submitted.elapsed());
            if remaining.is_zero() {
                return Err(ScriptError::Timeout(timeout));
            }
            inner.deadline.arm(remaining);
            let result = f(&inner);
            inner.deadline.disarm();
            result
        });
        match tokio::time::timeout(timeout, task).await {
            Ok(Ok(result)) => result,
            Ok(Err(join)) => Err(ScriptError::Crashed(join.to_string())),
            Err(_) => Err(ScriptError::Timeout(timeout)),
        }
    }
}

impl Inner {
    fn error(&self, error: &rhai::EvalAltResult) -> ScriptError {
        ScriptError::from_eval(error, self.limits.max_operations, self.limits.timeout)
    }

    fn dispatch(&self, handler: Handler) -> Result<Vec<ActionEffect>, ScriptError> {
        let host_call =
            |cap: &Capability| self.grants.get(cap).map_err(ScriptError::CapabilityDenied);
        let host_error = |error: crate::host::HostError| ScriptError::Runtime(error.0);
        match handler {
            Handler::Copy(text) => {
                let grant = host_call(&Capability::CLIPBOARD_WRITE)?;
                self.host
                    .clipboard_write(grant, &text)
                    .map_err(host_error)?;
                Ok(vec![ActionEffect::CloseWindow])
            }
            Handler::Paste(text) => {
                let grant = host_call(&Capability::CLIPBOARD_PASTE)?;
                self.host
                    .clipboard_paste(grant, &text)
                    .map_err(host_error)?;
                Ok(vec![ActionEffect::CloseWindow])
            }
            Handler::Open(target) => {
                let grant = host_call(&Capability::APPLICATION_OPEN)?;
                self.host.open(grant, &target).map_err(host_error)?;
                Ok(vec![ActionEffect::CloseWindow])
            }
            Handler::Named { name, args } => {
                let mut scope = Scope::new();
                let result = match args {
                    Some(args) => self.engine.call_fn(&mut scope, &self.ast, &name, (args,)),
                    None => self.engine.call_fn(&mut scope, &self.ast, &name, ()),
                };
                effects(&result.map_err(|error| self.error(&error))?)
            }
            Handler::Pointer { pointer, args } => {
                let result = match args {
                    Some(args) => pointer.call::<Dynamic>(&self.engine, &self.ast, (args,)),
                    None => pointer.call::<Dynamic>(&self.engine, &self.ast, ()),
                };
                effects(&result.map_err(|error| self.error(&error))?)
            }
        }
    }
}

/// What an action function returned, as effects. `()` is none; a map may set
/// `hud`, `toast` (with `message` and `style`), `rerender`, `pop`,
/// `pop_to_root` and `close`.
fn effects(value: &Dynamic) -> Result<Vec<ActionEffect>, ScriptError> {
    if value.is_unit() {
        return Ok(Vec::new());
    }
    let path = "the action's return value";
    let Some(map) = value.clone().try_cast::<rhai::Map>() else {
        return Err(ScriptError::InvalidView {
            path: path.to_owned(),
            message: format!("expected () or an object map, found {}", value.type_name()),
        });
    };
    const KEYS: &[&str] = &[
        "hud",
        "toast",
        "message",
        "style",
        "rerender",
        "pop",
        "pop_to_root",
        "close",
    ];
    if let Some(key) = map.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(ScriptError::InvalidView {
            path: format!("{path}.{key}"),
            message: format!("unknown effect; expected one of: {}", KEYS.join(", ")),
        });
    }
    let text = |key: &str| {
        map.get(key)
            .filter(|v| !v.is_unit())
            .map(ToString::to_string)
    };
    let flag = |key: &str| map.get(key).and_then(|v| v.as_bool().ok()).unwrap_or(false);

    let mut out = Vec::new();
    if let Some(title) = text("toast") {
        let style = match text("style").as_deref() {
            None | Some("info") => ToastStyle::Info,
            Some("success") => ToastStyle::Success,
            Some("failure") => ToastStyle::Failure,
            Some(other) => {
                return Err(ScriptError::InvalidView {
                    path: format!("{path}.style"),
                    message: format!("unknown toast style `{other}`"),
                });
            }
        };
        out.push(ActionEffect::Toast {
            style,
            title,
            message: text("message"),
        });
    }
    if flag("rerender") {
        out.push(ActionEffect::Rerender);
    }
    if flag("pop_to_root") {
        out.push(ActionEffect::PopToRoot);
    } else if flag("pop") {
        out.push(ActionEffect::PopView);
    }
    if let Some(text) = text("hud") {
        out.push(ActionEffect::Hud { text });
    } else if flag("close") {
        out.push(ActionEffect::CloseWindow);
    }
    Ok(out)
}
