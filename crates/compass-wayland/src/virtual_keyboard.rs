//! `zwp_virtual_keyboard_v1`: pressing the paste chord in whatever window
//! has focus.
//!
//! The C++ pastes on Linux by asking its input server to press Ctrl+V (Ctrl+
//! Shift+V in a terminal) on a uinput keyboard (`LinuxPasteService`). The
//! engine does that too when the helper runs; this is the path for when it
//! does not, on a compositor that lets a client type: a virtual keyboard on
//! the first seat, a keymap holding only the three keys the chord needs, and
//! the presses with the modifier state the protocol asks the client to send
//! itself. Nothing here needs a device node or a capability, and headless
//! Sway runs it.
//!
//! The keymap is the client's own (as `wtype`'s is), so the chord means the
//! same thing whatever layout the person types in; the focused window is
//! sent the small keymap with the first key and the seat's own again with the
//! next real key press.

use std::io::Write as _;
use std::os::fd::AsFd as _;
use std::time::Instant;

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_keyboard, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

/// `KEY_LEFTCTRL`, the evdev code the protocol's `key` takes.
pub const KEY_LEFTCTRL: u32 = 29;
/// `KEY_LEFTSHIFT`.
pub const KEY_LEFTSHIFT: u32 = 42;
/// `KEY_V`.
pub const KEY_V: u32 = 47;

/// The Shift bit of the modifier masks, as the `complete` compatibility
/// section numbers the real modifiers.
pub const SHIFT_MASK: u32 = 1;
/// The Control bit.
pub const CONTROL_MASK: u32 = 1 << 2;

/// The keymap the virtual keyboard carries: Control, Shift and V at their
/// evdev codes (plus the xkb offset of 8), and the standard types and
/// compatibility so the modifiers mean what they usually do.
pub const PASTE_KEYMAP: &str = r#"xkb_keymap {
  xkb_keycodes "compass" {
    minimum = 8;
    maximum = 255;
    <LCTL> = 37;
    <LFSH> = 50;
    <AB04> = 55;
  };
  xkb_types "compass" { include "complete" };
  xkb_compatibility "compass" { include "complete" };
  xkb_symbols "compass" {
    key <LCTL> { [ Control_L ] };
    key <LFSH> { [ Shift_L ] };
    key <AB04> { [ v, V ] };
    modifier_map Control { <LCTL> };
    modifier_map Shift { <LFSH> };
  };
};
"#;

/// One thing sent to the compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// `key`: an evdev code, pressed or released.
    Key {
        /// The evdev code.
        code: u32,
        /// Pressed (`true`) or released.
        pressed: bool,
    },
    /// `modifiers`: the depressed mask after the key before it.
    Modifiers(u32),
}

/// The paste chord as the protocol takes it: Ctrl+V, or Ctrl+Shift+V into a
/// terminal (`paste_mods` in the input server), each modifier followed by
/// the mask it leaves, the modifiers released after the V.
#[must_use]
pub fn paste_steps(terminal: bool) -> Vec<Step> {
    let mut held = vec![(KEY_LEFTCTRL, CONTROL_MASK)];
    if terminal {
        held.push((KEY_LEFTSHIFT, SHIFT_MASK));
    }
    let mut steps = Vec::with_capacity(4 * held.len() + 2);
    let mut mask = 0;
    for (code, bit) in &held {
        mask |= bit;
        steps.push(Step::Key {
            code: *code,
            pressed: true,
        });
        steps.push(Step::Modifiers(mask));
    }
    steps.push(Step::Key {
        code: KEY_V,
        pressed: true,
    });
    steps.push(Step::Key {
        code: KEY_V,
        pressed: false,
    });
    for (code, bit) in held.iter().rev() {
        mask &= !bit;
        steps.push(Step::Key {
            code: *code,
            pressed: false,
        });
        steps.push(Step::Modifiers(mask));
    }
    steps
}

/// Why no key could be pressed.
#[derive(Debug, thiserror::Error)]
pub enum VirtualKeyboardError {
    /// No compositor, or the connection failed.
    #[error("the compositor connection: {0}")]
    Connection(String),
    /// The compositor has no `zwp_virtual_keyboard_manager_v1`, or no seat.
    #[error("the compositor has no zwp_virtual_keyboard_manager_v1 or no seat")]
    Unsupported,
    /// The keymap file could not be written.
    #[error("the keymap file: {0}")]
    Keymap(#[from] std::io::Error),
}

struct State;

/// A virtual keyboard on the compositor's first seat.
pub struct VirtualKeyboard {
    queue: EventQueue<State>,
    keyboard: ZwpVirtualKeyboardV1,
    started: Instant,
}

impl VirtualKeyboard {
    /// Connects to the compositor in `WAYLAND_DISPLAY` and makes a keyboard.
    ///
    /// # Errors
    ///
    /// As [`Self::bind`], and [`VirtualKeyboardError::Connection`] when there
    /// is no compositor.
    pub fn connect() -> Result<Self, VirtualKeyboardError> {
        let connection = Connection::connect_to_env()
            .map_err(|err| VirtualKeyboardError::Connection(err.to_string()))?;
        Self::bind(&connection)
    }

