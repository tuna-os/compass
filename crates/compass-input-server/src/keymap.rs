//! Keycodes to text: xkbcommon for the server, a table for tests.

use std::collections::BTreeSet;

use compass_core::input_server::wire::LayoutInfo;
use compass_platform_linux::keyboard::{CharMap, EVDEV_OFFSET, KEY_LEFTSHIFT};
use xkbcommon::xkb;

use crate::tracker::KeyState;

/// The real key state: an xkb keymap compiled from RMLVO names.
pub struct XkbKeys {
    context: xkb::Context,
    state: xkb::State,
}

impl std::fmt::Debug for XkbKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XkbKeys").finish_non_exhaustive()
    }
}

/// Compiles `layout`, or the default keymap (which honours the
/// `XKB_DEFAULT_*` variables) when there is none.
fn compile(context: &xkb::Context, layout: Option<&LayoutInfo>) -> Option<xkb::Keymap> {
    fn field(value: Option<&String>) -> &str {
        value.map_or("", String::as_str)
    }
    let (rules, model, name, variant, options) = match layout {
        Some(info) => (
            field(info.rules.as_ref()),
            field(info.model.as_ref()),
            info.layout.as_str(),
            field(info.variant.as_ref()),
            info.options.clone(),
        ),
        None => ("", "", "", "", None),
    };
    // An empty name is xkbcommon's "use the default", as the C++'s null is.
    xkb::Keymap::new_from_names(
        context,
        rules,
        model,
        name,
        variant,
        options,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
}

/// The virtual keyboard's character map for `keymap`, as `buildCharMap`
/// reads it: each key's text unshifted, then with Left Shift down.
#[must_use]
pub fn char_map(keymap: &xkb::Keymap) -> CharMap {
    let mut state = xkb::State::new(keymap);
    let shift = xkb::Keycode::new(u32::from(KEY_LEFTSHIFT) + EVDEV_OFFSET);
    let single = |text: String| {
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if text.len() == 1 => Some(c),
            _ => None,
        }
    };
    CharMap::build(|code| {
        let key = xkb::Keycode::new(u32::from(code) + EVDEV_OFFSET);
        let unshifted = single(state.key_get_utf8(key));
        state.update_key(shift, xkb::KeyDirection::Down);
        let shifted = single(state.key_get_utf8(key));
        state.update_key(shift, xkb::KeyDirection::Up);
        (unshifted, shifted)
    })
}

impl XkbKeys {
    /// The default keymap, as the C++ compiles it at start-up.
    ///
    /// # Errors
    ///
    /// When xkbcommon cannot compile a keymap at all — no
    /// `xkeyboard-config` data, usually.
    pub fn new() -> Result<(Self, CharMap), String> {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = compile(&context, None)
            .ok_or_else(|| "xkbcommon could not compile the default keymap".to_owned())?;
        let map = char_map(&keymap);
        let state = xkb::State::new(&keymap);
        Ok((Self { context, state }, map))
    }

    fn keycode(code: u16) -> xkb::Keycode {
        xkb::Keycode::new(u32::from(code) + EVDEV_OFFSET)
    }
}

impl KeyState for XkbKeys {
    fn text(&self, code: u16) -> String {
        self.state.key_get_utf8(Self::keycode(code))
    }

    fn press(&mut self, code: u16) {
        self.state
            .update_key(Self::keycode(code), xkb::KeyDirection::Down);
    }

    fn release(&mut self, code: u16) {
        self.state
            .update_key(Self::keycode(code), xkb::KeyDirection::Up);
    }

    fn modifiers_held(&self) -> bool {
        [
            xkb::MOD_NAME_SHIFT,
            xkb::MOD_NAME_CTRL,
            xkb::MOD_NAME_ALT,
            xkb::MOD_NAME_LOGO,
        ]
        .into_iter()
        .any(|name| {
            self.state
                .mod_name_is_active(name, xkb::STATE_MODS_DEPRESSED)
        })
    }

    fn set_layout(&mut self, layout: &LayoutInfo) -> Result<CharMap, String> {
        let keymap = compile(&self.context, Some(layout))
            .ok_or_else(|| format!("xkbcommon could not compile layout {:?}", layout.layout))?;
        let map = char_map(&keymap);
        self.state = xkb::State::new(&keymap);
        Ok(map)
    }
}

