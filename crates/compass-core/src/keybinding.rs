//! Navigation chords: vim, emacs, or the arrow keys alone.
//!
//! Ports `KeyBindingService`
//! (`src/server/src/services/keybinding/keybinding-service.hpp`). The C++ is
//! written against `QKeyEvent`; this is written against nothing, so that
//! `compass-core` keeps knowing no front end. A caller translates its own key
//! events into [`Chord`] and gets back a [`Direction`] or nothing.
//!
//! # The default on Linux is vim, and that is not a typo
//!
//! `getMode` reads:
//!
//! ```cpp
//! if (keybinding == "vim") { return KeyBindingMode::Vim; }
//! if (keybinding == "emacs") { return KeyBindingMode::Emacs; }
//! #ifdef Q_OS_MACOS
//!   return KeyBindingMode::Native;
//! #else
//!   return KeyBindingMode::Vim;
//! #endif
//! ```
//!
//! So an unset `keybinding`, or the literal `"default"`, gives **vim chords on
//! Linux** — `Ctrl+J` and `Ctrl+K` move the selection. A port that made
//! `"default"` mean "arrows only" would quietly drop a binding every existing
//! user has.
//!
//! # `usesOnly`, and why an exact match is the right reading
//!
//! The C++ requires the named modifiers and forbids every other one except
//! `Keypad` and `GroupSwitch`:
//!
//! ```cpp
//! return (mods & required) == required &&
//!        (mods & ~(required | Qt::KeypadModifier | Qt::GroupSwitchModifier)) == 0;
//! ```
//!
//! [`Modifiers`] models the four that a chord can require, so "required and
//! nothing else" is an equality. The two Qt modifiers left out are not
//! modelled at all, which has the same effect as excusing them.

use serde::{Deserialize, Serialize};

/// Which chord scheme is in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scheme {
    /// Arrow keys only.
    Native,
    /// `Ctrl+J/K/H/L`.
    Vim,
    /// `Ctrl+N/P` and `Ctrl+Alt+B/F`.
    Emacs,
}

impl Scheme {
    /// The scheme a configured `keybinding` value names.
    ///
    /// Anything other than `"vim"` or `"emacs"` — including `"default"`, an
    /// empty string and a typo — is the platform default, which on Linux is
    /// [`Scheme::Vim`].
    #[must_use]
    pub fn from_config(keybinding: &str) -> Self {
        match keybinding {
            "vim" => Self::Vim,
            "emacs" => Self::Emacs,
            _ => Self::platform_default(),
        }
    }

    /// The default for the platform this build targets.
    #[must_use]
    pub fn platform_default() -> Self {
        if cfg!(target_os = "macos") {
            Self::Native
        } else {
            Self::Vim
        }
    }
}

impl Default for Scheme {
    fn default() -> Self {
        Self::platform_default()
    }
}

/// Which way a chord moves the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Towards the first row.
    Up,
    /// Towards the last row.
    Down,
    /// Out of a column, or back.
    Left,
    /// Into a column, or forward.
    Right,
}

/// The modifiers held down, as a chord can require them.
///
/// Physical keys: on macOS Qt swaps Control and Meta, and the C++ names the
/// physical Control key `PHYSICAL_CTRL` for exactly these chords. `ctrl` here
/// is that physical key, so a caller on macOS passes its Meta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    /// The physical Control key.
    pub ctrl: bool,
    /// Alt / Option.
    pub alt: bool,
    /// Shift.
    pub shift: bool,
    /// Super / Command.
    pub logo: bool,
}

impl Modifiers {
    /// Just Control.
    pub const CTRL: Self = Self {
        ctrl: true,
        alt: false,
        shift: false,
        logo: false,
    };

    /// Control and Alt together.
    pub const CTRL_ALT: Self = Self {
        ctrl: true,
        alt: true,
        shift: false,
        logo: false,
    };
}

/// A key press, as far as this module cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    /// The character the key produces, lower-cased by the caller or not —
    /// [`navigation`] compares case-insensitively, because `Ctrl+Shift+J` is
    /// not a navigation chord and `Ctrl+J` on a layout that reports `J` is.
    pub key: char,
    /// What was held down.
    pub modifiers: Modifiers,
}

impl Chord {
    /// A chord from its parts.
    #[must_use]
    pub fn new(key: char, modifiers: Modifiers) -> Self {
        Self { key, modifiers }
    }
}

