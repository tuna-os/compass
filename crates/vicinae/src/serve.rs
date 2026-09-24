//! The engine: the thing that actually runs.
//!
//! Everything else in this workspace is a library, and every `vicinae`
//! subcommand until now has been a *client* looking for a server that did not
//! exist. This module is that server.
//!
//! # Headless on purpose
//!
//! This engine has no window of its own and never opens one. It serves the
//! requests that do not need one — [`Request::Ping`], [`Request::Query`],
//! [`Request::Doctor`], [`Request::Shutdown`] — always, on any machine,
//! display or not.
//!
//! [`Request::Show`], [`Request::Hide`] and [`Request::Toggle`] are forwarded
//! to a **resident launcher window** that attached itself over the same socket
//! ([ADR-0015](../../../docs/rust-engine/adr/0015-the-launcher-window-is-resident.md)).
//! With no window attached they are **refused**, with [`ErrorKind::Unsupported`]
//! and a message saying how to start one.
//!
//! Refusing matters more than it looks. `Response::Ack` means "the side effect
//! was performed"; answering `Ack` to `Show` when nothing can be shown would
//! make a client unable to tell a working engine from a windowless one, and
//! would make UI bring-up debug a lie instead of a gap.
//!
//! What it *does* serve is a genuine vertical slice — index the machine's
//! applications, rank a query against them with frecency, answer over the same
//! socket the C++ engine uses — and all of it is exercisable with no display,
//! which is why it is the part built first.

use std::sync::Arc;

use anyhow::{Context, Result};
use compass_core::{AppIndex, Config, FrecencyStore, JsonFrecencyStore, SystemClock};
use compass_ipc::{
    ErrorKind, Listener, ProtocolError, QueryHit, Request, Response, SocketPath, WindowCommand,
    WindowLink, WindowOutcome, protocol::PROTOCOL_VERSION,
};
use tokio::sync::{Mutex, RwLock};

use crate::doctor;
use crate::engine::Engine;

/// The at-most-one launcher window this engine drives.
///
/// # Last window wins
///
/// A second `vicinae ui` **replaces** the first rather than being refused.
/// Dropping the old [`WindowLink`] closes its socket, so that window's
/// `next_command` returns `None` and it exits its loop.
///
/// The alternative — refuse the newcomer — reads better until you ask what
/// happens when a window wedges without closing its socket: the engine would
/// hold a link that answers nothing and refuse every replacement, and the only
/// way out would be restarting the daemon. Replacing has no state that can get
/// stuck. It also means the handler needs no reservation between answering
/// `WindowAttached` and the link actually arriving, which is a window (however
/// narrow) in which an attach that died mid-handshake could strand the slot.
///
/// # Death is noticed on the next push, not before
///
/// Nothing polls the link. A window that dies is discovered when the engine
/// next tries to push to it, at which point the slot is cleared and the request
/// is refused with [`no_window`] — correctly, since by then there is none.
/// Proactively watching would mean a second reader on a socket whose only
/// reader is [`WindowLink::push`]'s reply, so it would have to be built into
/// the link rather than bolted beside it.
type WindowSlot = Arc<Mutex<Option<WindowLink>>>;

/// Where launch history lives, under `$XDG_DATA_HOME`.
///
/// Returns `None` when there is no data home to put it in, which is the
/// read-only-session case the caller degrades into an in-memory store for.
fn frecency_path() -> Option<std::path::PathBuf> {
    Some(compass_core::xdg_dirs::data_home()?.join("vicinae/frecency.json"))
}

/// The engine's state, shared across connections.
///
/// `RwLock` rather than `Mutex` because queries are the hot path and they only
/// read; the write side is frecency updates and, later, reindexing.
pub struct EngineState {
    index: AppIndex,
    frecency: Box<dyn FrecencyStore + Send + Sync>,
    socket: SocketPath,
    /// How many hits a query answers with. `launcher.max_results` from the
    /// user's config, so the wire honours the same limit the UI would.
    max_results: usize,
    /// The attached launcher window, if one is.
    window: WindowSlot,
    /// Clipboard history, once [`crate::clipboard_service::run`] has opened
    /// it. `None` until then, and for good when there is no keyring.
    clipboard: Option<Arc<crate::clipboard_service::ClipboardStore>>,
}