    /// Makes a keyboard on `connection`'s first seat and gives it
    /// [`PASTE_KEYMAP`].
    ///
    /// # Errors
    ///
    /// [`VirtualKeyboardError::Unsupported`] without the manager or a seat,
    /// [`VirtualKeyboardError::Keymap`] when the keymap cannot be written,
    /// [`VirtualKeyboardError::Connection`] when the compositor hangs up.
    pub fn bind(connection: &Connection) -> Result<Self, VirtualKeyboardError> {
        let (globals, mut queue) = registry_queue_init::<State>(connection)
            .map_err(|err| VirtualKeyboardError::Connection(err.to_string()))?;
        let qh = queue.handle();
        let manager = globals
            .bind::<ZwpVirtualKeyboardManagerV1, _, _>(&qh, 1..=1, ())
            .map_err(|_| VirtualKeyboardError::Unsupported)?;
        let seat = globals
            .bind::<wl_seat::WlSeat, _, _>(&qh, 1..=1, ())
            .map_err(|_| VirtualKeyboardError::Unsupported)?;
        let keyboard = manager.create_virtual_keyboard(&seat, &qh, ());

        // The compositor maps `size` bytes and parses them as a C string, so
        // the terminator is part of the file.
        let mut file = tempfile::tempfile()?;
        file.write_all(PASTE_KEYMAP.as_bytes())?;
        file.write_all(&[0])?;
        file.flush()?;
        let size = u32::try_from(PASTE_KEYMAP.len() + 1).unwrap_or(u32::MAX);
        keyboard.keymap(wl_keyboard::KeymapFormat::XkbV1.into(), file.as_fd(), size);
        // A roundtrip, so a compositor that refuses the keyboard (an
        // unauthorised client) says so here rather than at the first key.
        queue
            .roundtrip(&mut State)
            .map_err(|err| VirtualKeyboardError::Connection(err.to_string()))?;
        Ok(Self {
            queue,
            keyboard,
            started: Instant::now(),
        })
    }

    /// Sends `steps` and waits until the compositor has them.
    ///
    /// # Errors
    ///
    /// [`VirtualKeyboardError::Connection`] when the compositor hangs up.
    pub fn send(&mut self, steps: &[Step]) -> Result<(), VirtualKeyboardError> {
        for step in steps {
            match *step {
                Step::Key { code, pressed } => {
                    let time =
                        u32::try_from(self.started.elapsed().as_millis()).unwrap_or(u32::MAX);
                    let state = if pressed {
                        wl_keyboard::KeyState::Pressed
                    } else {
                        wl_keyboard::KeyState::Released
                    };
                    self.keyboard.key(time, code, state.into());
                }
                Step::Modifiers(depressed) => self.keyboard.modifiers(depressed, 0, 0, 0),
            }
        }
        self.queue
            .roundtrip(&mut State)
            .map(drop)
            .map_err(|err| VirtualKeyboardError::Connection(err.to_string()))
    }

    /// Presses the paste chord: Ctrl+Shift+V into a terminal, Ctrl+V
    /// elsewhere.
    ///
    /// # Errors
    ///
    /// As [`Self::send`].
    pub fn paste(&mut self, terminal: bool) -> Result<(), VirtualKeyboardError> {
        self.send(&paste_steps(terminal))
    }
}

impl Drop for VirtualKeyboard {
    fn drop(&mut self) {
        self.keyboard.destroy();
        let _ = self.queue.flush();
    }
}

/// Presses the paste chord on a fresh keyboard in `WAYLAND_DISPLAY`'s
/// compositor. Blocking.
///
/// # Errors
///
/// As [`VirtualKeyboard::connect`] and [`VirtualKeyboard::paste`].
pub fn paste(terminal: bool) -> Result<(), VirtualKeyboardError> {
    VirtualKeyboard::connect()?.paste(terminal)
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

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpVirtualKeyboardManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpVirtualKeyboardManagerV1,
        _: <ZwpVirtualKeyboardManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpVirtualKeyboardV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpVirtualKeyboardV1,
        _: <ZwpVirtualKeyboardV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_v_holds_control_around_the_v() {
        assert_eq!(
            paste_steps(false),
            [
                Step::Key {
                    code: KEY_LEFTCTRL,
                    pressed: true
                },
                Step::Modifiers(CONTROL_MASK),
                Step::Key {
                    code: KEY_V,
                    pressed: true
                },
                Step::Key {
                    code: KEY_V,
                    pressed: false
                },
                Step::Key {
                    code: KEY_LEFTCTRL,
                    pressed: false
                },
                Step::Modifiers(0),
            ]
        );
    }

    #[test]
    fn a_terminal_gets_shift_as_well_released_in_reverse() {
        let steps = paste_steps(true);
        assert_eq!(steps[3], Step::Modifiers(CONTROL_MASK | SHIFT_MASK));
        assert_eq!(
            steps[6..],
            [
                Step::Key {
                    code: KEY_LEFTSHIFT,
                    pressed: false
                },
                Step::Modifiers(CONTROL_MASK),
                Step::Key {
                    code: KEY_LEFTCTRL,
                    pressed: false
                },
                Step::Modifiers(0),
            ]
        );
    }

    #[test]
    fn the_keymap_names_the_codes_the_steps_press() {
        for (name, code) in [
            ("LCTL", KEY_LEFTCTRL),
            ("LFSH", KEY_LEFTSHIFT),
            ("AB04", KEY_V),
        ] {
            let line = format!("<{name}> = {};", code + 8);
            assert!(PASTE_KEYMAP.contains(&line), "{line}");
        }
    }
}
