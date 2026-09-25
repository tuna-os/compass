//! Which desktop this is, and whether our GNOME Shell extension is there.
//!
//! Part of [`super`]; see that module for the purity rule every function here
//! keeps to.

use compass_ipc::{DoctorCheck, DoctorStatus};

use super::check;
use crate::doctor::bus::BusProbe;
use crate::doctor::env::Env;

/// Bus name GNOME Shell (and therefore every Shell extension) is reached at.
pub const GNOME_SHELL_BUS_NAME: &str = "org.gnome.Shell";
/// Object path of GNOME Shell itself.
pub const GNOME_SHELL_OBJECT_PATH: &str = "/org/gnome/Shell";

/// Object path doctor probes for the versioned Compass Shell-extension
/// contract: the Windows object, whose `Version` every contract revision
/// carries. Taken from `compass-shell` so doctor cannot probe a path the
/// extension does not export.
pub const EXTENSION_OBJECT_PATH: &str = compass_shell::WINDOWS_PATH;
/// Interface name of that contract.
pub const EXTENSION_INTERFACE: &str = compass_shell::WINDOWS_INTERFACE;
/// Contract version this build speaks.
pub const EXTENSION_CONTRACT_VERSION: u32 = compass_shell::CONTRACT_VERSION;
/// The oldest contract version this build still uses, without what came
/// after it.
pub const EXTENSION_OLDEST_CONTRACT_VERSION: u32 = compass_shell::OLDEST_CONTRACT_VERSION;

/// Interface name of the pre-existing, unversioned window-management contract
/// that the C++ engine talks to (`src/server/.../gnome-window-manager.hpp`).
///
/// Recorded for reference only: it exposes no properties, so its presence
/// cannot be probed without an Introspect call, and this build requires the
/// versioned [`EXTENSION_INTERFACE`] regardless.
pub const LEGACY_WINDOWS_INTERFACE: &str = "org.gnome.Shell.Extensions.Windows";

/// Exactly what stops working without the Shell extension, per PLAN §3.5.1.
pub const EXTENSION_DEGRADATION: &str = "window switching, clipboard history and paste are unavailable; app search, launching, \
     calculator, emoji, snippets and extensions are unaffected";

/// Whether the environment claims a GNOME session, with the desktop name(s).
#[must_use]
pub fn is_gnome(env: &Env) -> bool {
    env.list("XDG_CURRENT_DESKTOP")
        .iter()
        .any(|name| name.eq_ignore_ascii_case("GNOME"))
}

/// KWin's bus name, which its scripting interface and virtual desktops are
/// reached at.
pub const KWIN_BUS_NAME: &str = compass_platform_linux::compositor::kwin::KWIN_SERVICE;

/// Whether the environment claims a KDE session (`Environment::isPlasmaDesktop`).
#[must_use]
pub fn is_kde(env: &Env) -> bool {
    env.list("XDG_CURRENT_DESKTOP")
        .iter()
        .any(|name| name.eq_ignore_ascii_case("KDE"))
}

