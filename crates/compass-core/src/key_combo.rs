//! A key combination as the C++ stores and records it: `Keyboard::Shortcut`
//! (`src/server/src/internal/keyboard/keyboard.cpp`), the shortcut recorder's
//! capture (`ShortcutRecorderCapture.qml`) and its validation
//! (`shortcut_conflict::validate`).
//!
//! A root item's keyboard shortcut is kept in the configuration as
//! `Shortcut::toString` writes it — `super+control+alt+shift+KEY`, modifiers
//! in that order, the key upper-cased — so a file either engine wrote reads
//! the same in the other. Like [`crate::keybinding`], nothing here knows a
//! front end: a caller translates its key events into [`Key`] names.
//!
//! No crate covers this: `global-hotkey` and `keyboard-types` spell keys their
//! own way (`KeyA`, `Control`), and the point is the C++ spelling.

/// The four modifiers a combination can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Modifiers {
    /// Super / Meta / the Windows key.
    pub super_key: bool,
    /// Control.
    pub control: bool,
    /// Alt / Option.
    pub alt: bool,
    /// Shift.
    pub shift: bool,
}

impl Modifiers {
    /// Whether none is held.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self == Self::default()
    }

    fn with(mut self, modifier: Modifier, on: bool) -> Self {
        match modifier {
            Modifier::Super => self.super_key = on,
            Modifier::Control => self.control = on,
            Modifier::Alt => self.alt = on,
            Modifier::Shift => self.shift = on,
        }
        self
    }

    fn has(self, modifier: Modifier) -> bool {
        match modifier {
            Modifier::Super => self.super_key,
            Modifier::Control => self.control,
            Modifier::Alt => self.alt,
            Modifier::Shift => self.shift,
        }
    }
}

/// One modifier, as a key of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Modifier {
    /// Super.
    Super,
    /// Control.
    Control,
    /// Alt.
    Alt,
    /// Shift.
    Shift,
}

impl Modifier {
    /// In `toString`'s order.
    const ALL: [Self; 4] = [Self::Super, Self::Control, Self::Alt, Self::Shift];

    /// Its name in a stored shortcut.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Super => "super",
            Self::Control => "control",
            Self::Alt => "alt",
            Self::Shift => "shift",
        }
    }

    /// `modifierMap`: the spellings a stored shortcut may use.
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "super" | "meta" | "windows" => Self::Super,
            "cmd" | "command" | "ctrl" | "control" => Self::Control,
            "option" | "opt" | "alt" => Self::Alt,
            "shift" => Self::Shift,
            _ => return None,
        })
    }
}

/// A key: a modifier pressed on its own, or any other key by its `keyMap`
/// name (`a`, `5`, `.`, `return`, `arrowup`, `f5`, …), lower-case.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Key {
    /// A modifier key.
    Modifier(Modifier),
    /// Any other key.
    Named(String),
}

/// The C++ `keyMap`'s names for the keys that are not a character.
const NAMED_KEYS: &[&str] = &[
    "return",
    "delete",
    "tab",
    "arrowup",
    "arrowdown",
    "arrowleft",
    "arrowright",
    "pageup",
    "pagedown",
    "home",
    "end",
    "space",
    "escape",
    "enter",
    "backspace",
];

/// The characters `keyMap` names.
const CHARACTER_KEYS: &str = "abcdefghijklmnopqrstuvwxyz0123456789.,;=+-[]{}()/\\'`^@$";

impl Key {
    /// `keyFromString`: a key's name, any case; `None` for one the C++ does
    /// not know.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let lowered = name.to_lowercase();
        if let Some(modifier) = match lowered.as_str() {
            "super" => Some(Modifier::Super),
            "control" => Some(Modifier::Control),
            "alt" => Some(Modifier::Alt),
            "shift" => Some(Modifier::Shift),
            _ => None,
        } {
            return Some(Self::Modifier(modifier));
        }
        if lowered == "deleteforward" {
            return Some(Self::Named("backspace".to_owned()));
        }
        if NAMED_KEYS.contains(&lowered.as_str()) || function_key_number(&lowered).is_some() {
            return Some(Self::Named(lowered));
        }
        let mut chars = lowered.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if CHARACTER_KEYS.contains(c) => Some(Self::Named(lowered)),
            _ => None,
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Modifier(modifier) => modifier.name(),
            Self::Named(name) => name,
        }
    }

    /// `isFunctionKey`: F1 to F35.
    #[must_use]
    pub fn is_function_key(&self) -> bool {
        matches!(self, Self::Named(name) if function_key_number(name).is_some())
    }
}

