//! The virtual keyboard: what a snippet's text becomes on the wire to
//! `/dev/uinput`.
//!
//! A port of `linuxutils::UInputKeyboard` (`src/lib/linux-utils/`), minus the
//! device and the keymap compiler.
//!
//! # Why this is a model over a sink
//!
//! The C++ class is three things wedged together: an ioctl dance that creates
//! a uinput device, an xkbcommon call that compiles a keymap, and a protocol
//! that turns characters into a stream of `input_event` writes. Only the third
//! decides whether a pasted snippet arrives intact, and only the third can be
//! tested without root and a real device node. So the protocol takes an
//! [`EventSink`]: the tests give it a recorder, and the daemon gives it a file
//! descriptor.
//!
//! The delays are part of the protocol, not an implementation detail — a
//! virtual keyboard that types faster than the compositor reads drops
//! characters — so the sink is asked to wait rather than being made to guess.

use std::time::Duration;

/// The default pause between keys, from `DEFAULT_KEY_DELAY_US`.
pub const DEFAULT_KEY_DELAY_US: u32 = 2000;

/// The pause used around a modified key, from `MODIFIER_DELAY_US`.
///
/// Five times the ordinary delay, because a compositor that sees the key
/// before it has processed the modifier press types the unshifted character.
pub const MODIFIER_DELAY_US: u32 = 10_000;

/// What evdev keycodes are offset by to become xkb keycodes.
pub const EVDEV_OFFSET: u32 = 8;

/// The largest character this keyboard can type: the map is ASCII only.
pub const CHARMAP_SIZE: usize = 128;

/// The lowest key the C++ enables on the device, `KEY_ESC`.
pub const FIRST_KEY: u16 = 1;

/// One past the highest key the C++ enables on the device.
pub const KEY_LIMIT: u16 = 256;

/// `KEY_LEFTCTRL`.
pub const KEY_LEFTCTRL: u16 = 29;

/// `KEY_LEFTSHIFT`.
pub const KEY_LEFTSHIFT: u16 = 42;

/// How the device announces itself, from the `uinput_setup` the C++ fills in.
pub mod device {
    /// `BUS_VIRTUAL`.
    pub const BUSTYPE: u16 = 0x06;
    /// The vendor id the C++ invents.
    pub const VENDOR: u16 = 0x1234;
    /// The product id the C++ invents.
    pub const PRODUCT: u16 = 0x5678;
    /// The device version.
    pub const VERSION: u16 = 1;
    /// The name the device appears under in `/proc/bus/input/devices`.
    pub const NAME: &str = "compass-snippet-virtual-keyboard";
}

/// The modifiers the keyboard knows about.
///
/// # Four of these six do nothing
///
/// `applyMods` and `clearMods` test only `Ctrl` and `Shift`. `Capslock`,
/// `Alt`, `Logo` and `Altgr` are declared, can be passed, and are silently
/// ignored — a caller asking for Alt+F gets a bare F. That is reproduced, and
/// pinned by a test, because a port that quietly made Alt work would send a
/// keystroke the C++ engine does not and there is no way for the caller to tell which
/// build it is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers(u8);

impl Modifiers {
    /// No modifiers.
    pub const NONE: Self = Self(0);
    /// Shift.
    pub const SHIFT: Self = Self(1);
    /// Caps lock — declared, never applied.
    pub const CAPSLOCK: Self = Self(1 << 1);
    /// Control.
    pub const CTRL: Self = Self(1 << 2);
    /// Alt — declared, never applied.
    pub const ALT: Self = Self(1 << 3);
    /// The super/meta key — declared, never applied.
    pub const LOGO: Self = Self(1 << 4);
    /// AltGr — declared, never applied.
    pub const ALTGR: Self = Self(1 << 5);