/// The direction `chord` moves the selection under `scheme`, if any.
///
/// `None` covers three cases that are all "not a navigation chord": the wrong
/// key, the wrong modifiers, and extra modifiers on top of the right ones.
#[must_use]
pub fn navigation(scheme: Scheme, chord: Chord) -> Option<Direction> {
    let key = chord.key.to_ascii_lowercase();
    let mods = chord.modifiers;

    match scheme {
        // Arrows only; the caller handles those itself, because they need no
        // scheme to interpret.
        Scheme::Native => None,
        Scheme::Vim if mods == Modifiers::CTRL => match key {
            'j' => Some(Direction::Down),
            'k' => Some(Direction::Up),
            'h' => Some(Direction::Left),
            'l' => Some(Direction::Right),
            _ => None,
        },
        Scheme::Emacs if mods == Modifiers::CTRL => match key {
            'n' => Some(Direction::Down),
            'p' => Some(Direction::Up),
            _ => None,
        },
        Scheme::Emacs if mods == Modifiers::CTRL_ALT => match key {
            'b' => Some(Direction::Left),
            'f' => Some(Direction::Right),
            _ => None,
        },
        Scheme::Vim | Scheme::Emacs => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const CPP: &str = "src/server/src/services/keybinding/keybinding-service.hpp";

    fn read_cpp() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("two levels below the repository root")
            .join(CPP);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }

    #[test]
    fn the_default_on_linux_is_vim_and_the_cpp_still_says_so() {
        // The single most surprising line in the port. If the C++ ever stops
        // defaulting to vim off macOS, this fails here rather than by users
        // losing Ctrl+J.
        let cpp = read_cpp();
        let tail = cpp
            .split("static KeyBindingMode getMode")
            .nth(1)
            .expect("getMode is still there");
        let body = tail.split("\n  }").next().expect("its body");
        assert!(
            body.contains("return KeyBindingMode::Vim;"),
            "{CPP}'s getMode no longer falls back to Vim: {body}"
        );

        assert_eq!(Scheme::from_config("default"), Scheme::platform_default());
        assert_eq!(Scheme::from_config(""), Scheme::platform_default());
        assert_eq!(Scheme::from_config("nonsense"), Scheme::platform_default());
        #[cfg(not(target_os = "macos"))]
        assert_eq!(Scheme::platform_default(), Scheme::Vim);
    }

    #[test]
    fn the_two_named_schemes_are_named_exactly() {
        assert_eq!(Scheme::from_config("vim"), Scheme::Vim);
        assert_eq!(Scheme::from_config("emacs"), Scheme::Emacs);
        assert_eq!(
            Scheme::from_config("Vim"),
            Scheme::platform_default(),
            "the C++ compares the string exactly; a capitalised value is not the vim scheme"
        );
    }

    #[test]
    fn the_vim_chords_are_the_cpps() {
        for (key, direction) in [
            ('j', Direction::Down),
            ('k', Direction::Up),
            ('h', Direction::Left),
            ('l', Direction::Right),
        ] {
            assert_eq!(
                navigation(Scheme::Vim, Chord::new(key, Modifiers::CTRL)),
                Some(direction),
                "Ctrl+{key} should move {direction:?}"
            );
        }
    }

    #[test]
    fn the_emacs_chords_are_the_cpps_including_the_alt_pair() {
        assert_eq!(
            navigation(Scheme::Emacs, Chord::new('n', Modifiers::CTRL)),
            Some(Direction::Down)
        );
        assert_eq!(
            navigation(Scheme::Emacs, Chord::new('p', Modifiers::CTRL)),
            Some(Direction::Up)
        );
        // `isLeftKey`/`isRightKey` for emacs require PHYSICAL_CTRL|Alt.
        assert_eq!(
            navigation(Scheme::Emacs, Chord::new('b', Modifiers::CTRL_ALT)),
            Some(Direction::Left)
        );
        assert_eq!(
            navigation(Scheme::Emacs, Chord::new('f', Modifiers::CTRL_ALT)),
            Some(Direction::Right)
        );
        assert_eq!(
            navigation(Scheme::Emacs, Chord::new('b', Modifiers::CTRL)),
            None,
            "Ctrl+B alone is not the emacs left chord"
        );
    }

    #[test]
    fn an_extra_modifier_makes_it_a_different_chord() {
        // `usesOnly`. Ctrl+Shift+J is a chord an extension or the front end
        // may want for something else, and must not move the selection.
        let mut mods = Modifiers::CTRL;
        mods.shift = true;
        assert_eq!(navigation(Scheme::Vim, Chord::new('j', mods)), None);

        let mut mods = Modifiers::CTRL;
        mods.logo = true;
        assert_eq!(navigation(Scheme::Vim, Chord::new('j', mods)), None);
    }

    #[test]
    fn a_bare_letter_moves_nothing() {
        // The important negative: typing `j` into the search box must type a
        // `j`. A port that matched on the key alone would make the launcher
        // unusable in the most literal way.
        assert_eq!(
            navigation(Scheme::Vim, Chord::new('j', Modifiers::default())),
            None
        );
        assert_eq!(
            navigation(Scheme::Emacs, Chord::new('n', Modifiers::default())),
            None
        );
    }

    #[test]
    fn a_schemes_chords_do_not_answer_in_another_scheme() {
        assert_eq!(
            navigation(Scheme::Emacs, Chord::new('j', Modifiers::CTRL)),
            None,
            "Ctrl+J is vim's, and under emacs it is free for something else"
        );
        assert_eq!(
            navigation(Scheme::Vim, Chord::new('n', Modifiers::CTRL)),
            None
        );
        assert_eq!(
            navigation(Scheme::Native, Chord::new('j', Modifiers::CTRL)),
            None,
            "the native scheme has no chords; arrows are the caller's business"
        );
    }

    #[test]
    fn the_case_of_the_reported_key_does_not_matter() {
        // A layout that reports the shifted name of the key still produces
        // `Ctrl+J` as far as the user is concerned -- and `usesOnly` above
        // already refuses an actual Shift.
        assert_eq!(
            navigation(Scheme::Vim, Chord::new('J', Modifiers::CTRL)),
            Some(Direction::Down)
        );
    }
}