fn function_key_number(name: &str) -> Option<u8> {
    let number: u8 = name.strip_prefix('f')?.parse().ok()?;
    (1..=35).contains(&number).then_some(number)
}

/// A key and the modifiers held with it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyCombo {
    /// The key.
    pub key: Key,
    /// What was held with it.
    pub modifiers: Modifiers,
}

impl KeyCombo {
    /// A combination from its parts.
    #[must_use]
    pub fn new(key: Key, modifiers: Modifiers) -> Self {
        Self { key, modifiers }
    }

    /// `Shortcut(const QString &)`: a stored shortcut read back. A token
    /// that is neither a modifier nor a key makes the whole string invalid;
    /// modifiers alone are the last one pressed on its own, as the recorder
    /// stores a modifier-only chord. An empty token is a literal `+`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        if text.is_empty() {
            return None;
        }
        let parts: Vec<&str> = text.split('+').collect();
        let mut tokens = Vec::with_capacity(parts.len());
        let mut at = 0;
        while at < parts.len() {
            if parts[at].is_empty() {
                tokens.push("+");
                at += 2;
            } else {
                tokens.push(parts[at]);
                at += 1;
            }
        }
        let mut modifiers = Modifiers::default();
        let mut key = None;
        let mut last_modifier = None;
        for token in tokens {
            if let Some(modifier) = Modifier::from_name(&token.to_lowercase()) {
                modifiers = modifiers.with(modifier, true);
                last_modifier = Some(modifier);
            } else {
                key = Some(Key::from_name(token)?);
            }
        }
        let key = match (key, last_modifier) {
            (Some(key), _) => key,
            (None, Some(modifier)) => {
                modifiers = modifiers.with(modifier, false);
                Key::Modifier(modifier)
            }
            (None, None) => return None,
        };
        Some(Self { key, modifiers })
    }

    /// `Shortcut::toString`: how the configuration keeps it.
    #[must_use]
    pub fn to_config_string(&self) -> String {
        let mut parts: Vec<String> = Modifier::ALL
            .into_iter()
            .filter(|modifier| self.modifiers.has(*modifier))
            .map(|modifier| modifier.name().to_owned())
            .collect();
        parts.push(self.key.name().to_uppercase());
        parts.join("+")
    }

    /// `isModifierOnly`.
    #[must_use]
    pub fn is_modifier_only(&self) -> bool {
        matches!(self.key, Key::Modifier(_))
    }

    /// The badge's tokens (`buildDisplayTokenSpecs`, Linux spellings):
    /// the modifiers, then the key.
    #[must_use]
    pub fn display_tokens(&self) -> Vec<String> {
        let mut modifiers = self.modifiers;
        if let Key::Modifier(modifier) = self.key {
            modifiers = modifiers.with(modifier, true);
        }
        let mut tokens: Vec<String> = Modifier::ALL
            .into_iter()
            .filter(|modifier| modifiers.has(*modifier))
            .map(|modifier| {
                match modifier {
                    Modifier::Super => "◈",
                    Modifier::Control => "Ctrl",
                    Modifier::Alt => "Alt",
                    Modifier::Shift => "Shift",
                }
                .to_owned()
            })
            .collect();
        if let Key::Named(name) = &self.key {
            let label = match name.as_str() {
                "return" | "enter" => "Enter".to_owned(),
                "tab" => "Tab".to_owned(),
                "space" => "Space".to_owned(),
                "backspace" => "⌫".to_owned(),
                "delete" => "⌦".to_owned(),
                "arrowup" => "↑".to_owned(),
                "arrowdown" => "↓".to_owned(),
                "arrowleft" => "←".to_owned(),
                "arrowright" => "→".to_owned(),
                "pageup" => "PgUp".to_owned(),
                "pagedown" => "PgDn".to_owned(),
                "home" => "Home".to_owned(),
                "end" => "End".to_owned(),
                "escape" => "Esc".to_owned(),
                other => other.to_uppercase(),
            };
            tokens.push(label);
        }
        tokens
    }
}

