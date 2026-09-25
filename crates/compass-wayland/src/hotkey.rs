//! Global hotkeys over `xx-hotkey-v1` or `vicinae-hotkey-v1`, where the
//! compositor offers one.
//!
//! Ports the C++ `XxHotkeyGlobalShortcutBackend` and
//! `VicinaeHotkeyGlobalShortcutBackend`, in the C++ factory's order: the
//! `xx` protocol when the compositor advertises it, the `vicinae` one when it
//! does not. `xdg-desktop-portal-wlr` ships no GlobalShortcuts backend, so on
//! a wlroots compositor the portal the GNOME path uses does not exist; these
//! experimental protocols are the only way for an unprivileged client to
//! *ask* for a hotkey. Where the compositor carries neither — every released
//! Sway, Hyprland and niri as of this writing — the user binds
//! `vicinae toggle` in the compositor's own configuration instead, and
//! [`manual_binding_hint`] says how.
//!
//! One [`HotkeyClient`] holds one connection for any number of hotkeys, as
//! the C++ backend holds one manager: each [`HotkeyClient::bind`] asks for one
//! combination and waits for the compositor's answer, and dropping the
//! [`Hotkey`] it returns releases the combination. Events are dispatched on a
//! thread of the client's own and arrive, tagged with the id the hotkey was
//! bound under, on the channel the client was built with.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use compass_wayland_protocols::vicinae_hotkey_v1::{
    vicinae_hotkey_manager_v1::{self, VicinaeHotkeyManagerV1},
    vicinae_hotkey_v1::{self, VicinaeHotkeyV1},
};
use compass_wayland_protocols::xx_hotkey_v1::{
    xx_hotkey_manager_v1::{self, XxHotkeyManagerV1},
    xx_hotkey_v1::{self, XxHotkeyV1},
};
use tokio::sync::{mpsc, oneshot};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, QueueHandle};

/// `XKB_KEY_space`.
pub const KEYSYM_SPACE: u32 = 0x0020;

/// How long a bind waits for the compositor's `bound` or `denied`.
pub const COMMIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Modifier bits. Both protocols define the same four with the same values.
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

/// Which protocol a [`HotkeyClient`] speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// `xx-hotkey-v1`.
    Xx,
    /// `vicinae-hotkey-v1`.
    Vicinae,
}

impl Protocol {
    /// The protocol's name, for logs and `doctor`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Xx => "xx-hotkey-v1",
            Self::Vicinae => "vicinae-hotkey-v1",
        }
    }
}

/// What happened to a bound hotkey.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// The trigger fired. `serial` counts as a user interaction for
    /// `xdg-activation-v1`.
    Triggered {
        /// The id the hotkey was bound under.
        id: String,
        /// Input serial.
        serial: u32,
    },
    /// The compositor withdrew the binding; no more events will come for it.
    Revoked {
        /// The id the hotkey was bound under.
        id: String,
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
    /// The compositor advertises neither hotkey manager.
    #[error("the compositor advertises neither xx_hotkey_manager_v1 nor vicinae_hotkey_manager_v1")]
    Unsupported,
    /// The compositor refused the trigger, with its message or ours.
    #[error("{0}")]
    Denied(String),
    /// The compositor did not answer the bind.
    #[error("the compositor did not answer the hotkey bind")]
    NoAnswer,
    /// The connection failed.
    #[error("Wayland connection: {0}")]
    Connection(String),
}

/// The C++'s sentence for a denial that came with no message.
pub const DENIED_WITHOUT_MESSAGE: &str = "Compositor denied the bind. Try another key combination.";

/// A hotkey to ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyRequest {
    /// The caller's id for it, echoed on its events.
    pub id: String,
    /// What the hotkey does, for the compositor's UI.
    pub description: String,
    /// The unshifted XKB keysym.
    pub keysym: u32,
    /// A [`modifiers`] mask.
    pub modifiers: u32,
}

type Commit = oneshot::Sender<Result<(), String>>;

/// What each hotkey object carries into the dispatch thread.
struct HotkeyData {
    id: String,
    commit: Mutex<Option<Commit>>,
    events: mpsc::UnboundedSender<HotkeyEvent>,
}

impl HotkeyData {
    fn answer(&self, result: Result<(), String>) {
        let pending = self
            .commit
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(pending) = pending {
            let _ = pending.send(result);
        }
    }

