//! What the compositor advertises, and which integration family that makes it.
//!
//! Every wlroots-track feature is chosen from the global registry rather than
//! from a compositor name: Sway, Hyprland, niri, labwc and river all differ in
//! which protocols they carry, and a name-based switch would be wrong for the
//! next one. The one exception is GNOME, which is decided by
//! `$XDG_CURRENT_DESKTOP` **first**, so nothing a future Mutter starts to
//! advertise can move a GNOME session off the path it is tested on
//! (`PLAN.md` §3).

use std::collections::BTreeMap;

use wayland_client::Connection;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry;

/// `zwlr_layer_shell_v1`.
pub const LAYER_SHELL: &str = "zwlr_layer_shell_v1";
/// `zwlr_foreign_toplevel_manager_v1`: list, focus state, activate, close.
pub const WLR_FOREIGN_TOPLEVEL: &str = "zwlr_foreign_toplevel_manager_v1";
/// `ext_foreign_toplevel_list_v1`: list only.
pub const EXT_FOREIGN_TOPLEVEL_LIST: &str = "ext_foreign_toplevel_list_v1";
/// `ext_data_control_manager_v1`, the standardised data-control.
pub const EXT_DATA_CONTROL: &str = "ext_data_control_manager_v1";
/// `zwlr_data_control_manager_v1`, its wlroots predecessor.
pub const WLR_DATA_CONTROL: &str = "zwlr_data_control_manager_v1";
/// `xx_hotkey_manager_v1`, experimental global hotkeys.
pub const XX_HOTKEY: &str = "xx_hotkey_manager_v1";
/// `zwp_virtual_keyboard_manager_v1`: synthetic key events, for pasting.
pub const VIRTUAL_KEYBOARD: &str = "zwp_virtual_keyboard_manager_v1";
/// `zwp_keyboard_shortcuts_inhibit_manager_v1`: a focused surface takes the
/// compositor's own shortcuts.
pub const SHORTCUTS_INHIBIT: &str = "zwp_keyboard_shortcuts_inhibit_manager_v1";

/// `vicinae_hotkey_manager_v1`, Vicinae's own global hotkeys.
pub const VICINAE_HOTKEY: &str = "vicinae_hotkey_manager_v1";

/// The globals a compositor advertised: interface name to highest version.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Globals {
    interfaces: BTreeMap<String, u32>,
}

impl Globals {
    /// Globals from `(interface, version)` pairs, for tests and for callers
    /// that already hold a registry.
    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, u32)>) -> Self {
        let mut interfaces = BTreeMap::new();
        for (name, version) in pairs {
            let entry = interfaces.entry(name.to_owned()).or_insert(version);
            *entry = (*entry).max(version);
        }
        Self { interfaces }
    }

    /// Whether `interface` is advertised at all.
    #[must_use]
    pub fn has(&self, interface: &str) -> bool {
        self.interfaces.contains_key(interface)
    }

    /// The highest advertised version of `interface`.
    #[must_use]
    pub fn version(&self, interface: &str) -> Option<u32> {
        self.interfaces.get(interface).copied()
    }

    /// Every advertised interface name.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.interfaces.keys().map(String::as_str)
    }
}

/// Why the registry could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    /// No `WAYLAND_DISPLAY`, or nothing listening on it.
    #[error("no Wayland compositor to connect to: {0}")]
    Connect(#[from] wayland_client::ConnectError),
    /// Connected, but the registry round trip failed.
    #[error("the compositor's global registry could not be read: {0}")]
    Registry(String),
}

/// Reads the global registry of the compositor in `WAYLAND_DISPLAY`.
///
/// One connection, one round trip, then dropped: this is a question asked at
/// startup, and the long-lived clients open their own connections.
///
/// # Errors
///
/// [`ProbeError`] when there is no compositor or it will not answer.
pub fn probe() -> Result<Globals, ProbeError> {
    let connection = Connection::connect_to_env()?;
    probe_connection(&connection)
}

