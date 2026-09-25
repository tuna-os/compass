//! The key-sequence state machine: raw `EV_KEY` events in, snippet events out.
//!
//! A port of the keyboard branch of `SnippetService::listen`
//! (`src/snippet/src/server.cpp`) together with `emitExpansion`,
//! `flushPendingExpansion` and `hasActiveModifiers`. Text matching is
//! [`compass_core::snippet::Matcher`]; this adds what depends on the keys
//! themselves — press versus repeat versus release, the undo that Backspace
//! arms, and holding an expansion back while a modifier is down.
//!
//! # The order inside one press is load-bearing
//!
//! For a press (`value == 1`) or a repeat (`value == 2`) the C++:
//!
//! 1. asks xkb what the key types, **before** the key is pressed in the xkb
//!    state (so Shift typed alone types nothing, and `a` with Shift already
//!    held types `A`);
//! 2. presses it in the state, for a press only — a repeat does not;
//! 3. settles a pending undo: Backspace **press** straight after an expansion
//!    is the undo and nothing else happens; any other press disarms it (a
//!    repeat neither fires nor disarms it);
//! 4. feeds the text to the matcher, which checks every trigger.
//!
//! A release (`value == 0`) only releases the key, and if an expansion was
//! held back for a modifier and none is down any more, lets it go.
//!
//! # Why an expansion waits for the modifiers
//!
//! A trigger that ends in a shifted character (`;Sig`, `:)`) completes with
//! Shift still down. Injecting Ctrl+V then would reach the application as
//! Ctrl+Shift+V. So the expansion is held until every modifier is up, or for
//! [`MODIFIER_TIMEOUT`] at most — the caller's clock, since only the caller
//! knows how long it has been waiting.

use std::time::Duration;

use compass_core::input_server::wire::{Event, LayoutInfo};
use compass_core::snippet::{ExpansionMode, Matcher, Snippet};
use compass_platform_linux::keyboard::CharMap;

/// `KEY_BACKSPACE`.
pub const KEY_BACKSPACE: u16 = 14;

/// How long a held-back expansion waits for modifiers to be released:
/// `MODIFIER_TIMEOUT_MS`.
pub const MODIFIER_TIMEOUT: Duration = Duration::from_millis(3000);

/// The keyboard's xkb state, as far as the tracker needs it.
///
/// The real one is [`crate::keymap::XkbKeys`]; tests use a table so recorded
/// event sequences can be replayed without compiling a keymap.
pub trait KeyState {
    /// What `code` (an evdev keycode) types in the current state:
    /// `xkb_state_key_get_utf8`, so `"\u{8}"` for Backspace and `""` for a
    /// modifier.
    fn text(&self, code: u16) -> String;
    /// The key went down.
    fn press(&mut self, code: u16);
    /// The key came up.
    fn release(&mut self, code: u16);
    /// Whether Shift, Ctrl, Alt or Super is held (`XKB_STATE_MODS_DEPRESSED`).
    fn modifiers_held(&self) -> bool;
    /// Switch to another layout, answering the virtual keyboard's character
    /// map for the same one.
    ///
    /// # Errors
    ///
    /// When the layout does not compile; the old one is kept.
    fn set_layout(&mut self, layout: &LayoutInfo) -> Result<CharMap, String>;
}

/// The typed text, the armed undo and the held-back expansion.
#[derive(Debug)]
pub struct Tracker<K> {
    keys: K,
    matcher: Matcher,
    undo: Option<String>,
    pending: Option<String>,
}

impl<K: KeyState> Tracker<K> {
    /// A tracker reading keys through `keys`, with no snippets.
    pub fn new(keys: K) -> Self {
        Self {
            keys,
            matcher: Matcher::new(),
            undo: None,
            pending: None,
        }
    }

    /// The key state.
    pub fn keys(&self) -> &K {
        &self.keys
    }

    /// The key state, mutably.
    pub fn keys_mut(&mut self) -> &mut K {
        &mut self.keys
    }

    /// `createSnippet`.
    pub fn add(&mut self, trigger: String, mode: ExpansionMode) {
        self.matcher.add(Snippet::new(trigger, mode));
    }

    /// `removeSnippet`.
    pub fn remove(&mut self, trigger: &str) -> bool {
        self.matcher.remove(trigger)
    }

    /// `resetContext`, and the interrupted-injection path: forgets the typed
    /// text. The armed undo and a held-back expansion are kept, as in the C++.
    pub fn reset(&mut self) {
        self.matcher.reset();
    }

    /// What has been typed.
    #[must_use]
    pub fn buffer(&self) -> &str {
        self.matcher.buffer()
    }

    /// Whether an expansion is waiting for the modifiers.
    #[must_use]
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// One `EV_KEY` event from a keyboard: `code` and `value` as evdev gives
    /// them (0 release, 1 press, 2 repeat).
    pub fn key(&mut self, code: u16, value: i32) -> Option<Event> {
        if value == 0 {
            self.keys.release(code);
            if self.pending.is_some() && !self.keys.modifiers_held() {
                return self.flush_pending();
            }
            return None;
        }
        // `inputEv.value <= 2`: anything larger is not a key event evdev
        // sends, and is ignored.
        if value > 2 {
            return None;
        }

        let text = self.keys.text(code);
        if value == 1 {
            self.keys.press(code);
        }

        if value == 1 && self.undo.is_some() {
            if code == KEY_BACKSPACE {
                return self.undo.take().map(Event::Undo);
            }
            self.undo = None;
        }

        let trigger = self.matcher.on_key(&text, code == KEY_BACKSPACE)?;
        self.expand(trigger)
    }

    /// Lets a held-back expansion go: the modifiers came up, or the caller
    /// waited [`MODIFIER_TIMEOUT`] for them.
    pub fn flush_pending(&mut self) -> Option<Event> {
        let trigger = self.pending.take()?;
        self.undo = Some(trigger.clone());
        Some(Event::Trigger(trigger))
    }

    /// `emitExpansion`: now, or once the modifiers are up.
    fn expand(&mut self, trigger: String) -> Option<Event> {
        if self.keys.modifiers_held() {
            self.pending = Some(trigger);
            return None;
        }
        self.undo = Some(trigger.clone());
        Some(Event::Trigger(trigger))
    }
}