/// Whether KWin is there for the KDE window-management provider: its name on
/// the bus, and its virtual desktops.
pub async fn kwin<B: BusProbe>(env: &Env, bus: &B) -> DoctorCheck {
    use compass_platform_linux::compositor::kwin::{DESKTOPS_INTERFACE, DESKTOPS_PATH};
    const NAME: &str = "kde.kwin";

    if !is_kde(env) {
        return check(
            NAME,
            DoctorStatus::Ok,
            "not a KDE session, so KWin scripting is not used here",
        );
    }
    if env.get("WAYLAND_DISPLAY").is_none() {
        return check(
            NAME,
            DoctorStatus::Warn,
            "KDE without a Wayland display: window switching on X11 is not ported, so windows              and workspaces are unavailable",
        );
    }
    match bus.name_has_owner(KWIN_BUS_NAME).await {
        Ok(true) => {}
        Ok(false) => {
            return check(
                NAME,
                DoctorStatus::Warn,
                format!(
                    "nobody owns {KWIN_BUS_NAME} on the session bus, so window switching,                      workspaces and the window toggles are unavailable until KWin is running"
                ),
            );
        }
        Err(err) => {
            return check(
                NAME,
                DoctorStatus::Warn,
                format!("could not ask whether KWin is on the session bus: {err}"),
            );
        }
    }
    match bus
        .property(KWIN_BUS_NAME, DESKTOPS_PATH, DESKTOPS_INTERFACE, "count")
        .await
    {
        Ok(Some(count)) => check(
            NAME,
            DoctorStatus::Ok,
            format!(
                "KWin is on the session bus: windows through its scripting interface,                  {count} virtual desktop(s) as workspaces"
            ),
        ),
        Ok(None) => check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "KWin is on the session bus but has no {DESKTOPS_INTERFACE}, so windows can be                  switched but there are no workspaces"
            ),
        ),
        Err(err) => check(
            NAME,
            DoctorStatus::Warn,
            format!("KWin is on the session bus but its desktops could not be read: {err}"),
        ),
    }
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
                "XDG_CURRENT_DESKTOP={current} — not a GNOME session. The Rust engine targets \
                 GNOME 50/51, the wlroots compositors (Sway, Hyprland, niri: see \
                 wlroots.capabilities for what this one offers) and, for windows and \
                 workspaces, KDE Plasma (see kde.kwin); the rest of Phase 5 is still to come, \
                 so elsewhere window switching, clipboard history and the global hotkey may all \
                 be unavailable"
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
///
/// **Provisional.** This probes the bus name and the versioned contract
/// directly, because `compass-shell` — which will own this knowledge — is being
/// written in parallel. The shape is the part that matters: three outcomes
/// (present / absent / version mismatch), each with the precise degradation
/// spelled out, so swapping the probe for `compass-shell`'s richer client is a
/// body change, not an interface change.
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
            Ok(found)
                if (EXTENSION_OLDEST_CONTRACT_VERSION..EXTENSION_CONTRACT_VERSION)
                    .contains(&found) =>
            {
                check(
                    NAME,
                    DoctorStatus::Warn,
                    format!(
                        "extension present, contract v{found}, a release behind this build's \
                     v{EXTENSION_CONTRACT_VERSION}: window switching, clipboard history and \
                     paste work, Switch Workspaces does not. Update \
                     compass@tuna-os.github.io: extensions/gnome-shell/README.md in the \
                     Compass repository says how"
                    ),
                )
            }
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

    // "Nothing answered" has two causes with different advice: the extension is
    // missing, or GNOME Shell itself is not on the bus (a GNOME-flavoured
    // session without Shell, or Shell still starting). Telling someone to
    // install an extension when Shell is not running is a wrong diagnosis.
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
             {EXTENSION_DEGRADATION}. Install compass@tuna-os.github.io: \
             extensions/gnome-shell/README.md in the Compass repository says how"
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::bus::FakeBus;
    use crate::doctor::checks::tests::detail;

    // ----- desktop.environment --------------------------------------------

    fn gnome_env() -> Env {
        Env::from_pairs([("XDG_CURRENT_DESKTOP", "GNOME")])
    }

    fn shell_bus(version: &str) -> FakeBus {
        FakeBus::new()
            .with_name(GNOME_SHELL_BUS_NAME)
            .with_property(
                GNOME_SHELL_BUS_NAME,
                GNOME_SHELL_OBJECT_PATH,
                GNOME_SHELL_BUS_NAME,
                "ShellVersion",
                version,
            )
    }

    #[tokio::test]
    async fn desktop_gnome_with_a_shell_version_passes() {
        let c = desktop_environment(&gnome_env(), &shell_bus("51.0")).await;
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("GNOME Shell 51.0"));
    }

    #[tokio::test]
    async fn desktop_recognises_gnome_inside_a_colon_list_case_insensitively() {
        let env = Env::from_pairs([("XDG_CURRENT_DESKTOP", "ubuntu:gnome")]);
        assert!(is_gnome(&env));
        let c = desktop_environment(&env, &shell_bus("50.1")).await;
        assert_eq!(c.status, DoctorStatus::Ok);
    }

    #[tokio::test]
    async fn desktop_gnome_without_a_reachable_shell_warns() {
        let c = desktop_environment(&gnome_env(), &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("did not answer ShellVersion"));
    }

    #[tokio::test]
    async fn desktop_non_gnome_warns_about_the_target() {
        let env = Env::from_pairs([("XDG_CURRENT_DESKTOP", "sway:wlroots")]);
        assert!(!is_gnome(&env));
        let c = desktop_environment(&env, &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("not a GNOME session"));
        assert!(detail(&c).contains("Phase 5"));
    }

    #[tokio::test]
    async fn desktop_unset_warns_and_mentions_the_session_variable() {
        let env = Env::from_pairs([("XDG_SESSION_DESKTOP", "gnome")]);
        let c = desktop_environment(&env, &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("XDG_CURRENT_DESKTOP is unset"));
        assert!(detail(&c).contains("gnome"));
    }

    #[tokio::test]
    async fn desktop_gnome_with_an_unqueryable_bus_warns() {
        let c = desktop_environment(&gnome_env(), &FakeBus::failing_queries("Timeout")).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("could not be read"));
    }

    // ----- kde.kwin -------------------------------------------------------

    fn plasma_env() -> Env {
        Env::from_pairs([
            ("XDG_CURRENT_DESKTOP", "KDE"),
            ("WAYLAND_DISPLAY", "wayland-0"),
        ])
    }

    #[tokio::test]
    async fn kwin_with_its_desktops_passes_and_counts_them() {
        use compass_platform_linux::compositor::kwin::{DESKTOPS_INTERFACE, DESKTOPS_PATH};
        let bus = FakeBus::new().with_name(KWIN_BUS_NAME).with_property(
            KWIN_BUS_NAME,
            DESKTOPS_PATH,
            DESKTOPS_INTERFACE,
            "count",
            "4",
        );
        let c = kwin(&plasma_env(), &bus).await;
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(
            detail(&c).contains("4 virtual desktop(s)"),
            "{}",
            detail(&c)
        );
    }

    #[tokio::test]
    async fn kwin_absent_on_plasma_warns_and_elsewhere_is_not_asked() {
        let c = kwin(&plasma_env(), &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("nobody owns org.kde.KWin"));

        let c = kwin(&gnome_env(), &FakeBus::failing_queries("Timeout")).await;
        assert_eq!(c.status, DoctorStatus::Ok, "{}", detail(&c));

        let x11 = Env::from_pairs([("XDG_CURRENT_DESKTOP", "KDE")]);
        let c = kwin(&x11, &FakeBus::new().with_name(KWIN_BUS_NAME)).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("X11 is not ported"));
    }

    #[tokio::test]
    async fn kwin_without_virtual_desktops_warns_that_there_are_no_workspaces() {
        let bus = FakeBus::new().with_name(KWIN_BUS_NAME);
        let c = kwin(&plasma_env(), &bus).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("no workspaces"));
    }

    // ----- gnome.shell-extension ------------------------------------------

    fn extension_bus(version: &str) -> FakeBus {
        FakeBus::new()
            .with_name(GNOME_SHELL_BUS_NAME)
            .with_property(
                GNOME_SHELL_BUS_NAME,
                EXTENSION_OBJECT_PATH,
                EXTENSION_INTERFACE,
                "Version",
                version,
            )
    }

    #[tokio::test]
    async fn extension_at_the_expected_version_passes() {
        let bus = extension_bus(&EXTENSION_CONTRACT_VERSION.to_string());
        let c = shell_extension(&gnome_env(), &bus).await;
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("extension present"));
    }

    #[tokio::test]
    async fn extension_at_contract_v4_passes_and_one_a_release_behind_warns_about_workspaces() {
        let c = shell_extension(&gnome_env(), &extension_bus("4")).await;
        assert_eq!(c.status, DoctorStatus::Ok);
        assert_eq!(detail(&c), "extension present, contract v4");

        let c = shell_extension(&gnome_env(), &extension_bus("3")).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        let d = detail(&c);
        assert!(d.starts_with("extension present, contract v3"), "{d}");
        assert!(d.contains("Switch Workspaces does not"), "{d}");
        assert!(!d.contains("version mismatch"), "{d}");
    }

    #[tokio::test]
    async fn extension_version_mismatch_warns_and_names_the_degradation() {
        let bus = extension_bus(&(EXTENSION_CONTRACT_VERSION + 7).to_string());
        let c = shell_extension(&gnome_env(), &bus).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        let d = detail(&c);
        assert!(d.contains("version mismatch"));
        assert!(d.contains("window switching"));
        assert!(d.contains("clipboard history"));
        assert!(d.contains("paste"));
    }

    #[tokio::test]
    async fn extension_with_a_malformed_version_is_treated_as_incompatible() {
        let bus = extension_bus("v2-beta");
        let c = shell_extension(&gnome_env(), &bus).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("not a contract number"));
        assert!(detail(&c).contains("window switching"));
    }

    #[tokio::test]
    async fn extension_absent_on_a_running_shell_names_exactly_what_breaks() {
        let bus = FakeBus::new().with_name(GNOME_SHELL_BUS_NAME);
        let c = shell_extension(&gnome_env(), &bus).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        let d = detail(&c);
        assert!(d.contains("is not installed"));
        assert!(d.contains("window switching, clipboard history and paste are unavailable"));
        // The §3.5.1 promise: everything else is explicitly said to still work.
        assert!(d.contains("app search, launching"));
        assert!(!d.contains("some features"));
    }

    #[tokio::test]
    async fn extension_check_does_not_blame_the_extension_when_shell_is_absent() {
        let c = shell_extension(&gnome_env(), &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("nobody owns org.gnome.Shell"));
        assert!(!detail(&c).contains("is not installed"));
    }

    #[tokio::test]
    async fn extension_check_is_not_applicable_off_gnome() {
        let env = Env::from_pairs([("XDG_CURRENT_DESKTOP", "KDE")]);
        let c = shell_extension(&env, &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("not a GNOME session"));
    }

    #[tokio::test]
    async fn extension_check_reports_unknown_when_the_bus_cannot_be_queried() {
        let c = shell_extension(&gnome_env(), &FakeBus::failing_queries("Timeout")).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("status is unknown"));
    }
}
