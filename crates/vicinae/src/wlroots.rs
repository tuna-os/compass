//! The wlroots family (Sway, Hyprland, niri, labwc, river): which of its
//! protocols this session has, decided once per process.
//!
//! `PLAN.md` Phase 5 Track B. Everything here is chosen from the compositor's
//! global registry and **never on GNOME** — [`compass_wayland::compositor`]
//! decides GNOME by `$XDG_CURRENT_DESKTOP` before it looks at a single global,
//! so the GNOME path (Shell extension, GlobalShortcuts portal, `xdg_toplevel`)
//! cannot change because a Mutter release grew a protocol.
//!
//! Detection is lazy and memoised: the engine asks on the first window or
//! clipboard request and at startup, and a session with no Wayland display
//! (the engine's own tests, a TTY) is simply not wlroots.

use std::sync::{Arc, OnceLock};

use compass_platform_linux::compositor::Provider;
use compass_wayland::compositor::{Capabilities, Family, Session};
use compass_wayland::toplevel::Toplevels;

/// What this process found.
#[derive(Debug)]
pub struct Wlroots {
    /// The compositor's desktop name, for messages.
    pub desktop: Option<String>,
    /// What the wlroots track can use.
    pub capabilities: Capabilities,
    /// The window list, when the compositor carries a toplevel protocol and
    /// the connection came up.
    pub toplevels: Option<Arc<Toplevels>>,
}

static DETECTED: OnceLock<Option<Wlroots>> = OnceLock::new();

static COMPOSITOR: OnceLock<Option<Provider>> = OnceLock::new();

/// The compositor's own IPC (Hyprland's socket, niri's), when the
/// environment names one: the C++ window-manager providers, chosen as the
/// C++ chooses them, before the toplevel protocols. Decided from the
/// environment alone, so it needs no Wayland display and no round trip.
pub fn compositor() -> Option<&'static Provider> {
    COMPOSITOR
        .get_or_init(|| {
            let provider = Provider::detect();
            if let Some(provider) = &provider {
                tracing::info!(
                    compositor = provider.display_name(),
                    socket = %provider.socket().display(),
                    "compositor IPC"
                );
            }
            provider
        })
        .as_ref()
}

/// The wlroots session, or `None` on GNOME, on another compositor, or with
/// no Wayland display at all.
///
/// Blocking (one registry round trip, and a second connection for the window
/// list); call it from a blocking context or [`detect`].
pub fn session() -> Option<&'static Wlroots> {
    DETECTED.get_or_init(detect_now).as_ref()
}

/// [`session`], from async code.
pub async fn detect() -> Option<&'static Wlroots> {
    if let Some(done) = DETECTED.get() {
        return done.as_ref();
    }
    tokio::task::spawn_blocking(session)
        .await
        .unwrap_or_default()
}

fn detect_now() -> Option<Wlroots> {
    let session = match Session::detect() {
        Ok(session) => session,
        Err(err) => {
            tracing::debug!(error = %err, "no Wayland compositor to probe");
            return None;
        }
    };
    if session.family != Family::Wlroots {
        tracing::debug!(family = ?session.family, "not a wlroots compositor");
        return None;
    }
    let capabilities = session.wlroots_capabilities();
    let toplevels = if capabilities.windows() {
        match Toplevels::connect() {
            Ok(toplevels) => Some(Arc::new(toplevels)),
            Err(err) => {
                tracing::warn!(error = %err, "window switching unavailable on this compositor");
                None
            }
        }
    } else {
        None
    };
    let desktop = compass_wayland::compositor::current_desktop();
    tracing::info!(
        desktop = desktop.as_deref().unwrap_or("unknown"),
        ?capabilities,
        "wlroots compositor"
    );
    Some(Wlroots {
        desktop,
        capabilities,
        toplevels,
    })
}
