//! A global hotkey over `xx-hotkey-v1`, where the compositor offers it.
//!
//! Ports the C++ `XxHotkeyGlobalShortcutBackend`. `xdg-desktop-portal-wlr`
//! ships no GlobalShortcuts backend, so on a wlroots compositor the portal the
//! GNOME path uses does not exist; this experimental protocol (upstream
//! vicinae #1936) is the only way for an unprivileged client to *ask* for a
//! hotkey. Where the compositor does not carry it — every released Sway,
//! Hyprland and niri as of this writing — the user binds `vicinae toggle` in
//! the compositor's own configuration instead, and [`manual_binding_hint`]
//! says how.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use compass_wayland_protocols::xx_hotkey_v1::{
    xx_hotkey_manager_v1::{self, XxHotkeyManagerV1},
    xx_hotkey_v1::{self, XxHotkeyV1},
};
use tokio::sync::mpsc;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, QueueHandle};

/// `XKB_KEY_space`.
pub const KEYSYM_SPACE: u32 = 0x0020;

/// Modifier bits, as `xx_hotkey_manager_v1.modifiers` defines them.
pub mod modifiers {
    /// Shift.
    pub const SHIFT: u32 = 1;
    /// Control.
    pub const CTRL: u32 = 2;
    /// Alt.
    pub const ALT: u32 = 4;
    /// Super / Logo.
    pub const SUPER: u32 = 8;
}

/// What happened to a bound hotkey.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// The trigger fired. `serial` counts as a user interaction for
    /// `xdg-activation-v1`.
    Triggered {
        /// Input serial.
        serial: u32,
    },
    /// The compositor withdrew the binding; no more events will come.
    Revoked {
        /// Its advisory explanation, possibly empty.
        message: String,
    },
}

/// Why a hotkey could not be bound.
#[derive(Debug, thiserror::Error)]
pub enum HotkeyError {
    /// No compositor to connect to.
    #[error("no Wayland compositor: {0}")]
    Connect(#[from] wayland_client::ConnectError),
    /// The compositor does not advertise `xx_hotkey_manager_v1`.
    #[error("the compositor does not advertise xx_hotkey_manager_v1")]
    Unsupported,
    /// The compositor refused the trigger.
    #[error("the compositor refused the hotkey: {0}")]
    Denied(String),
    /// The compositor did not answer the commit.
    #[error("the compositor did not answer the hotkey commit")]
    NoAnswer,
    /// The connection failed.
    #[error("Wayland connection: {0}")]
    Connection(String),
}

/// A hotkey to ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyRequest {
    /// The desktop file ID, for the compositor's policy and audit UI.
    pub app_id: String,
    /// What the hotkey does, for the compositor's UI.
    pub description: String,
    /// The unshifted XKB keysym.
    pub keysym: u32,
    /// A [`modifiers`] mask.
    pub modifiers: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Commit {
    Pending,
    Bound,
    Denied,
}

struct State {
    commit: Commit,
    denial: String,
    events: mpsc::UnboundedSender<HotkeyEvent>,
    revoked: Arc<AtomicBool>,
}

/// A bound hotkey. Its events keep arriving for as long as the process
/// lives; the protocol ties the binding to the connection, which the event
/// thread owns.
#[derive(Debug)]
pub struct Hotkey {
    revoked: Arc<AtomicBool>,
}

impl Hotkey {
    /// Binds `request` on the compositor in `WAYLAND_DISPLAY`.
    ///
    /// Returns once the compositor has answered the commit; events arrive on
    /// `events` from then on.
    ///
    /// # Errors
    ///
    /// [`HotkeyError::Unsupported`] without the protocol,
    /// [`HotkeyError::Denied`] when the compositor refuses the trigger.
    pub fn bind(
        request: &HotkeyRequest,
        events: mpsc::UnboundedSender<HotkeyEvent>,
    ) -> Result<Self, HotkeyError> {
        Self::bind_on(Connection::connect_to_env()?, request, events)
    }