// Hand-written because `dyn FrecencyStore` is not `Debug`, and widening that
// trait to require it would push a formatting concern onto every future store
// implementation for the sake of one log line.
impl std::fmt::Debug for EngineState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineState")
            .field("applications", &self.index.len())
            .field("socket", &self.socket)
            .field("max_results", &self.max_results)
            // Deliberately not the link itself: formatting it would need the
            // lock, and `Debug` is used from log lines that must not block.
            .finish_non_exhaustive()
    }
}

impl EngineState {
    /// Builds the state by indexing this machine.
    ///
    /// A frecency store that cannot be loaded is **not** fatal: the launcher
    /// works without launch history, just less well ordered, and refusing to
    /// start over a corrupt ranking cache would be a worse failure than the one
    /// it is reporting.
    pub fn from_environment(socket: SocketPath) -> Self {
        let mut index = AppIndex::from_environment();
        tracing::info!(applications = index.len(), "indexed applications");

        // A bad config is reported and then ignored rather than fatal. Refusing
        // to start because `max_results` is misspelled would be a worse outcome
        // than starting with the default and saying so.
        let config = match Config::load() {
            Ok(config) => config,
            Err(err) => {
                let fallback = Config::default();
                tracing::warn!(error = %err, "using default configuration");
                fallback
            }
        };
        let max_results = config.launcher().max_results();
        index.apply_root_config(&config.root_config());

        let frecency = Self::open_frecency();

        Self {
            index,
            frecency,
            socket,
            max_results,
            window: WindowSlot::default(),
            clipboard: None,
        }
    }

    /// Opens the launch-history store, degrading to in-memory.
    ///
    /// Not fatal on failure: the launcher works without launch history, just
    /// less well ordered, and refusing to start over a corrupt ranking cache
    /// would be a worse failure than the one it is reporting.
    fn open_frecency() -> Box<dyn FrecencyStore + Send + Sync> {
        let clock = std::sync::Arc::new(SystemClock);
        match frecency_path() {
            Some(path) => match JsonFrecencyStore::open(&path) {
                Ok(store) => Box::new(store),
                Err(err) => {
                    tracing::warn!(
                        path = %path.display(), error = %err,
                        "launch history unreadable; ranking by match score alone this session"
                    );
                    Box::new(JsonFrecencyStore::in_memory(clock))
                }
            },
            None => {
                tracing::warn!("no XDG data home; launch history will not persist");
                Box::new(JsonFrecencyStore::in_memory(clock))
            }
        }
    }

    /// Builds state around an already-built index, for tests.
    #[must_use]
    pub fn with_index(
        index: AppIndex,
        frecency: Box<dyn FrecencyStore + Send + Sync>,
        socket: SocketPath,
        max_results: usize,
    ) -> Self {
        Self {
            index,
            frecency,
            socket,
            max_results,
            window: WindowSlot::default(),
            clipboard: None,
        }
    }

    /// Makes clipboard history available to requests.
    pub fn set_clipboard(&mut self, store: Arc<crate::clipboard_service::ClipboardStore>) {
        self.clipboard = Some(store);
    }

    /// The slot holding the attached launcher window.
    ///
    /// Cloned rather than borrowed because the attach callback outlives any
    /// borrow of the state: it runs on the connection's own task for as long
    /// as that window lives.
    #[must_use]
    pub fn window_slot(&self) -> WindowSlot {
        Arc::clone(&self.window)
    }