    /// The raw bits, as the C++ passes them around as `int mods`.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Build from raw bits.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    /// Whether every modifier in `other` is set here.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether no modifier at all is set.
    ///
    /// This is what chooses between the ordinary and the modifier delay, so it
    /// is true for `Modifiers::ALT` being absent but *false* for `ALT` being
    /// present even though `ALT` is never applied: the C++ branches on
    /// `mods ? ... : ...`, the raw integer, not on what it will do with it.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The union of two sets.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// One event written to the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEvent {
    /// `EV_KEY` with value 1.
    Press(u16),
    /// `EV_KEY` with value 0.
    Release(u16),
    /// `EV_SYN`/`SYN_REPORT`, which tells the reader the batch is complete.
    Sync,
}

/// Where the keyboard's events go, and what makes it wait.
pub trait EventSink {
    /// Write one event.
    fn emit(&mut self, event: KeyEvent);
    /// Wait, so the reader can keep up.
    fn delay(&mut self, duration: Duration);
}

/// Which key, with which modifiers, produces one character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyRecord {
    /// The evdev keycode.
    pub code: u16,
    /// The modifiers to hold while pressing it.
    pub mods: Modifiers,
}

/// ASCII character to keystroke, as `buildCharMap` computes it from a keymap.
#[derive(Debug, Clone)]
pub struct CharMap {
    /// One slot per ASCII character; `None` where the keymap types nothing.
    entries: [Option<KeyRecord>; CHARMAP_SIZE],
}

impl Default for CharMap {
    fn default() -> Self {
        Self::new()
    }
}

impl CharMap {
    /// An empty map, which types nothing at all.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: [None; CHARMAP_SIZE],
        }
    }

    /// Build a map from what a keymap reports for each key.
    ///
    /// `key_output` is asked, for each evdev code in the range the device
    /// enables, what that key types unshifted and what it types with shift
    /// held — which is exactly the pair `buildCharMap` reads out of
    /// `xkb_state_key_get_utf8`. Keeping xkbcommon on the other side of this
    /// argument is what lets the rule below be tested without compiling a
    /// keymap.
    ///
    /// # First one wins, and unshifted beats shifted
    ///
    /// The C++ writes a slot only `if (m_charMap[idx].code == 0)`, and asks
    /// the unshifted question before the shifted one for each key in
    /// ascending order. So a layout where two keys both produce `/` types the
    /// lower-numbered one, and a character reachable both ways is typed
    /// without shift. Both follow from the order alone, which is why this
    /// takes a callback rather than a map.
    #[must_use]
    pub fn build(mut key_output: impl FnMut(u16) -> (Option<char>, Option<char>)) -> Self {
        let mut map = Self::new();
        for code in 0..u16::try_from(CHARMAP_SIZE * 2).unwrap_or(u16::MAX) {
            let (unshifted, shifted) = key_output(code);
            map.insert_if_empty(unshifted, code, Modifiers::NONE);
            map.insert_if_empty(shifted, code, Modifiers::SHIFT);
        }
        map
    }

    /// Record `character` as typed by `code` with `mods`, unless something
    /// already claims it.
    fn insert_if_empty(&mut self, character: Option<char>, code: u16, mods: Modifiers) {
        let Some(character) = character else { return };
        // A non-ASCII character's code point is at least 128, so the bound
        // below is the whole ASCII test; the C++ gets the same effect by only
        // writing when xkb_state_key_get_utf8 returned exactly one byte.
        let index = character as usize;
        if index >= CHARMAP_SIZE || self.entries[index].is_some() {
            return;
        }
        // The C++ stores evdev code 0 as "empty", so a character bound to
        // evdev code 0 is unreachable there. That key is KEY_RESERVED and
        // types nothing, so the two agree in practice.
        if code == 0 {
            return;
        }
        self.entries[index] = Some(KeyRecord { code, mods });
    }

    /// Bind `character` to `code` with `mods`, replacing any existing entry.
    pub fn set(&mut self, character: char, code: u16, mods: Modifiers) {
        if character.is_ascii() {
            self.entries[character as usize] = Some(KeyRecord { code, mods });
        }
    }

    /// What types `character`, if anything does.
    #[must_use]
    pub fn get(&self, character: char) -> Option<KeyRecord> {
        if !character.is_ascii() {
            return None;
        }
        self.entries[character as usize]
    }
}