/// Reads the global registry over an existing connection.
///
/// # Errors
///
/// [`ProbeError::Registry`] when the round trip fails.
pub fn probe_connection(connection: &Connection) -> Result<Globals, ProbeError> {
    let (globals, _queue) = registry_queue_init::<Registry>(connection)
        .map_err(|err| ProbeError::Registry(err.to_string()))?;
    let list = globals.contents().clone_list();
    Ok(Globals::from_pairs(
        list.iter()
            .map(|global| (global.interface.as_str(), global.version)),
    ))
}

struct Registry;

impl wayland_client::Dispatch<wl_registry::WlRegistry, GlobalListContents> for Registry {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &wayland_client::QueueHandle<Self>,
    ) {
    }
}

/// Which integration family a session belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// GNOME: `xdg_toplevel`, the portal hotkey, the Shell extension.
    Gnome,
    /// A wlroots-protocol compositor: Sway, Hyprland, niri, labwc, river.
    ///
    /// Decided by `zwlr_layer_shell_v1`, which every one of them carries and
    /// Mutter and KWin's GNOME-like paths do not need.
    Wlroots,
    /// Anything else, KDE included for now: the GNOME path's protocol-free
    /// behaviour, with no Shell extension to find.
    Other,
}

/// Whether `$XDG_CURRENT_DESKTOP` (a `:`-separated list) names GNOME.
#[must_use]
pub fn desktop_is_gnome(current_desktop: Option<&str>) -> bool {
    current_desktop.is_some_and(|desktops| {
        desktops
            .split(':')
            .any(|desktop| desktop.eq_ignore_ascii_case("gnome"))
    })
}

/// Decide the family from the desktop name and the advertised globals.
///
/// GNOME is decided by name and wins outright: `PLAN.md` §3 pins GNOME
/// behaviour, and it must not change because a Mutter release grew a
/// protocol. Everything else is decided by what is advertised.
#[must_use]
pub fn family(current_desktop: Option<&str>, globals: &Globals) -> Family {
    if desktop_is_gnome(current_desktop) {
        return Family::Gnome;
    }
    if globals.has(LAYER_SHELL) {
        return Family::Wlroots;
    }
    Family::Other
}

/// `$XDG_CURRENT_DESKTOP`, falling back to `$XDG_SESSION_DESKTOP`.
///
/// Inside the Flatpak the first can be unset (#97), and the session name is
/// then the best remaining evidence.
#[must_use]
pub fn current_desktop() -> Option<String> {
    ["XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
}

/// What the wlroots track can use on this compositor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// The launcher can be a layer surface.
    pub layer_shell: bool,
    /// Windows can be listed, focused and closed.
    pub toplevel_management: bool,
    /// Windows can be listed but not acted on.
    pub toplevel_list: bool,
    /// The selection can be watched and set without focus.
    pub data_control: bool,
    /// A global hotkey can be bound over Wayland.
    pub hotkey: bool,
    /// Key events can be synthesised, so a paste can be pressed.
    pub virtual_keyboard: bool,
    /// The launcher can take the compositor's shortcuts while it records one.
    pub shortcuts_inhibit: bool,
}

impl Capabilities {
    /// What `globals` makes available.
    #[must_use]
    pub fn of(globals: &Globals) -> Self {
        Self {
            layer_shell: globals.has(LAYER_SHELL),
            toplevel_management: globals.has(WLR_FOREIGN_TOPLEVEL),
            toplevel_list: globals.has(EXT_FOREIGN_TOPLEVEL_LIST),
            data_control: globals.has(EXT_DATA_CONTROL) || globals.has(WLR_DATA_CONTROL),
            hotkey: globals.has(XX_HOTKEY) || globals.has(VICINAE_HOTKEY),
            virtual_keyboard: globals.has(VIRTUAL_KEYBOARD),
            shortcuts_inhibit: globals.has(SHORTCUTS_INHIBIT),
        }
    }

    /// Whether windows can be listed at all.
    #[must_use]
    pub const fn windows(&self) -> bool {
        self.toplevel_management || self.toplevel_list
    }
}