    /// Number of indexed applications.
    #[must_use]
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Whether nothing was indexed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Ranks `text` against the index.
    ///
    /// # Hits can be ordered "out of order" by score, and that is correct
    ///
    /// Ordering comes from the *combined* score — match quality plus the
    /// frecency boost — while the score reported on the wire is the match score
    /// alone, because that is what the protocol documents and what a client can
    /// interpret. So a frequently launched item with score 90 legitimately
    /// appears above a never-launched one with 95. A client that sorts by the
    /// reported score would undo the ranking; it should preserve the order it
    /// is given.
    #[must_use]
    pub fn query(&self, text: &str) -> Vec<QueryHit> {
        self.index
            .search_root_all(text, Some(self.frecency.as_ref()))
            .into_iter()
            .take(self.max_results)
            .map(|hit| match hit {
                compass_core::RootHit::App(ranked) => app_hit(&ranked),
                compass_core::RootHit::Command {
                    command,
                    match_score,
                } => QueryHit {
                    id: command.id(),
                    title: command.title.to_owned(),
                    subtitle: Some(command.subtitle.to_owned()),
                    score: match_score,
                },
            })
            .collect()
    }
}

fn app_hit(ranked: &compass_core::apps::ApplicationRootHit<'_>) -> QueryHit {
    QueryHit {
        // The ENTRYPOINT id, not the desktop key. The protocol
        // documents this field as the "stable identifier of the
        // underlying root item", and `AppIndex` already builds one
        // (`app_root_item` -> `entrypoint_id(APPS_PROVIDER_ID, ...)`);
        // this put `AppItem::key` there instead, so the wire carried
        // `host--byobu.desktop` for the item every other part of the
        // system — and the C++ engine — calls
        // `applications:host--byobu`.
        id: ranked.entrypoint_id.to_owned(),
        title: ranked.item.display_name(),
        subtitle: None,
        // `match_score`, not `score`. `Ranked::score` is the combined
        // value that includes the frecency boost and is not bounded,
        // whereas the wire documents `0..=100` on compass-search's
        // scale. See the ordering note on `query`.
        score: ranked.match_score,
    }
}

/// The three requests that need a launcher window, when none is connected.
///
/// [ADR-0015](../../../docs/rust-engine/adr/0015-the-launcher-window-is-resident.md)
/// makes the window a resident process that connects to this daemon, so the
/// honest answer is about a *connection* rather than about the build. That is
/// the more useful thing to be told: "start the window" is actionable, "this
/// engine is headless" was not.
///
/// The property this preserves is the one that matters. A client can still
/// tell "no window" from "the window was shown", which is the only thing
/// standing between an honest gap and a `toggle` that silently does nothing.
fn no_window(what: &str) -> Response {
    Response::Error(ProtocolError::new(
        ErrorKind::Unsupported,
        format!(
            "cannot {what}: no launcher window is connected to this engine. \
             Start one with `vicinae ui`. Query and doctor work without it."
        ),
    ))
}

/// Forwards `command` to the attached window, if there is one.
///
/// Clears the slot when the push fails. That is **hygiene, not behaviour**: a
/// push to a dead socket fails anyway, so the refusal a client sees is the same
/// either way — a control confirmed the end-to-end tests pass with the clearing
/// removed. What it buys is releasing the file descriptor and not paying a
/// doomed write on every subsequent request. See [`WindowSlot`].
pub(crate) async fn forward(slot: &WindowSlot, command: WindowCommand, what: &str) -> Response {
    let mut guard = slot.lock().await;

    let Some(link) = guard.as_mut() else {
        return no_window(what);
    };

    match link.push(command).await {
        Ok(WindowOutcome::Shown | WindowOutcome::Hidden) => Response::Ack,
        Ok(WindowOutcome::Failed(reason)) => {
            // The window is alive and said no. Keeping the link is the point:
            // a compositor that refused one activation will very likely accept
            // the next, and dropping the window over it would turn a recoverable
            // refusal into a dead launcher.
            tracing::warn!(reason = %reason, "the launcher window refused a command");
            Response::Error(ProtocolError::new(
                ErrorKind::Internal,
                format!("the launcher window could not {what}: {reason}"),
            ))
        }
        Err(err) => {
            tracing::info!(error = %err, "the launcher window went away");
            *guard = None;
            no_window(what)
        }
    }
}

