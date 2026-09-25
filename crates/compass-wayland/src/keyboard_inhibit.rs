//! `zwp_keyboard_shortcuts_inhibit_manager_v1`: while the launcher records a
//! shortcut, the compositor's own shortcuts go to the launcher instead.
//!
//! Ports `WaylandShortcutInhibitManager` (`services/shortcut-inhibit`),
//! which the C++'s shortcut recorder turns on while it captures
//! (`ShortcutInhibitor.enabled: capture.capturing`).
//!
//! # Which surface, without a pointer to it
//!
//! An inhibitor is made for a `wl_surface`, and the launcher's belongs to
//! the toolkit (`iced_layershell`), which hands it out only as a raw pointer
//! that this workspace may not turn into a proxy (`unsafe`). So this does not
//! ask for the surface; it is *told* it. The launcher gives the toolkit a
//! connection it made itself ([`Connection`] is shared by cloning), and this
//! binds its own `wl_keyboard` on that connection: the compositor sends its
//! `enter` with the surface that just took the keyboard, as a proxy on the
//! same connection. While shortcuts are [wanted](ShortcutInhibit::set_wanted),
//! whichever of the launcher's surfaces holds the keyboard gets the
//! inhibitor; it is destroyed when the keyboard leaves or the wish ends.
//!
//! This never reads the socket once bound: the toolkit does, and the events
//! it reads for this queue wait there until [`ShortcutInhibit::dispatch_pending`]
//! takes them (see there for why).
//!
//! The C++ keeps an inhibitor for its window until the window goes and lets
//! the compositor deactivate it while the window is not focused; this makes
//! one on focus and drops it on leave, which is the same keyboard for the
//! person and needs no knowledge of when the toolkit destroys a surface.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_keyboard, wl_registry, wl_seat, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::{
    zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1,
    zwp_keyboard_shortcuts_inhibitor_v1::{self, ZwpKeyboardShortcutsInhibitorV1},
};

/// Errors binding the manager.
#[derive(Debug, thiserror::Error)]
pub enum InhibitError {
    /// Compositor doesn't support keyboard-shortcuts-inhibit-v1.
    #[error("compositor doesn't support keyboard-shortcuts-inhibit-v1")]
    Unsupported,
    /// The registry could not be read or the connection failed.
    #[error("the compositor connection: {0}")]
    Connection(String),
}

/// Where the inhibitor is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InhibitState {
    /// No inhibitor.
    #[default]
    None,
    /// One was made; the compositor has not said whether it applies.
    Requested,
    /// The compositor sends the compositor's shortcuts to the surface.
    Active,
    /// The compositor keeps its shortcuts (the person or a policy said so).
    Inactive,
}

#[derive(Default)]
struct Shared {
    wanted: bool,
    focused: Option<wl_surface::WlSurface>,
    inhibitor: Option<ZwpKeyboardShortcutsInhibitorV1>,
    state: InhibitState,
    made: u32,
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the dispatcher and the handle both see.
#[derive(Clone)]
struct Core {
    shared: Arc<Mutex<Shared>>,
    manager: ZwpKeyboardShortcutsInhibitManagerV1,
    seat: wl_seat::WlSeat,
    qh: QueueHandle<Dispatcher>,
    connection: Connection,
}

impl Core {
    /// Makes or drops the inhibitor so it matches the wish and the focus.
    fn reconcile(&self, shared: &mut Shared) {
        let target = shared.focused.as_ref().filter(|_| shared.wanted);
        match (target, &shared.inhibitor) {
            (Some(surface), None) if surface.is_alive() => {
                shared.inhibitor =
                    Some(
                        self.manager
                            .inhibit_shortcuts(surface, &self.seat, &self.qh, ()),
                    );
                shared.state = InhibitState::Requested;
                shared.made += 1;
            }
            (None, Some(_)) => {
                if let Some(inhibitor) = shared.inhibitor.take() {
                    inhibitor.destroy();
                }
                shared.state = InhibitState::None;
            }
            _ => {}
        }
        let _ = self.connection.flush();
    }
}

/// The event side: reads the keyboard's focus and the inhibitor's state.
pub struct Dispatcher {
    core: Core,
}

/// The inhibit manager on one connection, with its own event queue.
pub struct ShortcutInhibit {
    queue: EventQueue<Dispatcher>,
    dispatcher: Dispatcher,
}

impl ShortcutInhibit {
    /// Binds the manager, the first seat and a keyboard on `connection`, and
    /// reads the keyboard's current focus.
    ///
    /// # Errors
    ///
    /// [`InhibitError::Unsupported`] without the manager or a seat,
    /// [`InhibitError::Connection`] when the registry cannot be read.
    pub fn bind(connection: &Connection) -> Result<Self, InhibitError> {
        let (globals, queue) = registry_queue_init::<Dispatcher>(connection)
            .map_err(|err| InhibitError::Connection(err.to_string()))?;
        let qh = queue.handle();
        let manager = globals
            .bind::<ZwpKeyboardShortcutsInhibitManagerV1, _, _>(&qh, 1..=1, ())
            .map_err(|_| InhibitError::Unsupported)?;
        let seat = globals
            .bind::<wl_seat::WlSeat, _, _>(&qh, 1..=5, ())
            .map_err(|_| InhibitError::Unsupported)?;
        let mut this = Self {
            queue,
            dispatcher: Dispatcher {
                core: Core {
                    shared: Arc::default(),
                    manager,
                    seat,
                    qh,
                    connection: connection.clone(),
                },
            },
        };
        // The seat's capabilities arrive first; the keyboard is asked for
        // there, and its `enter` on the next roundtrip.
        this.roundtrip()?;
        this.roundtrip()?;
        Ok(this)
    }

