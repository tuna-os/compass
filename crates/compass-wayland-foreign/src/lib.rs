//! A toolkit window's raw Wayland handles, as `wayland-client` proxies.
//!
//! The launcher's `wl_surface` is made by the toolkit (winit, under the
//! `xdg_toplevel` presentation) on the toolkit's own connection, and the
//! toolkit hands it out only as raw pointers, through `raw-window-handle`:
//! a `wl_display*` and a `wl_surface*`. Setting protocol state on that surface
//! from our side — background blur, `ext-background-effect-v1` — needs a
//! [`Connection`] over the same display and a [`WlSurface`] proxy for the
//! surface. [`bridge`] makes both, safely from the caller's side.
//!
//! # The exception
//!
//! This is the one crate in the workspace besides the SQLCipher and
//! generated-protocol ones that may write `unsafe` (ADR-0019), and it does so
//! in one function, `adopt`, with two `unsafe` blocks:
//!
//! 1. `Backend::from_foreign_display(display)`. The display must stay live
//!    for as long as the `Backend` or any clone of it exists.
//! 2. `ObjectId::from_ptr(wl_surface interface, surface)`. The pointer must be
//!    a live `wl_proxy` while the call runs, and for as long as the id is used.
//!
//! # The invariants, and what holds each one
//!
//! - **The pointers are live when they are read.** They come from a borrowed
//!   [`WindowHandle`] and [`DisplayHandle`] of one window object, whose
//!   lifetimes are the toolkit's promise that the handles are valid while
//!   borrowed; [`bridge`] reads both inside that borrow and takes them from
//!   the *same* object, so the surface is a surface of that display.
//! - **They are libwayland objects.** Only the `Wayland` variants of the raw
//!   handles are accepted; anything else is [`BridgeError::NotWayland`]. The
//!   surface pointer's interface is checked by name against `wl_surface`
//!   (`from_ptr` does, from libwayland's own record of the proxy), so a
//!   pointer to anything else is [`BridgeError::NotASurface`].
//! - **The surface is followed after the borrow ends.** A proxy made by
//!   `wayland-rs` carries a shared liveness flag that its owner clears when it
//!   destroys the proxy, and every request through the id checks it first. A
//!   surface without one (made by C code, or by a second copy of
//!   `wayland-backend`) could be freed under us unnoticed, so it is refused
//!   ([`BridgeError::Untracked`]) before the id leaves `adopt`. winit's and
//!   `iced_layershell`'s surfaces are `wayland-rs` proxies.
//! - **The display outlives the connection.** This one the types cannot say:
//!   the display belongs to the toolkit's event loop, which the launcher runs
//!   on the main thread for the life of the process (ADR-0015, the window is
//!   resident). Two things are done so that nothing here depends on it more
//!   than it must: one `Connection` is made per display and kept in a static
//!   for the rest of the process, so the backend's `Drop` (which destroys its
//!   event queue and proxies through the display) never runs; and the
//!   connection is only for use while the window it came from is open, which
//!   is how the launcher uses it (inside `iced::window::run`).
//! - **The backend is libwayland's.** `from_foreign_display` and `from_ptr`
//!   exist only in `wayland-backend`'s `client_system` backend, which this
//!   crate names in its manifest and calls through `wayland_backend::sys`,
//!   so a build that selected the pure-Rust backend fails to compile here
//!   rather than bridging into the wrong one. A process that cannot load
//!   libwayland at all gets [`BridgeError::NoLibrary`], not the backend's
//!   panic.
//!
//! What the bridged proxies may be used for is ordinary protocol traffic on a
//! queue of our own; the toolkit keeps reading the socket (see
//! `compass_wayland::material`).

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::{Mutex, PoisonError};

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WindowHandle,
};
use wayland_backend::sys::client::{Backend, ObjectId};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Proxy};

/// Why a window's handles could not be bridged.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    /// The toolkit would not give the handle out.
    #[error("the window's handle: {0}")]
    Handle(#[from] HandleError),
    /// The window is not a Wayland window (X11, or another platform).
    #[error("the window is not a Wayland window")]
    NotWayland,
    /// libwayland-client cannot be loaded in this process.
    #[error("libwayland-client is not available")]
    NoLibrary,
    /// The window handle's pointer is not a `wl_surface`.
    #[error("the window handle is not a wl_surface")]
    NotASurface,
    /// The surface is not a `wayland-rs` proxy, so when it is destroyed
    /// could not be known.
    #[error("the surface is not a wayland-rs proxy, so its lifetime cannot be followed")]
    Untracked,
}

/// A window's surface, as a proxy on a connection over its display.
#[derive(Debug, Clone)]
pub struct Bridged {
    /// A connection over the window's display, with its own event queue.
    /// The same one (cloned) for every window of that display.
    pub connection: Connection,
    /// The window's `wl_surface`, usable in requests on `connection`.
    pub surface: WlSurface,
}

/// The one connection made per foreign display, kept for the process's life
/// so its `Drop` never runs against a display the toolkit has closed.
static CONNECTIONS: Mutex<Vec<(usize, Connection)>> = Mutex::new(Vec::new());