    fn triggered(&self, serial: u32) {
        let _ = self.events.send(HotkeyEvent::Triggered {
            id: self.id.clone(),
            serial,
        });
    }

    fn revoked(&self, message: String) {
        let _ = self.events.send(HotkeyEvent::Revoked {
            id: self.id.clone(),
            message,
        });
    }
}

fn denial(message: String) -> String {
    if message.is_empty() {
        DENIED_WITHOUT_MESSAGE.to_owned()
    } else {
        message
    }
}

enum Manager {
    Xx(XxHotkeyManagerV1),
    Vicinae(VicinaeHotkeyManagerV1),
}

enum Object {
    Xx(XxHotkeyV1),
    Vicinae(VicinaeHotkeyV1),
}

impl Object {
    fn destroy(&self) {
        match self {
            Self::Xx(hotkey) => hotkey.destroy(),
            Self::Vicinae(hotkey) => hotkey.destroy(),
        }
    }
}

/// The dispatch thread's state: everything lives in each object's data.
struct State;

/// A connection to the compositor's hotkey manager.
pub struct HotkeyClient {
    connection: Connection,
    qh: QueueHandle<State>,
    manager: Manager,
    app_id: String,
    events: mpsc::UnboundedSender<HotkeyEvent>,
}

impl std::fmt::Debug for HotkeyClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HotkeyClient")
            .field("protocol", &self.protocol())
            .finish_non_exhaustive()
    }
}

impl HotkeyClient {
    /// Connects to the compositor in `WAYLAND_DISPLAY`.
    ///
    /// # Errors
    ///
    /// [`HotkeyError::Unsupported`] without either protocol.
    pub fn connect(
        app_id: &str,
        events: mpsc::UnboundedSender<HotkeyEvent>,
    ) -> Result<Self, HotkeyError> {
        Self::connect_on(Connection::connect_to_env()?, app_id, events)
    }

    /// [`HotkeyClient::connect`], over an existing connection.
    ///
    /// # Errors
    ///
    /// As [`HotkeyClient::connect`].
    pub fn connect_on(
        connection: Connection,
        app_id: &str,
        events: mpsc::UnboundedSender<HotkeyEvent>,
    ) -> Result<Self, HotkeyError> {
        let (globals, mut queue) = registry_queue_init::<State>(&connection)
            .map_err(|err| HotkeyError::Connection(err.to_string()))?;
        let qh = queue.handle();
        let manager = match globals.bind::<XxHotkeyManagerV1, _, _>(&qh, 1..=1, ()) {
            Ok(manager) => {
                if !app_id.is_empty() {
                    manager.set_app_id(app_id.to_owned());
                }
                Manager::Xx(manager)
            }
            Err(_) => Manager::Vicinae(
                globals
                    .bind::<VicinaeHotkeyManagerV1, _, _>(&qh, 1..=1, ())
                    .map_err(|_| HotkeyError::Unsupported)?,
            ),
        };
        std::thread::Builder::new()
            .name("compass-hotkey".to_owned())
            .spawn(move || {
                let mut state = State;
                loop {
                    if let Err(err) = queue.blocking_dispatch(&mut state) {
                        tracing::warn!(error = %err, "lost the compositor's hotkey connection");
                        break;
                    }
                }
            })
            .map_err(|err| HotkeyError::Connection(err.to_string()))?;
        Ok(Self {
            connection,
            qh,
            manager,
            app_id: app_id.to_owned(),
            events,
        })
    }

    /// The protocol the compositor offered.
    #[must_use]
    pub fn protocol(&self) -> Protocol {
        match self.manager {
            Manager::Xx(_) => Protocol::Xx,
            Manager::Vicinae(_) => Protocol::Vicinae,
        }
    }

