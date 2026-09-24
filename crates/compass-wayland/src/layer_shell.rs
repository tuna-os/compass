//! Choosing the launcher's surface: `xdg_toplevel` or `wlr-layer-shell`.
//!
//! On GNOME (our first target) `wlr-layer-shell` is not implemented
//! (Mutter 51 `src/meson.build` has no `wlr-layer-shell`). The launcher
//! therefore runs as a plain `xdg_toplevel` centred window. On wlroots
//! (Hyprland/Sway/niri) the same launcher should run as a layer-surface
//! via `iced_layershell` to get exclusive focus and correct stacking.
//!
//! This module is the seam: it decides which surface to use based on the
//! compositor, falling back to `xdg_toplevel` when `wlr-layer-shell` is not
//! advertised **or the session is GNOME**. The surface itself is
//! `iced_layershell`, run by `compass_ui::run_resident_layer_shell`; the
//! binary asks [`select_surface`] which of the two entry points to call.

/// Which surface the launcher should use on this compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceKind {
    /// Plain `xdg_toplevel` — GNOME, and fallback for any compositor that
    /// does not advertise `zwlr_layer_shell_v1`.
    XdgToplevel,
    /// `zwlr_layer_shell_v1` — wlroots compositors where it is advertised.
    LayerShell,
}

/// Decide the surface from the compositor's advertised globals.
///
/// `advertised` is the list of global interface names the compositor sent
/// in its `registry` event. When `zwlr_layer_shell_v1` is present, a
/// wlroots compositor is assumed; otherwise the launcher falls back to
/// `xdg_toplevel`, which is what GNOME and every other compositor
/// implements. This is the correct fallback rather than a probe failure:
/// Mutter deliberately does not implement `wlr-layer-shell` (PLAN.md §3.1,
/// Mutter 51 `src/meson.build`).
#[must_use]
pub fn decide_surface(advertised: &[&str]) -> SurfaceKind {
    if advertised.contains(&"zwlr_layer_shell_v1") {
        SurfaceKind::LayerShell
    } else {
        SurfaceKind::XdgToplevel
    }
}

/// Whether `iced_layershell` should be used for this surface.
///
/// Feature-gated: `iced_layershell` is only compiled when the `layer-shell`
/// feature is enabled. Without the feature, every surface is `xdg_toplevel`
/// even when the compositor advertises the layer shell.
#[must_use]
pub fn should_use_layer_shell(surface: SurfaceKind, feature_enabled: bool) -> bool {
    matches!((surface, feature_enabled), (SurfaceKind::LayerShell, true))
}

/// Environment override: `VICINAE_LAYER_SHELL=0` keeps the `xdg_toplevel`
/// surface on a wlroots compositor (a layer shell that misbehaves with the
/// launcher, or a user who wants it tiled). Any other value, or unset, leaves
/// the decision to the compositor.
pub const OVERRIDE_ENV: &str = "VICINAE_LAYER_SHELL";

/// Decide the surface for a session: layer shell only on the wlroots family,
/// only where it is advertised, and only if the user has not turned it off.
///
/// GNOME is `xdg_toplevel` whatever it advertises — see
/// [`crate::compositor::family`].
#[must_use]
pub fn select_surface(
    session: &crate::compositor::Session,
    override_value: Option<&str>,
) -> SurfaceKind {
    if override_value.is_some_and(|value| matches!(value.trim(), "0" | "false" | "no" | "off")) {
        return SurfaceKind::XdgToplevel;
    }
    if session.wlroots_capabilities().layer_shell {
        SurfaceKind::LayerShell
    } else {
        SurfaceKind::XdgToplevel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_surface_keeps_gnome_on_xdg_toplevel_and_honours_the_override() {
        use crate::compositor::{Family, Globals, Session};
        let globals = Globals::from_pairs([("zwlr_layer_shell_v1", 4)]);
        let wlroots = Session {
            family: Family::Wlroots,
            globals: globals.clone(),
        };
        let gnome = Session {
            family: Family::Gnome,
            globals,
        };
        assert_eq!(select_surface(&wlroots, None), SurfaceKind::LayerShell);
        assert_eq!(select_surface(&wlroots, Some("1")), SurfaceKind::LayerShell);
        assert_eq!(
            select_surface(&wlroots, Some("0")),
            SurfaceKind::XdgToplevel
        );
        assert_eq!(select_surface(&gnome, None), SurfaceKind::XdgToplevel);
    }

    #[test]
    fn gnome_without_layer_shell_uses_xdg_toplevel() {
        let advertised = ["wl_compositor", "xdg_wm_base", "wl_seat"];
        assert_eq!(decide_surface(&advertised), SurfaceKind::XdgToplevel);
    }

    #[test]
    fn wlroots_with_layer_shell_uses_layer_shell() {
        let advertised = [
            "wl_compositor",
            "xdg_wm_base",
            "zwlr_layer_shell_v1",
            "wl_seat",
        ];
        assert_eq!(decide_surface(&advertised), SurfaceKind::LayerShell);
    }

    #[test]
    fn layer_shell_without_feature_falls_back_to_xdg() {
        assert!(!should_use_layer_shell(SurfaceKind::LayerShell, false));
        assert!(should_use_layer_shell(SurfaceKind::LayerShell, true));
        assert!(!should_use_layer_shell(SurfaceKind::XdgToplevel, true));
    }
}
