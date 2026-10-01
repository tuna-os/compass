//! Session bus and XDG desktop portal probes.
//!
//! Part of [`super`]; see that module for the purity rule every function here
//! keeps to.

use compass_ipc::{DoctorCheck, DoctorStatus};

use super::check;
use crate::doctor::bus::BusProbe;
use crate::doctor::env::Env;

/// Bus name of the XDG desktop portal.
pub const PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Desktop";
/// Object path the portal exports its interfaces on.
pub const PORTAL_OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
/// Interface providing the only hotkey mechanism available on GNOME.
pub const GLOBAL_SHORTCUTS_INTERFACE: &str = "org.freedesktop.portal.GlobalShortcuts";

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
///
/// The portal is D-Bus-activatable, so an unowned name is not proof of absence.
/// When nobody owns it we read its `version` property, which starts it if it is
/// installed; only then is "absent" an honest answer.
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

    // Asking for a property starts a D-Bus-activatable portal. On a first
    // run that start can outlast the call, so the answer is "no such thing"
    // or an error although the portal is now coming up: it is asked once
    // more, after the name has had a moment to be taken.
    let mut answer = bus
        .property(
            PORTAL_BUS_NAME,
            PORTAL_OBJECT_PATH,
            "org.freedesktop.portal.OpenURI",
            "version",
        )
        .await;
    if !matches!(answer, Ok(Some(_))) {
        tokio::time::sleep(PORTAL_START_GRACE).await;
        if matches!(bus.name_has_owner(PORTAL_BUS_NAME).await, Ok(true)) {
            return check(
                NAME,
                DoctorStatus::Ok,
                format!("{PORTAL_BUS_NAME} was not running and started when asked"),
            );
        }
        answer = bus
            .property(
                PORTAL_BUS_NAME,
                PORTAL_OBJECT_PATH,
                "org.freedesktop.portal.OpenURI",
                "version",
            )
            .await;
    }
    match answer {
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
///
/// This one carries real weight. On GNOME it is the *only* hotkey mechanism
/// (PLAN §3.4); `xdg-desktop-portal-gnome` has shipped a backend since 48.rc,
/// while `xdg-desktop-portal-wlr` ships none at all — so a running portal
/// proves nothing and the interface has to be probed directly.
/// How long a portal that is being started on demand gets before it is asked
/// again.
const PORTAL_START_GRACE: std::time::Duration = std::time::Duration::from_millis(500);

/// Whether the launcher's hotkey can be bound, and how.
///
/// On Sway, Hyprland and niri the compositor's own configuration binds keys,
/// so a missing GlobalShortcuts portal is how those desktops work, not a
/// fault: the check passes and says which line to add. Elsewhere the portal
/// is the way, and its absence fails on GNOME and KDE, where it should be.
pub async fn global_shortcuts<B: BusProbe>(env: &Env, flatpak: bool, bus: &B) -> DoctorCheck {
    const NAME: &str = "portal.global-shortcuts";
    if let Some(compositor) = super::desktop::wlroots_compositor(env) {
        let command = compass_core::hotkey_guide::toggle_command(flatpak);
        return check(
            NAME,
            DoctorStatus::Ok,
            format!(
                "the launcher's key is bound in {}'s own configuration, not through a portal: \
                 add `{}` to {}, then reload it",
                compositor.name(),
                compositor.binding(&command),
                compositor.config_file()
            ),
        );
    }
    let expected = super::desktop::is_gnome(env) || super::desktop::is_kde(env);
    let missing = if expected {
        DoctorStatus::Fail
    } else {
        DoctorStatus::Warn
    };
    let command = compass_core::hotkey_guide::toggle_command(flatpak);
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
            missing,
            format!(
                "{GLOBAL_SHORTCUTS_INTERFACE} is not implemented by the running portal backend, \
                 so Compass cannot bind its hotkey. GNOME provides it from 48 onwards \
                 (xdg-desktop-portal-gnome) and KDE Plasma from 6 (xdg-desktop-portal-kde). \
                 Until then, bind a key to `{command}` in your desktop's keyboard settings"
            ),
        ),
        Err(err) => check(
            NAME,
            missing,
            format!(
                "could not query {GLOBAL_SHORTCUTS_INTERFACE}: {err}. Bind a key to `{command}` \
                 in your desktop's keyboard settings if the hotkey does not work"
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::bus::FakeBus;
    use crate::doctor::checks::tests::detail;

    // ----- dbus.session ---------------------------------------------------

    #[tokio::test]
    async fn session_bus_reachable_with_address_passes() {
        let env = Env::from_pairs([("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus")]);
        let c = session_bus(&env, &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("unix:path=/run/user/1000/bus"));
    }

    #[tokio::test]
    async fn session_bus_reachable_without_the_variable_warns() {
        let c = session_bus(&Env::empty(), &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("DBUS_SESSION_BUS_ADDRESS is unset"));
    }

    #[tokio::test]
    async fn session_bus_unreachable_with_an_address_fails() {
        let env = Env::from_pairs([("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nope")]);
        let c = session_bus(&env, &FakeBus::unreachable("No such file or directory")).await;
        assert_eq!(c.status, DoctorStatus::Fail);
        assert!(detail(&c).contains("unix:path=/nope"));
        assert!(detail(&c).contains("No such file or directory"));
    }

    #[tokio::test]
    async fn session_bus_absent_entirely_fails() {
        let c = session_bus(&Env::empty(), &FakeBus::unreachable("connection refused")).await;
        assert_eq!(c.status, DoctorStatus::Fail);
        assert!(detail(&c).contains("no session bus"));
    }

    // ----- portal.desktop -------------------------------------------------

    #[tokio::test]
    async fn portal_owned_passes() {
        let c = desktop_portal(&FakeBus::new().with_name(PORTAL_BUS_NAME)).await;
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("is owned"));
    }

    #[tokio::test]
    async fn portal_activatable_but_not_running_passes_and_says_so() {
        let bus = FakeBus::new().with_property(
            PORTAL_BUS_NAME,
            PORTAL_OBJECT_PATH,
            "org.freedesktop.portal.OpenURI",
            "version",
            "4",
        );
        let c = desktop_portal(&bus).await;
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("activated on demand"));
    }

    #[tokio::test]
    async fn portal_absent_fails_and_names_what_breaks() {
        let c = desktop_portal(&FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Fail);
        let d = detail(&c);
        assert!(d.contains("not installed or not running"));
        assert!(d.contains("global hotkey"));
    }

    #[tokio::test]
    async fn portal_unqueryable_fails_as_could_not_ask_not_as_absent() {
        let c = desktop_portal(&FakeBus::failing_queries("Connection reset by peer")).await;
        assert_eq!(c.status, DoctorStatus::Fail);
        assert!(detail(&c).contains("could not ask"));
        assert!(!detail(&c).contains("not installed"));
    }

    // ----- portal.global-shortcuts ----------------------------------------

    #[tokio::test]
    async fn global_shortcuts_present_passes_with_its_version() {
        let bus = FakeBus::new().with_property(
            PORTAL_BUS_NAME,
            PORTAL_OBJECT_PATH,
            GLOBAL_SHORTCUTS_INTERFACE,
            "version",
            "2",
        );
        let c = global_shortcuts(&gnome(), false, &bus).await;
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("v2"));
    }

    fn gnome() -> Env {
        Env::from_pairs([("XDG_CURRENT_DESKTOP", "GNOME")])
    }

    #[tokio::test]
    async fn global_shortcuts_absent_on_gnome_fails_and_says_what_to_do() {
        let bus = FakeBus::new().with_name(PORTAL_BUS_NAME);
        let c = global_shortcuts(&gnome(), false, &bus).await;
        assert_eq!(c.status, DoctorStatus::Fail);
        let d = detail(&c);
        assert!(d.contains("cannot bind its hotkey"));
        assert!(d.contains("`compass toggle`"));
    }

    #[tokio::test]
    async fn global_shortcuts_unqueryable_fails_without_claiming_absence() {
        let c = global_shortcuts(&gnome(), false, &FakeBus::failing_queries("Timeout")).await;
        assert_eq!(c.status, DoctorStatus::Fail);
        assert!(detail(&c).contains("could not query"));
        assert!(!detail(&c).contains("not implemented"));
    }

    #[tokio::test]
    async fn on_sway_the_compositor_binding_is_the_hotkey_and_passes() {
        let env = Env::from_pairs([
            ("SWAYSOCK", "/run/user/1000/sway-ipc.sock"),
            ("XDG_CURRENT_DESKTOP", "sway"),
        ]);
        let bus = FakeBus::new().with_name(PORTAL_BUS_NAME);
        let c = global_shortcuts(&env, false, &bus).await;
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(
            detail(&c)
                .contains("`bindsym $mod+space exec compass toggle` to ~/.config/sway/config"),
            "{}",
            detail(&c)
        );
        let c = global_shortcuts(&env, true, &FakeBus::unreachable("no bus")).await;
        assert_eq!(c.status, DoctorStatus::Ok, "no bus is no matter here");
        assert!(detail(&c).contains("exec flatpak run org.tunaos.compass toggle"));
    }
}