    /// Asks for `request` and waits for the compositor's answer.
    ///
    /// # Errors
    ///
    /// [`HotkeyError::Denied`] when the compositor refuses the trigger,
    /// [`HotkeyError::NoAnswer`] when it says nothing within
    /// [`COMMIT_TIMEOUT`].
    pub async fn bind(&self, request: &HotkeyRequest) -> Result<Hotkey, HotkeyError> {
        let (commit, answer) = oneshot::channel();
        let data = Arc::new(HotkeyData {
            id: request.id.clone(),
            commit: Mutex::new(Some(commit)),
            events: self.events.clone(),
        });
        let object = match &self.manager {
            Manager::Xx(manager) => {
                let hotkey = manager.create_hotkey(&self.qh, data);
                if !request.description.is_empty() {
                    hotkey.set_description(request.description.clone());
                }
                // Truncated, not retained: an unknown bit is a fatal protocol
                // error (`invalid_trigger`), which would take the connection
                // down with it.
                hotkey.set_key_trigger(
                    request.keysym,
                    xx_hotkey_manager_v1::Modifiers::from_bits_truncate(request.modifiers),
                );
                hotkey.commit();
                Object::Xx(hotkey)
            }
            Manager::Vicinae(manager) => Object::Vicinae(manager.bind(
                request.keysym,
                vicinae_hotkey_manager_v1::Modifiers::from_bits_truncate(request.modifiers),
                None,
                self.app_id.clone(),
                request.description.clone(),
                &self.qh,
                data,
            )),
        };
        let hotkey = Hotkey {
            object,
            connection: self.connection.clone(),
        };
        self.connection
            .flush()
            .map_err(|err| HotkeyError::Connection(err.to_string()))?;
        match tokio::time::timeout(COMMIT_TIMEOUT, answer).await {
            Ok(Ok(Ok(()))) => Ok(hotkey),
            Ok(Ok(Err(message))) => Err(HotkeyError::Denied(message)),
            Ok(Err(_)) | Err(_) => Err(HotkeyError::NoAnswer),
        }
    }
}

/// A hotkey the compositor bound. Dropping it releases the combination.
pub struct Hotkey {
    object: Object,
    connection: Connection,
}

impl std::fmt::Debug for Hotkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Hotkey")
    }
}

impl Drop for Hotkey {
    fn drop(&mut self) {
        self.object.destroy();
        let _ = self.connection.flush();
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

impl Dispatch<VicinaeHotkeyManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &VicinaeHotkeyManagerV1,
        _: vicinae_hotkey_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<XxHotkeyV1, Arc<HotkeyData>> for State {
    fn event(
        _: &mut Self,
        _: &XxHotkeyV1,
        event: xx_hotkey_v1::Event,
        data: &Arc<HotkeyData>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xx_hotkey_v1::Event::Bound => data.answer(Ok(())),
            xx_hotkey_v1::Event::Denied { message, .. } => data.answer(Err(denial(message))),
            xx_hotkey_v1::Event::Revoked { message } => data.revoked(message),
            xx_hotkey_v1::Event::Triggered { serial, .. } => data.triggered(serial),
            _ => {}
        }
    }
}

impl Dispatch<VicinaeHotkeyV1, Arc<HotkeyData>> for State {
    fn event(
        _: &mut Self,
        _: &VicinaeHotkeyV1,
        event: vicinae_hotkey_v1::Event,
        data: &Arc<HotkeyData>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            vicinae_hotkey_v1::Event::Bound => data.answer(Ok(())),
            vicinae_hotkey_v1::Event::Denied { message, .. } => data.answer(Err(denial(message))),
            vicinae_hotkey_v1::Event::Revoked { message, .. } => data.revoked(message),
            vicinae_hotkey_v1::Event::Pressed { serial, .. } => data.triggered(serial),
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

    #[test]
    fn a_denial_without_a_message_gets_the_cpps_sentence() {
        assert_eq!(denial(String::new()), DENIED_WITHOUT_MESSAGE);
        assert_eq!(denial("taken".to_owned()), "taken");
    }

    #[test]
    fn both_protocols_define_the_same_modifier_bits() {
        use compass_wayland_protocols::vicinae_hotkey_v1::vicinae_hotkey_manager_v1::Modifiers as V;
        use compass_wayland_protocols::xx_hotkey_v1::xx_hotkey_manager_v1::Modifiers as X;
        for (bit, xx, vicinae) in [
            (modifiers::SHIFT, X::Shift.bits(), V::Shift.bits()),
            (modifiers::CTRL, X::Ctrl.bits(), V::Ctrl.bits()),
            (modifiers::ALT, X::Alt.bits(), V::Alt.bits()),
            (modifiers::SUPER, X::Super.bits(), V::Super.bits()),
        ] {
            assert_eq!((bit, bit), (xx, vicinae));
        }
    }
}
