//! DBus session bus, portal, desktop shortcuts, and Shell extension diagnostic checks.

use compass_ipc::{DoctorCheck, DoctorStatus};
use super::check;
use super::bus::BusProbe;
use super::env::Env;

/// Bus name of the XDG desktop portal.
pub const PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Desktop";
/// Object path the portal exports its interfaces on.
pub const PORTAL_OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
/// Interface providing the only hotkey mechanism available on GNOME.
pub const GLOBAL_SHORTCUTS_INTERFACE: &str = "org.freedesktop.portal.GlobalShortcuts";

/// Bus name GNOME Shell (and therefore every Shell extension) is reached at.
pub const GNOME_SHELL_BUS_NAME: &str = "org.gnome.Shell";
/// Object path of GNOME Shell itself.
pub const GNOME_SHELL_OBJECT_PATH: &str = "/org/gnome/Shell";

/// Object path of the versioned Compass Shell-extension contract (PLAN §3.5.3).
pub const EXTENSION_OBJECT_PATH: &str = "/org/gnome/Shell/Extensions/Vicinae";
/// Interface name of that contract.
pub const EXTENSION_INTERFACE: &str = "org.gnome.Shell.Extensions.Vicinae";
/// Contract version this build speaks.
pub const EXTENSION_CONTRACT_VERSION: u32 = 1;

/// Interface name of the pre-existing, unversioned window-management contract
/// that the C++ engine talks to (`src/server/.../gnome-window-manager.hpp`).
pub const LEGACY_WINDOWS_INTERFACE: &str = "org.gnome.Shell.Extensions.Windows";

/// Exactly what stops working without the Shell extension, per PLAN §3.5.1.
pub const EXTENSION_DEGRADATION: &str = "window switching, clipboard history and paste are unavailable; app search, launching, \
     calculator, emoji, snippets and extensions are unaffected";

/// Whether a session bus is configured and reachable.
pub async fn session_bus<B: BusProbe>(env: &Env, bus: &B) -> DoctorCheck {
    const NAME: &str = "dbus.session";
    let address = env.get("DBUS_SESSION_BUS_ADDRESS");
    match (bus.connect().await, address) {
        (Ok(()), Some(address)) => check(
            NAME,
            DoctorStatus::Ok,
            format!("session bus reachable (DBUS_SESSION_BUS_ADDRESS={address})"),
        ),
        (Ok(()), None) => check(
            NAME,
            DoctorStatus::Warn,
            "DBUS_SESSION_BUS_ADDRESS is unset, but a session bus was still reachable via the \
             default $XDG_RUNTIME_DIR/bus path. Child processes that read the variable \
             directly — notably anything launched on the host from a sandbox — will not find it",
        ),
        (Err(err), Some(address)) => check(
            NAME,
            DoctorStatus::Fail,
            format!(
                "DBUS_SESSION_BUS_ADDRESS={address} but connecting failed: {err}. Every \
                 portal, the global hotkey and all GNOME Shell integration are unavailable"
            ),
        ),
        (Err(err), None) => check(
            NAME,
            DoctorStatus::Fail,
            format!(
                "no session bus: DBUS_SESSION_BUS_ADDRESS is unset and connecting to the \
                 default path failed ({err}). Every portal, the global hotkey and all GNOME \
                 Shell integration are unavailable"
            ),
        ),
    }
}

/// Whether `org.freedesktop.portal.Desktop` is there.
pub async fn desktop_portal<B: BusProbe>(bus: &B) -> DoctorCheck {
    const NAME: &str = "portal.desktop";

    match bus.name_has_owner(PORTAL_BUS_NAME).await {
        Ok(true) => {
            return check(
                NAME,
                DoctorStatus::Ok,
                format!("{PORTAL_BUS_NAME} is owned on the session bus"),
            );
        }
        Ok(false) => {}
        Err(err) => {
            return check(
                NAME,
                DoctorStatus::Fail,
                format!("could not ask the session bus who owns {PORTAL_BUS_NAME}: {err}"),
            );
        }
    }

    match bus
        .property(
            PORTAL_BUS_NAME,
            PORTAL_OBJECT_PATH,
            "org.freedesktop.portal.OpenURI",
            "version",
        )
        .await
    {
        Ok(Some(version)) => check(
            NAME,
            DoctorStatus::Ok,
            format!(
                "{PORTAL_BUS_NAME} was not running but activated on demand (OpenURI v{version})"
            ),
        ),
        Ok(None) => check(
            NAME,
            DoctorStatus::Fail,
            format!(
                "{PORTAL_BUS_NAME} is not owned and could not be activated: xdg-desktop-portal \
                 is not installed or not running. The global hotkey, file chooser and host \
                 URI opening are all unavailable"
            ),
        ),
        Err(err) => check(
            NAME,
            DoctorStatus::Fail,
            format!("could not query {PORTAL_BUS_NAME}: {err}"),
        ),
    }
}

/// Whether the GlobalShortcuts portal interface is actually implemented.
pub async fn global_shortcuts<B: BusProbe>(bus: &B) -> DoctorCheck {
    const NAME: &str = "portal.global-shortcuts";
    match bus
        .property(
            PORTAL_BUS_NAME,
            PORTAL_OBJECT_PATH,
            GLOBAL_SHORTCUTS_INTERFACE,
            "version",
        )
        .await
    {
        Ok(Some(version)) => check(
            NAME,
            DoctorStatus::Ok,
            format!("{GLOBAL_SHORTCUTS_INTERFACE} v{version} is available"),
        ),
        Ok(None) => check(
            NAME,
            DoctorStatus::Fail,
            format!(
                "{GLOBAL_SHORTCUTS_INTERFACE} is not implemented by the running portal backend, \
                 so the global hotkey cannot be bound. xdg-desktop-portal-gnome provides it from \
                 48.rc onwards; xdg-desktop-portal-wlr ships no GlobalShortcuts backend at all, \
                 so on wlroots compositors there is currently no hotkey path"
            ),
        ),
        Err(err) => check(
            NAME,
            DoctorStatus::Fail,
            format!("could not query {GLOBAL_SHORTCUTS_INTERFACE}: {err}"),
        ),
    }
}