    /// The handle the launcher sets the wish through.
    #[must_use]
    pub fn handle(&self) -> InhibitHandle {
        InhibitHandle {
            core: self.dispatcher.core.clone(),
        }
    }

    /// Sends what is queued and reads the compositor's answers.
    ///
    /// # Errors
    ///
    /// [`InhibitError::Connection`] when the connection failed.
    pub fn roundtrip(&mut self) -> Result<(), InhibitError> {
        self.queue
            .roundtrip(&mut self.dispatcher)
            .map(drop)
            .map_err(|err| InhibitError::Connection(err.to_string()))
    }

    /// Handles the events already read for this queue, without reading.
    ///
    /// On a connection the toolkit reads, this is the only way to take them:
    /// a second thread blocking on the same socket can read the toolkit's
    /// events for it and leave the toolkit waiting on a socket with nothing
    /// left in it, so nothing here ever reads.
    ///
    /// # Errors
    ///
    /// [`InhibitError::Connection`] when the connection failed.
    pub fn dispatch_pending(&mut self) -> Result<(), InhibitError> {
        self.queue
            .dispatch_pending(&mut self.dispatcher)
            .map(drop)
            .map_err(|err| InhibitError::Connection(err.to_string()))
    }

    /// [`InhibitHandle::set_wanted`], after taking the focus changes already
    /// read.
    pub fn set_wanted(&mut self, wanted: bool) {
        if let Err(error) = self.dispatch_pending() {
            tracing::info!(%error, "the shortcut inhibitor's connection");
        }
        self.handle().set_wanted(wanted);
    }
}

/// Sets whether shortcuts are inhibited, from any thread.
#[derive(Clone)]
pub struct InhibitHandle {
    core: Core,
}

impl std::fmt::Debug for InhibitHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InhibitHandle")
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

impl InhibitHandle {
    /// Inhibit the compositor's shortcuts on whichever of this connection's
    /// surfaces holds the keyboard (`true`), or stop (`false`).
    pub fn set_wanted(&self, wanted: bool) {
        let mut shared = lock(&self.core.shared);
        if shared.wanted == wanted {
            return;
        }
        shared.wanted = wanted;
        self.core.reconcile(&mut shared);
    }

    /// Whether inhibition is wanted.
    #[must_use]
    pub fn wanted(&self) -> bool {
        lock(&self.core.shared).wanted
    }

    /// Whether a surface of this connection holds the keyboard.
    #[must_use]
    pub fn focused(&self) -> bool {
        lock(&self.core.shared).focused.is_some()
    }

    /// Where the inhibitor is.
    #[must_use]
    pub fn state(&self) -> InhibitState {
        lock(&self.core.shared).state
    }

    /// How many inhibitors have been made, for tests and logs.
    #[must_use]
    pub fn made(&self) -> u32 {
        lock(&self.core.shared).made
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Dispatcher {
    fn event(
        _: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: wayland_client::WEnum::Value(capabilities),
        } = event
            && capabilities.contains(wl_seat::Capability::Keyboard)
        {
            seat.get_keyboard(qh, ());
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for Dispatcher {
    fn event(
        dispatcher: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let core = &dispatcher.core;
        let mut shared = lock(&core.shared);
        match event {
            wl_keyboard::Event::Enter { surface, .. } => {
                if let Some(inhibitor) = shared.inhibitor.take() {
                    inhibitor.destroy();
                    shared.state = InhibitState::None;
                }
                shared.focused = Some(surface);
            }
            wl_keyboard::Event::Leave { .. } => shared.focused = None,
            _ => return,
        }
        core.reconcile(&mut shared);
    }
}

impl Dispatch<ZwpKeyboardShortcutsInhibitorV1, ()> for Dispatcher {
    fn event(
        dispatcher: &mut Self,
        inhibitor: &ZwpKeyboardShortcutsInhibitorV1,
        event: zwp_keyboard_shortcuts_inhibitor_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let mut shared = lock(&dispatcher.core.shared);
        if shared.inhibitor.as_ref() != Some(inhibitor) {
            return;
        }
        shared.state = match event {
            zwp_keyboard_shortcuts_inhibitor_v1::Event::Active => InhibitState::Active,
            zwp_keyboard_shortcuts_inhibitor_v1::Event::Inactive => InhibitState::Inactive,
            _ => shared.state,
        };
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Dispatcher {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpKeyboardShortcutsInhibitManagerV1, ()> for Dispatcher {
    fn event(
        _: &mut Self,
        _: &ZwpKeyboardShortcutsInhibitManagerV1,
        _: <ZwpKeyboardShortcutsInhibitManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl compass_platform::ShortcutInhibitor for ShortcutInhibit {
    fn set_wanted(&mut self, wanted: bool) {
        ShortcutInhibit::set_wanted(self, wanted);
    }

    fn dispatch_pending(&mut self) -> Result<(), String> {
        ShortcutInhibit::dispatch_pending(self).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inhibit_error_display() {
        assert!(
            InhibitError::Unsupported
                .to_string()
                .contains("doesn't support")
        );
        assert!(
            InhibitError::Connection("gone".into())
                .to_string()
                .contains("gone")
        );
    }

    #[test]
    fn nothing_is_inhibited_until_asked() {
        assert_eq!(InhibitState::default(), InhibitState::None);
    }
}
