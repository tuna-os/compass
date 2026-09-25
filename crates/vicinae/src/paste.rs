//! Pasting into the window the launcher hands focus back to: the engine's
//! `PasteService` (`src/server/src/services/paste`).
//!
//! On GNOME the Shell extension does the whole of it (it sets the clipboard,
//! waits for the focus change and presses the chord). On a wlroots
//! compositor the engine does it as the C++'s `LinuxPasteService` does: the
//! content goes on the clipboard over data-control, the wait for focus is
//! [`compass_core::paste::PasteService`]'s, and the chord is pressed by the
//! input server's `injectPaste` when the helper runs with injection, else on
//! a `zwp_virtual_keyboard_v1` keyboard ([`compass_wayland::virtual_keyboard`])
//! where the compositor has one. With neither, the content is copied and the
//! caller is told the paste could not happen, as the C++ copies first and
//! then says "the current platform cannot paste".

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use compass_core::input_server::wire::Call;
use compass_core::paste::{
    FOCUS_POLL_INTERVAL_MS, PasteEnvironment, PasteService, PasteTarget, Window,
};
use compass_ipc::{ErrorKind, ProtocolError, Response};
use compass_wayland::data_control::Offer;
use compass_wayland::toplevel::Toplevels;
use tokio::sync::RwLock;

use crate::serve::EngineState;
use crate::snippet_expansion::Expander;

/// What presses the paste chord.
#[derive(Clone)]
pub(crate) enum Injector {
    /// The input server's `injectPaste` (uinput), as the C++ uses.
    InputServer(Arc<Expander>),
    /// A virtual keyboard over the compositor's protocol.
    VirtualKeyboard,
}

impl Injector {
    /// Which one this is, for logs.
    const fn name(&self) -> &'static str {
        match self {
            Self::InputServer(_) => "input server",
            Self::VirtualKeyboard => "virtual keyboard",
        }
    }
}

/// The newest paste; an older one still waiting for focus gives up, as the
/// C++'s single timer is stopped by the next `pasteContent`.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The engine, for pastes asked for off the request path (an extension's
/// `Clipboard.paste`).
static ENGINE: OnceLock<Weak<RwLock<EngineState>>> = OnceLock::new();

/// Makes the engine reachable from [`paste_blocking`].
pub(crate) fn install(state: &Arc<RwLock<EngineState>>) {
    let _ = ENGINE.set(Arc::downgrade(state));
}

/// The wlroots session, when the clipboard is reached over data-control.
async fn data_control() -> Option<&'static crate::wlroots::Wlroots> {
    crate::wlroots::detect()
        .await
        .filter(|wlroots| wlroots.capabilities.data_control)
}

/// What would press the chord in this session: the running helper first,
/// as the C++ has only it, then the compositor's virtual keyboard.
pub(crate) async fn injector(
    state: &Arc<RwLock<EngineState>>,
    wlroots: &crate::wlroots::Wlroots,
) -> Option<Injector> {
    let expander = state.read().await.expander();
    if let Some(expander) = expander {
        let status = expander.server().status();
        if status.running && status.injection {
            return Some(Injector::InputServer(expander));
        }
    }
    wlroots
        .capabilities
        .virtual_keyboard
        .then_some(Injector::VirtualKeyboard)
}

/// Whether a paste would be pressed rather than only copied: the Shell
/// extension, or data-control with something to press the chord.
pub(crate) async fn can_paste(state: &Arc<RwLock<EngineState>>) -> bool {
    if let Some(wlroots) = data_control().await {
        return injector(state, wlroots).await.is_some();
    }
    state.read().await.shell_client().is_some()
}

/// The refusal when there is no clipboard to paste through at all (no
/// data-control, no session bus), asked before anything is looked up.
pub(crate) async fn no_clipboard(state: &Arc<RwLock<EngineState>>, what: &str) -> Option<Response> {
    if data_control().await.is_some() || state.read().await.shell_client().is_some() {
        return None;
    }
    Some(Response::Error(crate::window_service::no_bus(what)))
}

/// `content` as data-control offers: text as UTF-8 text, anything else under
/// its own type.
fn offers(content: &compass_shell::ClipboardContent) -> Vec<Offer> {
    vec![Offer {
        mime_type: content.mime_type.clone(),
        data: content.data.clone(),
    }]
}