    /// [`Hotkey::bind`], over an existing connection.
    ///
    /// # Errors
    ///
    /// As [`Hotkey::bind`].
    pub fn bind_on(
        connection: Connection,
        request: &HotkeyRequest,
        events: mpsc::UnboundedSender<HotkeyEvent>,
    ) -> Result<Self, HotkeyError> {
        let (globals, mut queue) = registry_queue_init::<State>(&connection)
            .map_err(|err| HotkeyError::Connection(err.to_string()))?;
        let qh = queue.handle();
        let manager = globals
            .bind::<XxHotkeyManagerV1, _, _>(&qh, 1..=1, ())
            .map_err(|_| HotkeyError::Unsupported)?;

        let revoked = Arc::new(AtomicBool::new(false));
        let mut state = State {
            commit: Commit::Pending,
            denial: String::new(),
            events,
            revoked: Arc::clone(&revoked),
        };

        if !request.app_id.is_empty() {
            manager.set_app_id(request.app_id.clone());
        }
        let hotkey = manager.create_hotkey(&qh, ());
        if !request.description.is_empty() {
            hotkey.set_description(request.description.clone());
        }
        // Truncated, not retained: an unknown bit is a fatal protocol error
        // (`invalid_trigger`), which would take the connection down with it.
        hotkey.set_key_trigger(
            request.keysym,
            xx_hotkey_manager_v1::Modifiers::from_bits_truncate(request.modifiers),
        );
        hotkey.commit();
        queue
            .roundtrip(&mut state)
            .map_err(|err| HotkeyError::Connection(err.to_string()))?;

        match state.commit {
            Commit::Bound => {}
            Commit::Denied => {
                hotkey.destroy();
                return Err(HotkeyError::Denied(state.denial));
            }
            Commit::Pending => {
                hotkey.destroy();
                return Err(HotkeyError::NoAnswer);
            }
        }

        std::thread::Builder::new()
            .name("compass-hotkey".to_owned())
            .spawn(move || {
                // Held here so the binding lives exactly as long as this
                // thread's connection.
                let _manager = manager;
                let _hotkey = hotkey;
                while !state.revoked.load(Ordering::Acquire) {
                    if let Err(err) = queue.blocking_dispatch(&mut state) {
                        tracing::warn!(error = %err, "lost the compositor's hotkey connection");
                        break;
                    }
                }
            })
            .map_err(|err| HotkeyError::Connection(err.to_string()))?;

        Ok(Self { revoked })
    }

    /// Whether the compositor has withdrawn the binding.
    #[must_use]
    pub fn is_revoked(&self) -> bool {
        self.revoked.load(Ordering::Acquire)
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
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

impl Dispatch<XxHotkeyManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &XxHotkeyManagerV1,
        _: xx_hotkey_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<XxHotkeyV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &XxHotkeyV1,
        event: xx_hotkey_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xx_hotkey_v1::Event::Bound => state.commit = Commit::Bound,
            xx_hotkey_v1::Event::Denied { message, .. } => {
                state.commit = Commit::Denied;
                state.denial = message;
            }
            xx_hotkey_v1::Event::Revoked { message } => {
                state.revoked.store(true, Ordering::Release);
                let _ = state.events.send(HotkeyEvent::Revoked { message });
            }
            xx_hotkey_v1::Event::Triggered { serial, .. } => {
                let _ = state.events.send(HotkeyEvent::Triggered { serial });
            }
            _ => {}
        }
    }
}

/// How to bind the launcher by hand on a compositor with no hotkey protocol,
/// for the one compositor `$XDG_CURRENT_DESKTOP` names, or all of them.
#[must_use]
pub fn manual_binding_hint(current_desktop: Option<&str>) -> String {
    const SWAY: &str =
        "Sway / i3-style (~/.config/sway/config): bindsym $mod+space exec vicinae toggle";
    const HYPRLAND: &str =
        "Hyprland (~/.config/hypr/hyprland.conf): bind = SUPER, SPACE, exec, vicinae toggle";
    const NIRI: &str =
        "niri (~/.config/niri/config.kdl), in binds: Mod+Space { spawn \"vicinae\" \"toggle\"; }";
    const RIVER: &str = "river (init): riverctl map normal Super Space spawn 'vicinae toggle'";
    const LABWC: &str = "labwc (rc.xml): <keybind key=\"W-space\"><action name=\"Execute\" command=\"vicinae toggle\"/></keybind>";

    let desktop = current_desktop.unwrap_or_default().to_ascii_lowercase();
    let one = [
        ("sway", SWAY),
        ("hyprland", HYPRLAND),
        ("niri", NIRI),
        ("river", RIVER),
        ("labwc", LABWC),
    ]
    .into_iter()
    .find(|(name, _)| desktop.split(':').any(|d| d == *name))
    .map(|(_, hint)| hint);

    match one {
        Some(hint) => format!("bind the launcher in your compositor: {hint}"),
        None => format!(
            "bind `vicinae toggle` to a key in your compositor's configuration, e.g. {SWAY}; \
             {HYPRLAND}; {NIRI}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hint_names_the_running_compositor_when_it_can() {
        assert!(manual_binding_hint(Some("sway")).contains("bindsym $mod+space"));
        assert!(manual_binding_hint(Some("Hyprland")).contains("bind = SUPER, SPACE"));
        assert!(manual_binding_hint(Some("niri")).contains("Mod+Space"));
    }

    #[test]
    fn an_unknown_compositor_gets_every_example_and_the_command() {
        let hint = manual_binding_hint(None);
        assert!(hint.contains("vicinae toggle"));
        assert!(hint.contains("bindsym") && hint.contains("SUPER, SPACE"));
    }
}