/// Answers one request.
///
/// Separated from the serve loop so the whole request surface is testable
/// without a socket, and so the serve loop stays about lifecycle.
pub async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    match request {
        Request::Ping => Response::Pong {
            protocol_version: PROTOCOL_VERSION,
            pid: std::process::id(),
        },

        Request::Query { text } => {
            let state = state.read().await;
            Response::QueryResults {
                hits: state.query(&text),
            }
        }

        Request::Doctor => {
            let (socket, engine) = {
                let state = state.read().await;
                (state.socket.clone(), Engine::Rust)
            };
            Response::DoctorReport {
                checks: doctor::run_on_this_machine(&socket, engine).await.checks,
            }
        }

        Request::RecordLaunch { key } => {
            let state = Arc::clone(state);
            // JSON persistence is blocking disk work. Keep it off the async
            // executor, and serialize updates through the daemon's store.
            tokio::task::spawn_blocking(move || {
                let mut state = state.blocking_write();
                if state.index.get(&key).is_none() && compass_core::commands::by_id(&key).is_none()
                {
                    return Response::Error(ProtocolError::new(
                        ErrorKind::BadRequest,
                        "launch key is neither an application nor a builtin command",
                    ));
                }
                match state.frecency.record_launch(&key) {
                    Ok(()) => Response::Ack,
                    Err(error) => {
                        tracing::warn!(%error, "could not persist launch history");
                        Response::Error(ProtocolError::new(
                            ErrorKind::Internal,
                            "could not persist launch history",
                        ))
                    }
                }
            })
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "launch history task failed");
                Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    "launch history task failed",
                ))
            })
        }

        Request::ClipboardHistory { query, limit } => {
            if limit == 0 {
                return Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "a clipboard history request must ask for at least one entry",
                ));
            }
            let Some(store) = state.read().await.clipboard.clone() else {
                return Response::Error(ProtocolError::new(
                    ErrorKind::Unsupported,
                    "clipboard history is unavailable: no keyring, or the store would not open \
                     (the engine log says which)",
                ));
            };
            // SQLite is blocking I/O; keep it off the executor.
            match tokio::task::spawn_blocking(move || store.history(&query, limit)).await {
                Ok(Ok(entries)) => Response::ClipboardHistory { entries },
                Ok(Err(err)) => {
                    Response::Error(ProtocolError::new(ErrorKind::Internal, err.to_string()))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("clipboard history task failed: {err}"),
                )),
            }
        }

        Request::ClipboardContent { id } => {
            let Some(store) = state.read().await.clipboard.clone() else {
                return Response::Error(ProtocolError::new(
                    ErrorKind::Unsupported,
                    "clipboard history is unavailable: no keyring, or the store would not open \
                     (the engine log says which)",
                ));
            };
            match tokio::task::spawn_blocking(move || store.content(&id)).await {
                Ok(Ok(Some((mime_type, data)))) => Response::ClipboardContent { mime_type, data },
                Ok(Ok(None)) => Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "no clipboard history entry has that id",
                )),
                Ok(Err(err)) => {
                    Response::Error(ProtocolError::new(ErrorKind::Internal, err.to_string()))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("clipboard content task failed: {err}"),
                )),
            }
        }

        // Handled by the serve loop, which owns the shutdown signal; reaching
        // here means the loop did not intercept it.
        Request::Shutdown => Response::ShuttingDown,

        Request::Toggle | Request::Show | Request::Hide => {
            let (slot, command, what) = {
                let state = state.read().await;
                match request {
                    Request::Toggle => (
                        state.window_slot(),
                        WindowCommand::Toggle,
                        "toggle a window",
                    ),
                    Request::Show => (state.window_slot(), WindowCommand::Show, "show a window"),
                    _ => (state.window_slot(), WindowCommand::Hide, "hide a window"),
                }
            };
            // The read lock is released before the push: a window that takes a
            // moment to answer must not block queries from other clients.
            forward(&slot, command, what).await
        }

        // Accepting is the whole decision: the transport reads this response,
        // writes it, and then hands the connection over as a `WindowLink`. The
        // engine never refuses an attach — see `WindowSlot` for why replacing
        // beats refusing.
        Request::AttachWindow => Response::WindowAttached,

        // An outcome is a *reply* on an attached connection, never a request.
        // Arriving here means a peer sent one on an ordinary connection, which
        // is a confused client rather than a window.
        Request::WindowOutcome(outcome) => Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            format!(
                "{outcome:?} is a reply to a pushed window command, not a request;                  send AttachWindow first and answer the commands that follow"
            ),
        )),
    }
}