/// `pasteContent`: puts `content` on the clipboard and pastes it into the
/// window focus returns to. `what` names the action in a refusal.
pub(crate) async fn paste(
    state: &Arc<RwLock<EngineState>>,
    content: compass_shell::ClipboardContent,
    what: &str,
) -> Response {
    if let Some(wlroots) = data_control().await {
        return match paste_over_data_control(state, wlroots, offers(&content)).await {
            Ok(()) => Response::Ack,
            Err(error) => Response::Error(error),
        };
    }
    let (shell, terminals) = {
        let state = state.read().await;
        (
            state.shell_client(),
            compass_core::app_service::AppService::new(state.app_index()).terminal_window_classes(),
        )
    };
    let Some(shell) = shell else {
        return Response::Error(crate::window_service::no_bus(what));
    };
    let terminals: Vec<&str> = terminals.iter().map(String::as_str).collect();
    let pasted = match shell.set_clipboard(&content).await {
        Ok(()) => shell.paste(&terminals).await,
        Err(err) => Err(err),
    };
    match pasted {
        Ok(()) => Response::Ack,
        Err(err) => Response::Error(crate::window_service::refusal(&err, what)),
    }
}

/// An extension's `Clipboard.paste` on a wlroots compositor, from its host
/// thread: the offers are copied and the chord pressed as [`paste`] does.
/// Without the engine (a host run on its own) they are only copied.
pub(crate) fn paste_blocking(handle: &tokio::runtime::Handle, offers: Vec<Offer>) {
    let state = ENGINE.get().and_then(Weak::upgrade);
    let Some(wlroots) = crate::wlroots::session().filter(|w| w.capabilities.data_control) else {
        return;
    };
    let result = match state {
        Some(state) => handle.block_on(paste_over_data_control(&state, wlroots, offers)),
        None => compass_wayland::clipboard::set(offers)
            .map_err(|err| ProtocolError::new(ErrorKind::Internal, err.to_string())),
    };
    if let Err(error) = result {
        tracing::info!(error = %error.message, "paste");
    }
}

/// The wlroots half of `pasteContent`. Answers once the content is on the
/// clipboard; the chord follows when focus has landed.
async fn paste_over_data_control(
    state: &Arc<RwLock<EngineState>>,
    wlroots: &'static crate::wlroots::Wlroots,
    offers: Vec<Offer>,
) -> Result<(), ProtocolError> {
    let injector = injector(state, wlroots).await;
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let env = Env {
        offers: Cell::new(Some(offers)),
        copied: Cell::new(Ok(())),
        injector,
        toplevels: wlroots.toplevels.clone(),
        handoff: handoff_detection(wlroots),
        state: Arc::clone(state),
        handle: tokio::runtime::Handle::current(),
        generation,
        polling: Cell::new(false),
        scheduled: Cell::new(None),
        terminal: Cell::new(false),
    };
    tokio::task::spawn_blocking(move || {
        let mut service = PasteService::new(env);
        let scheduled = service.paste_content(&());
        let copied = service.env().copied.replace(Ok(()));
        let _ = tx.send((copied, scheduled, service.supports_paste()));
        if scheduled {
            drive(&mut service);
        }
    });
    let (copied, _, supported) = rx.await.map_err(|_| {
        ProtocolError::new(ErrorKind::Internal, "the paste task ended before copying")
    })?;
    copied?;
    if !supported {
        return Err(ProtocolError::new(
            ErrorKind::Unsupported,
            "Pasting needs the input server or a compositor with zwp_virtual_keyboard_v1; \
             the content was copied instead",
        ));
    }
    Ok(())
}

/// Whether focus landing can be seen: the toplevel list carries which window
/// is active, and the launcher is itself a toplevel in it. A layer-shell
/// launcher is not in the list, so the window under it reads as focused
/// before the launcher has closed; there the C++'s blind delay is used.
fn handoff_detection(wlroots: &crate::wlroots::Wlroots) -> bool {
    let toplevel_launcher = !wlroots.capabilities.layer_shell
        || compass_wayland::layer_shell::override_disables(
            std::env::var(compass_wayland::layer_shell::OVERRIDE_ENV)
                .ok()
                .as_deref(),
        );
    wlroots.toplevels.is_some() && wlroots.capabilities.toplevel_management && toplevel_launcher
}