/// `shortcut_conflict::validate`'s first sentence.
pub const MODIFIER_REQUIRED: &str = "Modifier required";

/// Whether `combo` may be recorded for the item `exclude_id`: it needs a
/// modifier unless it is a function key or a modifier alone, and must not be
/// another item's already (`findConflict`, over `bound`: each item's id,
/// title and stored shortcut). The sentence to show when not.
///
/// # Errors
///
/// [`MODIFIER_REQUIRED`], or `Already bound to "<title>"`.
pub fn validate<'a>(
    combo: &KeyCombo,
    exclude_id: &str,
    bound: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>,
) -> Result<(), String> {
    if combo.modifiers.is_empty() && !combo.key.is_function_key() && !combo.is_modifier_only() {
        return Err(MODIFIER_REQUIRED.to_owned());
    }
    for (id, title, shortcut) in bound {
        if id != exclude_id && KeyCombo::parse(shortcut).as_ref() == Some(combo) {
            return Err(format!("Already bound to \"{title}\""));
        }
    }
    Ok(())
}

/// What a key event does to a recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recorded {
    /// Nothing yet (a modifier went down or came up with others held).
    Pending,
    /// A combination was pressed.
    Captured(KeyCombo),
    /// Escape or Backspace, bare: the host closes, or clears.
    Dismissed(Key),
}

/// The capture's chord tracking (`handleKey`): a combination is a key
/// pressed with modifiers, or modifiers pressed and released with no other
/// key in between, which records the modifiers alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Recorder {
    held: Modifiers,
    chord: Modifiers,
    consumed: bool,
}

impl Recorder {
    /// A key went down with `modifiers` held.
    pub fn press(&mut self, key: Key, modifiers: Modifiers) -> Recorded {
        if let Key::Modifier(modifier) = key {
            self.held = self.held.with(modifier, true);
            self.chord = self.chord.with(modifier, true);
            return Recorded::Pending;
        }
        if !self.held.is_empty() {
            self.consumed = true;
        }
        let dismisses = matches!(&key, Key::Named(name) if name == "escape" || name == "backspace");
        if dismisses && modifiers.is_empty() {
            return Recorded::Dismissed(key);
        }
        Recorded::Captured(KeyCombo { key, modifiers })
    }