/// Whether the environment claims a GNOME session, with the desktop name(s).
#[must_use]
pub fn is_gnome(env: &Env) -> bool {
    env.list("XDG_CURRENT_DESKTOP")
        .iter()
        .any(|name| name.eq_ignore_ascii_case("GNOME"))
}

/// Which desktop this is, and GNOME Shell's version when it will tell us.
pub async fn desktop_environment<B: BusProbe>(env: &Env, bus: &B) -> DoctorCheck {
    const NAME: &str = "desktop.environment";

    let Some(current) = env.get("XDG_CURRENT_DESKTOP") else {
        let session = env.get("XDG_SESSION_DESKTOP").unwrap_or("also unset");
        return check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "XDG_CURRENT_DESKTOP is unset (XDG_SESSION_DESKTOP: {session}), so the desktop \
                 cannot be identified and desktop-specific integration is skipped"
            ),
        );
    };

    if !is_gnome(env) {
        return check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "XDG_CURRENT_DESKTOP={current} — not a GNOME session. The Rust engine currently \
                 targets GNOME 50/51; wlroots (Hyprland/Sway/niri) and KDE support is Phase 5 \
                 work, so window switching, clipboard history and the global hotkey may all be \
                 unavailable here"
            ),
        );
    }

    match bus
        .property(
            GNOME_SHELL_BUS_NAME,
            GNOME_SHELL_OBJECT_PATH,
            GNOME_SHELL_BUS_NAME,
            "ShellVersion",
        )
        .await
    {
        Ok(Some(version)) => check(
            NAME,
            DoctorStatus::Ok,
            format!("GNOME (XDG_CURRENT_DESKTOP={current}), GNOME Shell {version}"),
        ),
        Ok(None) => check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "XDG_CURRENT_DESKTOP={current} claims GNOME, but {GNOME_SHELL_BUS_NAME} did not \
                 answer ShellVersion. Either GNOME Shell is not running or this is a \
                 GNOME-flavoured session without it; the Shell version gates extension \
                 compatibility, so it could not be assessed"
            ),
        ),
        Err(err) => check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "XDG_CURRENT_DESKTOP={current} claims GNOME, but the Shell version could not be \
                 read: {err}"
            ),
        ),
    }
}

/// Whether the Compass GNOME Shell extension is installed, and at what version.
pub async fn shell_extension<B: BusProbe>(env: &Env, bus: &B) -> DoctorCheck {
    const NAME: &str = "gnome.shell-extension";

    if !is_gnome(env) {
        return check(
            NAME,
            DoctorStatus::Ok,
            "not a GNOME session, so the GNOME Shell extension is not used here",
        );
    }

    let version = bus
        .property(
            GNOME_SHELL_BUS_NAME,
            EXTENSION_OBJECT_PATH,
            EXTENSION_INTERFACE,
            "Version",
        )
        .await;

    match version {
        Ok(Some(raw)) => match raw.parse::<u32>() {
            Ok(found) if found == EXTENSION_CONTRACT_VERSION => check(
                NAME,
                DoctorStatus::Ok,
                format!("extension present, contract v{found}"),
            ),
            Ok(found) => check(
                NAME,
                DoctorStatus::Warn,
                format!(
                    "version mismatch: the installed extension speaks contract v{found}, this \
                     build speaks v{EXTENSION_CONTRACT_VERSION}. Until they match, \
                     {EXTENSION_DEGRADATION}"
                ),
            ),
            Err(_) => check(
                NAME,
                DoctorStatus::Warn,
                format!(
                    "the extension reported a Version of {raw:?}, which is not a contract \
                     number this build can compare against v{EXTENSION_CONTRACT_VERSION}. \
                     Treating it as incompatible: {EXTENSION_DEGRADATION}"
                ),
            ),
        },
        Ok(None) => absent_extension(bus).await,
        Err(err) => check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "could not query {EXTENSION_INTERFACE} on the session bus: {err}. Extension \
                 status is unknown; if it is in fact missing then {EXTENSION_DEGRADATION}"
            ),
        ),
    }
}

async fn absent_extension<B: BusProbe>(bus: &B) -> DoctorCheck {
    const NAME: &str = "gnome.shell-extension";

    match bus.name_has_owner(GNOME_SHELL_BUS_NAME).await {
        Ok(false) => {
            return check(
                NAME,
                DoctorStatus::Warn,
                format!(
                    "nobody owns {GNOME_SHELL_BUS_NAME} on the session bus, so whether the \
                     extension is installed could not be determined. If GNOME Shell is not \
                     running, {EXTENSION_DEGRADATION}"
                ),
            );
        }
        Ok(true) => {}
        Err(err) => {
            return check(
                NAME,
                DoctorStatus::Warn,
                format!(
                    "could not ask the session bus about {GNOME_SHELL_BUS_NAME}: {err}. \
                     Extension status is unknown"
                ),
            );
        }
    }

    check(
        NAME,
        DoctorStatus::Warn,
        format!(
            "GNOME Shell is running but the Compass extension is not installed: nothing exports \
             {EXTENSION_INTERFACE} at {EXTENSION_OBJECT_PATH} on {GNOME_SHELL_BUS_NAME}. On \
             GNOME 50/51 the extension is the only mechanism available for these, so \
             {EXTENSION_DEGRADATION}"
        ),
    )
}
