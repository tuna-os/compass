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

/// The KWin provider, once [`start_kwin`] has it running.
static KWIN: OnceLock<Provider> = OnceLock::new();

/// The compositor's own IPC (Hyprland's socket, niri's, KWin's scripting),
/// when there is one: the C++ window-manager providers, chosen as the C++
/// chooses them, before the toplevel protocols. Hyprland and niri are
/// decided from the environment alone, so they need no Wayland display and
/// no round trip; KWin is whatever [`start_kwin`] started.
pub fn compositor() -> Option<&'static Provider> {
    if let Some(kwin) = KWIN.get() {
        return Some(kwin);
    }
    COMPOSITOR
        .get_or_init(|| {
            let provider = Provider::detect();
            if let Some(provider) = &provider {
                tracing::info!(
                    compositor = provider.display_name(),
                    socket = ?provider.socket(),
                    "compositor IPC"
                );
            }
            provider
        })
        .as_ref()
}

/// How long starting the KWin provider may take: a KWin that never answers
/// `loadScript` must not leave a task waiting for ever.
const KWIN_START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// On a Plasma Wayland session (`Environment::isWaylandPlasmaDesktop`),
/// starts the KWin provider on the session bus and makes it [`compositor`];
/// elsewhere nothing. Once per process, from start-up.
pub async fn start_kwin() {
    use compass_platform_linux::compositor::kwin::{Kwin, is_plasma_wayland};
    if KWIN.get().is_some() || !is_plasma_wayland(|name| std::env::var_os(name)) {
        return;
    }
    let started = tokio::time::timeout(KWIN_START_TIMEOUT, async {
        let connection = zbus::Connection::session()
            .await
            .map_err(|err| err.to_string())?;
        Kwin::start(connection).await.map_err(|err| err.to_string())
    })
    .await
    .unwrap_or_else(|_| Err("KWin did not answer in time".to_owned()));
    match started {
        Ok(kwin) => {
            tracing::info!("compositor IPC: KWin scripting");
            let _ = KWIN.set(Provider::Kwin(kwin));
        }
        Err(err) => tracing::warn!(error = %err, "KDE window management unavailable"),
    }
}

/// Unloads the KWin tracker script, when [`start_kwin`] loaded one: the C++
/// provider's `aboutToQuit`.
pub async fn stop_kwin() {
    if let Some(Provider::Kwin(kwin)) = KWIN.get() {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), kwin.stop()).await;
    }
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