    /// A key came up.
    pub fn release(&mut self, key: &Key) -> Recorded {
        let Key::Modifier(modifier) = *key else {
            return Recorded::Pending;
        };
        self.held = self.held.with(modifier, false);
        if !self.held.is_empty() {
            return Recorded::Pending;
        }
        let (chord, consumed) = (self.chord, self.consumed);
        *self = Self::default();
        if consumed || chord.is_empty() {
            return Recorded::Pending;
        }
        Recorded::Captured(KeyCombo {
            key: Key::Modifier(modifier),
            modifiers: chord.with(modifier, false),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> Key {
        Key::Named(name.to_owned())
    }

    const CTRL_SHIFT: Modifiers = Modifiers {
        super_key: false,
        control: true,
        alt: false,
        shift: true,
    };

    #[test]
    fn a_combination_is_stored_in_the_cpps_spelling_and_read_back() {
        let combo = KeyCombo::new(named("a"), CTRL_SHIFT);
        assert_eq!(combo.to_config_string(), "control+shift+A");
        assert_eq!(KeyCombo::parse("control+shift+A"), Some(combo.clone()));
        assert_eq!(KeyCombo::parse("Shift+CTRL+a"), Some(combo));
        let all = KeyCombo::new(
            named("space"),
            Modifiers {
                super_key: true,
                control: true,
                alt: true,
                shift: true,
            },
        );
        assert_eq!(all.to_config_string(), "super+control+alt+shift+SPACE");
        assert_eq!(KeyCombo::parse("super+control+alt+shift+SPACE"), Some(all));
        let plus = KeyCombo::parse("control++").expect("a literal plus");
        assert_eq!(plus.key, named("+"));
        assert!(plus.modifiers.control);
        assert_eq!(KeyCombo::parse("control+nope"), None);
        assert_eq!(KeyCombo::parse(""), None);
        let meta = KeyCombo::parse("control+super").expect("modifiers alone");
        assert_eq!(meta.key, Key::Modifier(Modifier::Super));
        assert!(meta.modifiers.control && !meta.modifiers.super_key);
        assert_eq!(meta.to_config_string(), "control+SUPER");
    }

    #[test]
    fn a_recording_is_a_key_with_modifiers_or_modifiers_released_alone() {
        let mut recorder = Recorder::default();
        assert_eq!(
            recorder.press(Key::Modifier(Modifier::Control), Modifiers::default()),
            Recorded::Pending
        );
        let ctrl = Modifiers {
            control: true,
            ..Modifiers::default()
        };
        assert_eq!(
            recorder.press(named("k"), ctrl),
            Recorded::Captured(KeyCombo::new(named("k"), ctrl))
        );
        // Releasing the modifier after a key was pressed records nothing more.
        assert_eq!(
            recorder.release(&Key::Modifier(Modifier::Control)),
            Recorded::Pending
        );

        // Super and Control pressed and let go: the last one released is the
        // key, the other its modifier.
        let _ = recorder.press(Key::Modifier(Modifier::Super), Modifiers::default());
        let _ = recorder.press(Key::Modifier(Modifier::Control), Modifiers::default());
        assert_eq!(
            recorder.release(&Key::Modifier(Modifier::Super)),
            Recorded::Pending
        );
        assert_eq!(
            recorder.release(&Key::Modifier(Modifier::Control)),
            Recorded::Captured(KeyCombo::new(
                Key::Modifier(Modifier::Control),
                Modifiers {
                    super_key: true,
                    ..Modifiers::default()
                }
            ))
        );

        assert_eq!(
            recorder.press(named("escape"), Modifiers::default()),
            Recorded::Dismissed(named("escape"))
        );
        assert_eq!(
            recorder.press(named("backspace"), ctrl),
            Recorded::Captured(KeyCombo::new(named("backspace"), ctrl))
        );
    }

    #[test]
    fn a_combination_needs_a_modifier_and_must_not_be_anothers() {
        let bound = [
            ("apps:firefox", "Firefox", "control+shift+A"),
            ("commands:emoji", "Emoji", "super+E"),
        ];
        assert_eq!(
            validate(&KeyCombo::new(named("a"), Modifiers::default()), "x", bound),
            Err(MODIFIER_REQUIRED.to_owned())
        );
        assert_eq!(
            validate(
                &KeyCombo::new(named("f5"), Modifiers::default()),
                "x",
                bound
            ),
            Ok(())
        );
        assert_eq!(
            validate(
                &KeyCombo::new(Key::Modifier(Modifier::Super), Modifiers::default()),
                "x",
                bound
            ),
            Ok(())
        );
        let taken = KeyCombo::new(named("a"), CTRL_SHIFT);
        assert_eq!(
            validate(&taken, "x", bound),
            Err("Already bound to \"Firefox\"".to_owned())
        );
        assert_eq!(validate(&taken, "apps:firefox", bound), Ok(()), "its own");
    }

    #[test]
    fn the_badge_names_the_modifiers_then_the_key() {
        assert_eq!(
            KeyCombo::new(named("arrowup"), CTRL_SHIFT).display_tokens(),
            ["Ctrl", "Shift", "↑"]
        );
        assert_eq!(
            KeyCombo::parse("super+e").unwrap().display_tokens(),
            ["◈", "E"]
        );
        assert_eq!(
            KeyCombo::parse("control+super").unwrap().display_tokens(),
            ["◈", "Ctrl"]
        );
    }
}
