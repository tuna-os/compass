//! What a wlroots compositor offers the launcher.
//!
//! Part of [`super`]; the compositor probe (a registry round trip, and a
//! request on the compositor's own socket) is made by the caller and arrives
//! as [`WaylandFindings`], so every outcome here is a value a test builds.

use compass_ipc::{DoctorCheck, DoctorStatus};
use compass_wayland::compositor::{Capabilities, Family};

use super::check;
use super::portal::{GLOBAL_SHORTCUTS_INTERFACE, PORTAL_BUS_NAME, PORTAL_OBJECT_PATH};
use crate::doctor::bus::BusProbe;
use crate::doctor::env::Env;

/// What probing the Wayland compositor found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaylandFindings {
    /// Which integration family it is.
    pub family: Family,
    /// The wlroots protocols it advertises (whatever the family).
    pub capabilities: Capabilities,
    /// The compositor's own IPC, when the environment names one: its name
    /// and whether it answered.
    pub compositor_ipc: Option<(String, bool)>,
}

/// `wlroots.capabilities`: each protocol the wlroots track uses, whether the
/// compositor advertises it, and what its absence costs — plus the portal's
/// GlobalShortcuts, the other way a hotkey can be bound, and the compositor's
/// own IPC (Hyprland, niri), which is where workspaces come from.
pub async fn wlroots<B: BusProbe>(
    env: &Env,
    wayland: Option<&WaylandFindings>,
    bus: &B,
) -> DoctorCheck {
    const NAME: &str = "wlroots.capabilities";

    let Some(wayland) = wayland else {
        return match env.get("WAYLAND_DISPLAY") {
            None => check(
                NAME,
                DoctorStatus::Ok,
                "no Wayland display, so there is no wlroots compositor to probe",
            ),
            Some(display) => check(
                NAME,
                DoctorStatus::Warn,
                format!(
                    "WAYLAND_DISPLAY={display} is set but the compositor could not be probed, \
                     so which protocols it offers is unknown"
                ),
            ),
        };
    };
    if wayland.family != Family::Wlroots {
        return check(
            NAME,
            DoctorStatus::Ok,
            format!(
                "not a wlroots compositor ({:?}), so the wlroots protocols are not used here",
                wayland.family
            ),
        );
    }

    let caps = wayland.capabilities;
    let portal_shortcuts = bus
        .property(
            PORTAL_BUS_NAME,
            PORTAL_OBJECT_PATH,
            GLOBAL_SHORTCUTS_INTERFACE,
            "version",
        )
        .await
        .ok()
        .flatten();
    let yes_no = |present: bool| if present { "yes" } else { "no" };
    let toplevel = if caps.toplevel_management {
        "zwlr_foreign_toplevel_manager_v1 (list, focus, close)"
    } else if caps.toplevel_list {
        "ext_foreign_toplevel_list_v1 only (list; no focus or close)"
    } else {
        "none"
    };
    let ipc = match &wayland.compositor_ipc {
        Some((name, true)) => format!("{name} (answering: workspaces, pids, geometry)"),
        Some((name, false)) => format!("{name} (named by the environment but not answering)"),
        None => "none (no Hyprland or niri socket; no workspaces)".to_owned(),
    };
    let mut detail = format!(
        "layer-shell: {}; foreign-toplevel: {toplevel}; data-control: {}; \
         hotkey protocol: {}; portal GlobalShortcuts: {}; compositor IPC: {ipc}",
        yes_no(caps.layer_shell),
        yes_no(caps.data_control),
        yes_no(caps.hotkey),
        portal_shortcuts
            .as_deref()
            .map_or_else(|| "no".to_owned(), |v| format!("v{v}")),
    );

    let mut missing = Vec::new();
    if !caps.layer_shell {
        missing.push("the launcher is an ordinary window, not an overlay");
    }
    if !caps.windows() {
        missing.push("window switching and WindowManagement have no windows");
    } else if !caps.toplevel_management {
        missing.push("windows can be listed but not focused or closed");
    }
    if !caps.data_control {
        missing.push("clipboard history and the selection need the launcher focused");
    }
    if !caps.hotkey && portal_shortcuts.is_none() {
        missing.push("no global hotkey: bind `vicinae toggle` in the compositor's config");
    }
    if matches!(&wayland.compositor_ipc, Some((_, false))) {
        missing.push("the compositor's socket does not answer, so there are no workspaces");
    }
    if missing.is_empty() {
        return check(NAME, DoctorStatus::Ok, detail);
    }
    detail.push_str(". Without the rest: ");
    detail.push_str(&missing.join("; "));
    check(NAME, DoctorStatus::Warn, detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::bus::FakeBus;
    use crate::doctor::checks::tests::detail;

    fn sway(capabilities: Capabilities) -> WaylandFindings {
        WaylandFindings {
            family: Family::Wlroots,
            capabilities,
            compositor_ipc: None,
        }
    }

    fn everything() -> Capabilities {
        Capabilities {
            layer_shell: true,
            toplevel_management: true,
            toplevel_list: true,
            data_control: true,
            hotkey: true,
        }
    }

    #[tokio::test]
    async fn a_full_wlroots_compositor_reports_every_protocol() {
        let mut found = sway(everything());
        found.compositor_ipc = Some(("Hyprland".into(), true));
        let c = wlroots(&Env::empty(), Some(&found), &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Ok, "{}", detail(&c));
        let d = detail(&c);
        for part in [
            "layer-shell: yes",
            "zwlr_foreign_toplevel_manager_v1",
            "data-control: yes",
            "hotkey protocol: yes",
            "portal GlobalShortcuts: no",
            "Hyprland (answering",
        ] {
            assert!(d.contains(part), "{part} missing from {d}");
        }
    }

    #[tokio::test]
    async fn no_hotkey_path_at_all_is_named_and_the_portal_counts_as_one() {
        let caps = Capabilities {
            hotkey: false,
            ..everything()
        };
        let c = wlroots(&Env::empty(), Some(&sway(caps)), &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("no global hotkey"), "{}", detail(&c));

        let portal = FakeBus::new().with_property(
            PORTAL_BUS_NAME,
            PORTAL_OBJECT_PATH,
            GLOBAL_SHORTCUTS_INTERFACE,
            "version",
            "1",
        );
        let c = wlroots(&Env::empty(), Some(&sway(caps)), &portal).await;
        assert_eq!(c.status, DoctorStatus::Ok, "{}", detail(&c));
        assert!(detail(&c).contains("portal GlobalShortcuts: v1"));
    }

    #[tokio::test]
    async fn list_only_toplevels_and_a_silent_compositor_socket_warn() {
        let caps = Capabilities {
            toplevel_management: false,
            ..everything()
        };
        let mut found = sway(caps);
        found.compositor_ipc = Some(("niri".into(), false));
        let c = wlroots(&Env::empty(), Some(&found), &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        let d = detail(&c);
        assert!(d.contains("ext_foreign_toplevel_list_v1 only"), "{d}");
        assert!(d.contains("not focused or closed"), "{d}");
        assert!(d.contains("does not answer"), "{d}");
    }

    #[tokio::test]
    async fn gnome_and_no_display_are_not_wlroots_problems() {
        let gnome = WaylandFindings {
            family: Family::Gnome,
            capabilities: Capabilities::default(),
            compositor_ipc: None,
        };
        let c = wlroots(&Env::empty(), Some(&gnome), &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Ok);
        let c = wlroots(&Env::empty(), None, &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Ok);
        let env = Env::from_pairs([("WAYLAND_DISPLAY", "wayland-1")]);
        let c = wlroots(&env, None, &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Warn);
    }
}