/// The virtual keyboard's protocol, over any [`EventSink`].
#[derive(Debug)]
pub struct VirtualKeyboard<S: EventSink> {
    /// Where events go.
    sink: S,
    /// The pause between keys.
    key_delay_us: u32,
    /// What types what.
    char_map: CharMap,
}

impl<S: EventSink> VirtualKeyboard<S> {
    /// A keyboard writing to `sink`, with the default delay and an empty map.
    pub fn new(sink: S) -> Self {
        Self {
            sink,
            key_delay_us: DEFAULT_KEY_DELAY_US,
            char_map: CharMap::new(),
        }
    }

    /// Replace the character map, as `setKeymap` does.
    pub fn set_char_map(&mut self, char_map: CharMap) {
        self.char_map = char_map;
    }

    /// Set the pause between keys, in microseconds.
    pub fn set_key_delay_us(&mut self, micros: u32) {
        self.key_delay_us = micros;
    }

    /// The sink, for a caller that has to read back what was written.
    pub fn sink(&self) -> &S {
        &self.sink
    }

    /// The sink, mutably: the input server waits through it between the
    /// backspaces and the paste (`prePasteDelayUs`), so a test's recorder
    /// sees that pause where the C++ `usleep`s.
    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    /// Press and release `code`, syncing after each, with no modifiers held.
    pub fn send_key(&mut self, code: u16) {
        self.sink.emit(KeyEvent::Press(code));
        self.sink.emit(KeyEvent::Sync);
        self.sink.emit(KeyEvent::Release(code));
        self.sink.emit(KeyEvent::Sync);
    }

    /// Press and release `code` with `mods` held.
    ///
    /// The extra sync after the key and the one after the modifiers are
    /// released are both in the C++, and the first is redundant —
    /// [`Self::send_key`] already ends with one. Reproduced because a reader
    /// counting syncs is a reader that can tell the two builds apart.
    pub fn send_key_with_mods(&mut self, code: u16, mods: Modifiers) {
        let around = if mods.is_empty() {
            self.key_delay_us
        } else {
            MODIFIER_DELAY_US
        };

        self.apply_mods(mods);
        self.wait(around);
        self.send_key(code);
        self.wait(self.key_delay_us);
        self.sink.emit(KeyEvent::Sync);
        self.wait(around);
        self.clear_mods(mods);
        self.sink.emit(KeyEvent::Sync);
    }

    /// Press and release `code` `count` times, pausing between each.
    ///
    /// Note that this pauses *after* the last press too, and holds no
    /// modifiers whatever the caller wanted.
    pub fn repeat_key(&mut self, code: u16, count: u32) {
        for _ in 0..count {
            self.send_key(code);
            self.wait(self.key_delay_us);
        }
    }

    /// Type `text`, skipping every character the map cannot produce.
    ///
    /// A character with no binding is dropped silently, which is how a snippet
    /// containing an em dash arrives with a hole in it rather than not at all.
    pub fn type_text(&mut self, text: &str) {
        for character in text.chars() {
            let Some(record) = self.char_map.get(character) else {
                continue;
            };
            self.send_key_with_mods(record.code, record.mods);
        }
    }

    /// Hold down the modifiers the C++ actually applies: Ctrl, then Shift.
    fn apply_mods(&mut self, mods: Modifiers) {
        if mods.contains(Modifiers::CTRL) {
            self.sink.emit(KeyEvent::Press(KEY_LEFTCTRL));
        }
        if mods.contains(Modifiers::SHIFT) {
            self.sink.emit(KeyEvent::Press(KEY_LEFTSHIFT));
        }
    }

    /// Release them, in the same order.
    fn clear_mods(&mut self, mods: Modifiers) {
        if mods.contains(Modifiers::CTRL) {
            self.sink.emit(KeyEvent::Release(KEY_LEFTCTRL));
        }
        if mods.contains(Modifiers::SHIFT) {
            self.sink.emit(KeyEvent::Release(KEY_LEFTSHIFT));
        }
    }

    /// Ask the sink to wait `micros` microseconds.
    fn wait(&mut self, micros: u32) {
        self.sink.delay(Duration::from_micros(u64::from(micros)));
    }
}