/// A US QWERTY keyboard as a table: what the tests replay recordings on.
///
/// It answers what xkbcommon's `us` layout answers for the keys a recording
/// uses — Shift, Caps Lock, Ctrl turning letters into control characters,
/// Backspace as `"\b"` — without needing `xkeyboard-config` on the machine.
#[derive(Debug, Clone, Default)]
pub struct UsKeys {
    held: BTreeSet<u16>,
    caps: bool,
}

/// (evdev code, unshifted, shifted) for the printable keys.
const US: &[(u16, char, char)] = &[
    (2, '1', '!'),
    (3, '2', '@'),
    (4, '3', '#'),
    (5, '4', '$'),
    (6, '5', '%'),
    (7, '6', '^'),
    (8, '7', '&'),
    (9, '8', '*'),
    (10, '9', '('),
    (11, '0', ')'),
    (12, '-', '_'),
    (13, '=', '+'),
    (16, 'q', 'Q'),
    (17, 'w', 'W'),
    (18, 'e', 'E'),
    (19, 'r', 'R'),
    (20, 't', 'T'),
    (21, 'y', 'Y'),
    (22, 'u', 'U'),
    (23, 'i', 'I'),
    (24, 'o', 'O'),
    (25, 'p', 'P'),
    (26, '[', '{'),
    (27, ']', '}'),
    (30, 'a', 'A'),
    (31, 's', 'S'),
    (32, 'd', 'D'),
    (33, 'f', 'F'),
    (34, 'g', 'G'),
    (35, 'h', 'H'),
    (36, 'j', 'J'),
    (37, 'k', 'K'),
    (38, 'l', 'L'),
    (39, ';', ':'),
    (40, '\'', '"'),
    (41, '`', '~'),
    (43, '\\', '|'),
    (44, 'z', 'Z'),
    (45, 'x', 'X'),
    (46, 'c', 'C'),
    (47, 'v', 'V'),
    (48, 'b', 'B'),
    (49, 'n', 'N'),
    (50, 'm', 'M'),
    (51, ',', '<'),
    (52, '.', '>'),
    (53, '/', '?'),
    (57, ' ', ' '),
];

/// Left/right Shift, Ctrl, Alt, Super.
const SHIFT: [u16; 2] = [42, 54];
const CTRL: [u16; 2] = [29, 97];
const ALT: [u16; 2] = [56, 100];
const LOGO: [u16; 2] = [125, 126];
const CAPS_LOCK: u16 = 58;

impl UsKeys {
    /// A keyboard with nothing held.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn any(&self, keys: [u16; 2]) -> bool {
        keys.iter().any(|key| self.held.contains(key))
    }

    /// The character map this table implies, for the virtual keyboard.
    #[must_use]
    pub fn char_map() -> CharMap {
        CharMap::build(|code| {
            US.iter()
                .find(|(key, ..)| *key == code)
                .map_or((None, None), |(_, plain, shifted)| {
                    (Some(*plain), Some(*shifted))
                })
        })
    }
}

impl KeyState for UsKeys {
    fn text(&self, code: u16) -> String {
        match code {
            1 => return "\u{1b}".into(),
            14 => return "\u{8}".into(),
            15 => return "\t".into(),
            28 | 96 => return "\r".into(),
            111 => return "\u{7f}".into(),
            _ => {}
        }
        let Some(&(_, plain, shifted)) = US.iter().find(|(key, ..)| *key == code) else {
            return String::new();
        };
        let shift = self.any(SHIFT);
        let letter = plain.is_ascii_lowercase();
        let upper = if letter { shift != self.caps } else { shift };
        let c = if upper { shifted } else { plain };
        if self.any(CTRL) && c.is_ascii_alphabetic() {
            // xkb's control transformation: Ctrl+A is U+0001.
            let control = (c.to_ascii_uppercase() as u8) - b'@';
            return char::from(control).to_string();
        }
        c.to_string()
    }

    fn press(&mut self, code: u16) {
        if code == CAPS_LOCK {
            self.caps = !self.caps;
        }
        self.held.insert(code);
    }

    fn release(&mut self, code: u16) {
        self.held.remove(&code);
    }

    fn modifiers_held(&self) -> bool {
        self.any(SHIFT) || self.any(CTRL) || self.any(ALT) || self.any(LOGO)
    }

    fn set_layout(&mut self, layout: &LayoutInfo) -> Result<CharMap, String> {
        if layout.layout == "us" {
            Ok(Self::char_map())
        } else {
            Err(format!(
                "the table only knows `us`, not {:?}",
                layout.layout
            ))
        }
    }
}