/// Binds the socket and serves until shutdown.
///
/// Returns when a client sends [`Request::Shutdown`] or the process is asked to
/// terminate. The socket file is removed on the way out by [`Listener`]'s drop.
///
/// `hotkey` asks the engine to bind the global launcher shortcut. Turning it
/// off is not only a test affordance: on GNOME, binding means a permission
/// prompt, and a user whose compositor already binds a key to `vicinae toggle`
/// should not be asked for one they will not use.
pub async fn run(socket: &SocketPath, hotkey: bool) -> Result<()> {
    // Before binding, not after: on the `/tmp` fallback the directory may
    // already exist and belong to someone else, and `DirBuilder::recursive`
    // adopts a directory rather than correcting it. See #88.
    socket
        .ensure_private_parent()
        .with_context(|| format!("checking the engine socket directory for {socket}"))?;

    let listener = Listener::bind(socket.as_path())
        .await
        .with_context(|| format!("binding the engine socket at {socket}"))?;

    let state = Arc::new(RwLock::new(EngineState::from_environment(socket.clone())));
    tracing::info!(socket = %socket, "engine listening");

    // A channel rather than a flag: `Shutdown` must be answered *before* the
    // loop stops, or the client sees its connection drop and reports a crash
    // instead of a clean stop.
    let (stop_tx, mut stop_rx) = tokio::sync::mpsc::channel::<()>(1);

    let window_slot = state.read().await.window_slot();

    // Detached: the hotkey is a convenience, the socket is the contract. A
    // portal that never answers must not keep the engine from listening, so
    // this is spawned rather than awaited or raced against the serve loop.
    // Detached for the same reason: a keyring that is slow or absent costs
    // clipboard history, never the socket.
    tokio::spawn(crate::clipboard_service::run(Arc::clone(&state)));

    if hotkey {
        tokio::spawn(crate::hotkey::run(Arc::clone(&state)));
    } else {
        tracing::info!("not binding the launcher hotkey (--no-hotkey)");
    }

    let serving = {
        let state = Arc::clone(&state);
        listener.serve_with_shutdown_attach(
            move |request| {
                let state = Arc::clone(&state);
                let stop_tx = stop_tx.clone();
                async move {
                    if matches!(request, Request::Shutdown) {
                        tracing::info!("shutdown requested by a client");
                        // Buffered, so the response is written first and this
                        // never blocks the handler.
                        let _ = stop_tx.try_send(());
                        return Response::ShuttingDown;
                    }
                    handle(&state, request).await
                }
            },
            move |link| {
                let window_slot = Arc::clone(&window_slot);
                async move {
                    tracing::info!("a launcher window attached");
                    // Replaces any previous window; see `WindowSlot`. The old
                    // link drops here, closing that window's socket.
                    let replaced = window_slot.lock().await.replace(link).is_some();
                    if replaced {
                        tracing::info!("the previous launcher window was replaced");
                    }
                }
            },
            async move {
                let _ = stop_rx.recv().await;
            },
        )
    };

    serving.await.context("serving the engine socket")?;
    tracing::info!("engine stopped");
    Ok(())
}