/// One answer to "what is this session": its family and what it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// The integration family.
    pub family: Family,
    /// The globals the compositor advertised.
    pub globals: Globals,
}

impl Session {
    /// Probes the compositor in `WAYLAND_DISPLAY` and reads the desktop name.
    ///
    /// # Errors
    ///
    /// [`ProbeError`] when there is no compositor to ask.
    pub fn detect() -> Result<Self, ProbeError> {
        let globals = probe()?;
        let family = family(current_desktop().as_deref(), &globals);
        Ok(Self { family, globals })
    }

    /// What the wlroots track can use. Empty on GNOME whatever is advertised,
    /// so no GNOME code path can pick up a wlroots client by accident.
    #[must_use]
    pub fn wlroots_capabilities(&self) -> Capabilities {
        match self.family {
            Family::Wlroots => Capabilities::of(&self.globals),
            Family::Gnome | Family::Other => Capabilities::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sway_1_9() -> Globals {
        // What headless Sway 1.9 advertised in the harness, trimmed.
        Globals::from_pairs([
            ("wl_compositor", 6),
            ("wl_seat", 9),
            ("xdg_wm_base", 5),
            (LAYER_SHELL, 4),
            (WLR_FOREIGN_TOPLEVEL, 3),
            (WLR_DATA_CONTROL, 2),
            (VIRTUAL_KEYBOARD, 1),
            (SHORTCUTS_INHIBIT, 1),
        ])
    }

    fn mutter_51() -> Globals {
        Globals::from_pairs([
            ("wl_compositor", 6),
            ("wl_seat", 9),
            ("xdg_wm_base", 6),
            ("xdg_activation_v1", 1),
        ])
    }

    #[test]
    fn a_wlroots_compositor_is_recognised_by_its_layer_shell() {
        assert_eq!(family(Some("sway"), &sway_1_9()), Family::Wlroots);
        assert_eq!(family(None, &sway_1_9()), Family::Wlroots);
        assert_eq!(family(Some("Hyprland"), &sway_1_9()), Family::Wlroots);
    }

    #[test]
    fn gnome_is_decided_by_name_even_if_mutter_grows_a_layer_shell() {
        assert_eq!(family(Some("GNOME"), &mutter_51()), Family::Gnome);
        assert_eq!(family(Some("ubuntu:GNOME"), &mutter_51()), Family::Gnome);
        // The regression this guards: a future Mutter advertising the layer
        // shell must not move GNOME onto the wlroots path.
        assert_eq!(family(Some("GNOME"), &sway_1_9()), Family::Gnome);
    }

    #[test]
    fn neither_gnome_nor_wlroots_is_other() {
        assert_eq!(family(Some("KDE"), &mutter_51()), Family::Other);
        assert_eq!(family(None, &Globals::default()), Family::Other);
    }

    #[test]
    fn gnome_gets_no_wlroots_capabilities_whatever_is_advertised() {
        let session = Session {
            family: Family::Gnome,
            globals: sway_1_9(),
        };
        assert_eq!(session.wlroots_capabilities(), Capabilities::default());
    }

    #[test]
    fn capabilities_follow_the_registry() {
        let caps = Capabilities::of(&sway_1_9());
        assert!(caps.layer_shell && caps.toplevel_management && caps.data_control);
        assert!(!caps.toplevel_list && !caps.hotkey);
        assert!(caps.windows());
        assert!(caps.virtual_keyboard && caps.shortcuts_inhibit);

        let ext_only = Capabilities::of(&Globals::from_pairs([
            (EXT_FOREIGN_TOPLEVEL_LIST, 1),
            (EXT_DATA_CONTROL, 1),
        ]));
        assert!(ext_only.windows() && !ext_only.toplevel_management);
        assert!(ext_only.data_control);
        assert!(!ext_only.virtual_keyboard && !ext_only.shortcuts_inhibit);
    }

    #[test]
    fn the_highest_advertised_version_is_kept() {
        let globals = Globals::from_pairs([("wl_output", 2), ("wl_output", 4), ("wl_output", 3)]);
        assert_eq!(globals.version("wl_output"), Some(4));
    }
}