/// Runs the service's timers: the focus poll, then the delayed paste.
fn drive(service: &mut PasteService<Env>) {
    let current =
        |service: &PasteService<Env>| GENERATION.load(Ordering::SeqCst) == service.env().generation;
    loop {
        if !current(service) {
            tracing::debug!("a newer paste replaced this one");
            return;
        }
        if service.env().polling.get() {
            std::thread::sleep(Duration::from_millis(FOCUS_POLL_INTERVAL_MS));
            service.poll_focus();
            continue;
        }
        let Some(delay) = service.env().scheduled.take() else {
            tracing::info!("paste dropped: focus never landed on another window");
            return;
        };
        std::thread::sleep(Duration::from_millis(delay));
        if current(service) {
            service.execute_paste();
        }
        return;
    }
}

/// The world the service runs in, on one blocking thread.
struct Env {
    offers: Cell<Option<Vec<Offer>>>,
    copied: Cell<Result<(), ProtocolError>>,
    injector: Option<Injector>,
    toplevels: Option<Arc<Toplevels>>,
    handoff: bool,
    state: Arc<RwLock<EngineState>>,
    handle: tokio::runtime::Handle,
    generation: u64,
    polling: Cell<bool>,
    scheduled: Cell<Option<u64>>,
    terminal: Cell<bool>,
}

impl Env {
    fn active(&self, own: bool) -> Option<Window> {
        self.toplevels
            .as_ref()?
            .list()
            .into_iter()
            .find(|window| {
                window.activated && (own || !window.app_id.eq_ignore_ascii_case(compass_ui::APP_ID))
            })
            .map(|window| Window {
                title: window.title,
                wm_class: window.app_id,
            })
    }
}

impl PasteEnvironment for Env {
    type Content = ();

    fn copy(&self, (): &()) -> bool {
        let Some(offers) = self.offers.take() else {
            return false;
        };
        let copied = compass_wayland::clipboard::set(offers).map_err(|err| {
            ProtocolError::new(ErrorKind::Internal, format!("Copying failed: {err}"))
        });
        let ok = copied.is_ok();
        self.copied.set(copied);
        ok
    }

    fn supports_paste(&self) -> bool {
        self.injector.is_some()
    }

    fn supports_focus_handoff_detection(&self) -> bool {
        self.handoff
    }

    fn focused_foreign_window(&self) -> Option<Window> {
        self.active(false)
    }

    fn focused_window(&self) -> Option<Window> {
        self.active(true)
    }

    fn find_app(&self, wm_class: &str) -> Option<String> {
        let found = self.handle.block_on(async {
            let state = self.state.read().await;
            compass_core::app_service::AppService::new(state.app_index())
                .find_by_class(wm_class)
                .map(|app| {
                    (
                        app.desktop_id().to_owned(),
                        app.categories().iter().any(|c| c == "TerminalEmulator"),
                    )
                })
        });
        self.terminal.set(found.as_ref().is_some_and(|(_, t)| *t));
        found.map(|(id, _)| id)
    }

    fn schedule_execute(&self, delay_ms: u64) {
        self.scheduled.set(Some(delay_ms));
    }

    fn start_focus_polling(&self) {
        self.polling.set(true);
    }

    fn stop_focus_polling(&self) {
        self.polling.set(false);
    }

    fn paste_to_app(&self, target: &PasteTarget) -> bool {
        let Some(injector) = &self.injector else {
            return false;
        };
        let terminal = target.app.is_some() && self.terminal.get();
        tracing::info!(
            window = target
                .window
                .as_ref()
                .map_or("<unknown>", |w| w.wm_class.as_str()),
            app = target.app.as_deref().unwrap_or("<none>"),
            terminal,
            by = injector.name(),
            "pasting"
        );
        match injector {
            Injector::InputServer(expander) => {
                let reply = self
                    .handle
                    .block_on(expander.server().call(Call::InjectPaste { terminal }));
                matches!(reply, Some(Ok(_)))
            }
            Injector::VirtualKeyboard => match compass_wayland::virtual_keyboard::paste(terminal) {
                Ok(()) => true,
                Err(error) => {
                    tracing::warn!(%error, "the paste chord could not be pressed");
                    false
                }
            },
        }
    }

    // The C++ puts back the selection it saw before the paste; nothing here
    // tracks one, so the pasted content stays on the clipboard (PARITY.md).
    fn schedule_clipboard_restore(&self) {}
}