/// Bridges `window`'s Wayland handles into a [`Connection`] over its display
/// and a [`WlSurface`] proxy of its surface.
///
/// Both handles are taken from `window` itself, so the surface is one of the
/// display's. Use the result only while the window is open; the surface proxy
/// knows when the toolkit destroys the surface (requests on it then fail
/// rather than reach freed memory), and the connection lives for the process.
///
/// # Errors
///
/// [`BridgeError`]: a handle the toolkit withholds, a window that is not a
/// Wayland one, no libwayland, a handle that is not a `wl_surface`, or a
/// surface whose destruction could not be followed.
pub fn bridge<W>(window: &W) -> Result<Bridged, BridgeError>
where
    W: HasWindowHandle + HasDisplayHandle + ?Sized,
{
    let display = window.display_handle()?;
    let RawDisplayHandle::Wayland(_) = display.as_raw() else {
        return Err(BridgeError::NotWayland);
    };
    let surface = window.window_handle()?;
    let RawWindowHandle::Wayland(_) = surface.as_raw() else {
        return Err(BridgeError::NotWayland);
    };
    if !wayland_sys::client::is_lib_available() {
        return Err(BridgeError::NoLibrary);
    }
    let (connection, id) = adopt(&display, &surface)?;
    let surface = WlSurface::from_id(&connection, id).map_err(|_| BridgeError::NotASurface)?;
    Ok(Bridged {
        connection,
        surface,
    })
}

/// The two `unsafe` calls, and nothing else that needs them.
///
/// Takes the borrowed handles rather than their pointers so that both are
/// known live (the borrow) and Wayland (checked by [`bridge`], and again
/// here) for the whole of the function.
#[allow(unsafe_code)]
fn adopt(
    display: &DisplayHandle<'_>,
    surface: &WindowHandle<'_>,
) -> Result<(Connection, ObjectId), BridgeError> {
    let (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(surface)) =
        (display.as_raw(), surface.as_raw())
    else {
        return Err(BridgeError::NotWayland);
    };
    let display: NonNull<c_void> = display.display;
    let surface: NonNull<c_void> = surface.surface;

    let connection = {
        let mut known = CONNECTIONS.lock().unwrap_or_else(PoisonError::into_inner);
        let key = display.as_ptr() as usize;
        match known.iter().find(|(at, _)| *at == key) {
            Some((_, connection)) => connection.clone(),
            None => {
                // SAFETY: `display` is the `wl_display*` of a live window: it
                // was read from a `DisplayHandle` borrowed from that window
                // for this call, and the handle is the `Wayland` variant, so
                // it is libwayland's display. `from_foreign_display` needs it
                // to outlive the backend; the backend's last clone is the one
                // pushed into `CONNECTIONS` below and never removed, so the
                // backend is never dropped, and it is used only while the
                // toolkit's event loop — which owns the display for the life
                // of the launcher process — runs (crate docs, "the display
                // outlives the connection"). libwayland was checked loadable
                // by the caller.
                let backend = unsafe { Backend::from_foreign_display(display.as_ptr().cast()) };
                let connection = Connection::from_backend(backend);
                known.push((key, connection.clone()));
                connection
            }
        }
    };

    // SAFETY: `surface` is the `wl_surface*` of the same live window, read
    // from a `WindowHandle` borrowed for this call, so it is a live `wl_proxy`
    // while `from_ptr` reads its class, id, listener and user data.
    // `from_ptr` refuses a proxy of another interface. After the call the id
    // outlives the borrow, which `from_ptr` allows only while the pointer
    // stays valid: that is made true by refusing, below, any proxy that is not
    // `wayland-rs`-managed, since a managed one carries the liveness flag its
    // owner clears on destruction and every later use of the id checks.
    let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.as_ptr().cast()) }
        .map_err(|_| BridgeError::NotASurface)?;
    // `get_data` answers only for a live, `wayland-rs`-managed proxy.
    if connection.backend().get_data(id.clone()).is_err() {
        return Err(BridgeError::Untracked);
    }
    Ok((connection, id))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window whose handles are whatever the test says.
    struct Window {
        display: fn() -> Result<DisplayHandle<'static>, HandleError>,
        surface: fn() -> Result<WindowHandle<'static>, HandleError>,
    }

    impl HasDisplayHandle for Window {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            (self.display)()
        }
    }

    impl HasWindowHandle for Window {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            (self.surface)()
        }
    }

    fn never() -> Result<WindowHandle<'static>, HandleError> {
        panic!("the window handle is not to be asked for once the display is refused");
    }

    #[test]
    fn a_withheld_display_is_the_toolkits_error() {
        let window = Window {
            display: || Err(HandleError::Unavailable),
            surface: never,
        };
        assert!(matches!(
            bridge(&window),
            Err(BridgeError::Handle(HandleError::Unavailable))
        ));
    }

    #[test]
    fn a_display_of_another_platform_is_not_wayland() {
        let others: [fn() -> Result<DisplayHandle<'static>, HandleError>; 3] = [
            || Ok(DisplayHandle::windows()),
            || Ok(DisplayHandle::appkit()),
            || Ok(DisplayHandle::web()),
        ];
        for display in others {
            let window = Window {
                display,
                surface: never,
            };
            assert!(matches!(bridge(&window), Err(BridgeError::NotWayland)));
        }
    }

    #[test]
    fn the_errors_say_what_went_wrong() {
        assert!(
            BridgeError::NotWayland
                .to_string()
                .contains("not a Wayland")
        );
        assert!(BridgeError::NoLibrary.to_string().contains("libwayland"));
        assert!(BridgeError::NotASurface.to_string().contains("wl_surface"));
        assert!(BridgeError::Untracked.to_string().contains("lifetime"));
        assert!(
            BridgeError::Handle(HandleError::NotSupported)
                .to_string()
                .contains("handle")
        );
    }
}
