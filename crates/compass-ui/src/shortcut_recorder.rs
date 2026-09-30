//! The action panel's shortcut recorder (`ShortcutRecorderPanelView` and
//! `ShortcutRecorderPanel.qml`): the panel's list gives way to a capture area
//! that records the next key combination for a root item.
//!
//! The chord tracking, the stored spelling and the validation are
//! [`compass_core::key_combo`]'s; this is the part that knows Iced's key
//! events and what the panel shows.

use compass_core::global_shortcuts;
use compass_core::key_combo::{Key, KeyCombo, Modifier, Modifiers, Recorded, Recorder};

/// The launcher hotkey's setting, which the settings view records.
pub const LAUNCHER_HOTKEY_SETTING: &str = "launcher.hotkey";

/// The status line while nothing has been captured yet.
pub const RECORDING: &str = "Recording...";

/// The status line once a combination is accepted.
pub const UPDATED: &str = "Keybind updated";

/// The status line while the desktop is asked whether it would bind the
/// combination.
pub const CHECKING: &str = "Checking...";

/// The hint under the capture area while the item has a shortcut.
pub const REMOVE_HINT: &str = "Press Backspace to remove the current shortcut";

/// What the host does after a key event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Keep recording.
    Recording,
    /// Keep this shortcut for the item (empty to clear it) and close the
    /// panel, as `accept`/`clear` then `requestClose` do.
    Save(String),
    /// Ask the engine whether the desktop would bind this combination
    /// (`probeBind`), and hand its answer to [`ShortcutRecorder::probed`].
    Probe(String),
    /// Back to the action list (`navigateBack`).
    Back,
}

/// The recorder's state while the panel shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutRecorder {
    /// The root item's id.
    pub id: String,
    /// The item's title, at the top.
    pub title: String,
    /// The shortcut it has, as stored.
    pub current: Option<String>,
    /// What the badge shows: the chord being pressed, or the current one.
    pub tokens: Vec<String>,
    /// The line under the badge.
    pub status: String,
    /// Whether that line is an error.
    pub error: bool,
    recorder: Recorder,
    /// The combination the engine is being asked about.
    probing: Option<String>,
}

impl ShortcutRecorder {
    /// Starts recording for the item `id`, showing the shortcut it has.
    #[must_use]
    pub fn new(id: String, title: String, current: Option<String>) -> Self {
        let current = current.filter(|shortcut| !shortcut.is_empty());
        let tokens = current
            .as_deref()
            .and_then(KeyCombo::parse)
            .map(|combo| combo.display_tokens())
            .unwrap_or_default();
        Self {
            id,
            title,
            current,
            tokens,
            status: RECORDING.to_owned(),
            error: false,
            recorder: Recorder::default(),
            probing: None,
        }
    }

    /// Takes one key event; `launcher_hotkey` (as stored) and `bound`
    /// (every root item's id, title and stored shortcut) are for the
    /// conflict check, [`compass_core::global_shortcuts::validate`].
    pub fn key<'a>(
        &mut self,
        event: &iced::keyboard::Event,
        launcher_hotkey: Option<&str>,
        bound: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>,
    ) -> Outcome {
        use iced::keyboard::Event;
        let recorded = match event {
            Event::KeyPressed {
                key,
                modifiers,
                repeat: false,
                ..
            } => {
                let Some(key) = recorder_key(key) else {
                    return Outcome::Recording;
                };
                let modifiers = combo_modifiers(*modifiers);
                if let Key::Modifier(modifier) = key {
                    // What is held so far, shown as it is pressed.
                    self.tokens =
                        KeyCombo::new(key.clone(), without(modifiers, modifier)).display_tokens();
                    self.status = RECORDING.to_owned();
                    self.error = false;
                }
                self.recorder.press(key, modifiers)
            }
            Event::KeyReleased { key, .. } => match recorder_key(key) {
                Some(key) => self.recorder.release(&key),
                None => Recorded::Pending,
            },
            _ => Recorded::Pending,
        };
        match recorded {
            Recorded::Pending => Outcome::Recording,
            Recorded::Dismissed(key) => {
                if key == Key::Named("backspace".to_owned()) && self.current.is_some() {
                    Outcome::Save(String::new())
                } else {
                    Outcome::Back
                }
            }
            Recorded::Captured(combo) => {
                self.tokens = combo.display_tokens();
                // The settings view records the launcher hotkey under its
                // setting's key; the conflict check knows it by its id.
                let exclude = if self.id == LAUNCHER_HOTKEY_SETTING {
                    global_shortcuts::LAUNCHER_ID
                } else {
                    self.id.as_str()
                };
                match global_shortcuts::validate(&combo, exclude, launcher_hotkey, bound) {
                    Ok(()) => {
                        let trigger = combo.to_config_string();
                        self.status = CHECKING.to_owned();
                        self.error = false;
                        self.probing = Some(trigger.clone());
                        Outcome::Probe(trigger)
                    }
                    Err(reason) => {
                        self.status = reason;
                        self.error = true;
                        Outcome::Recording
                    }
                }
            }
        }
    }

    /// The engine's answer about `trigger`: kept when the desktop would bind
    /// it, else its refusal shown in the recorder, which keeps recording, as
    /// `validate` returns `probeBind`'s reason. An answer about a combination
    /// no longer being asked about changes nothing.
    pub fn probed(&mut self, trigger: &str, refusal: Option<String>) -> Outcome {
        if self.probing.as_deref() != Some(trigger) {
            return Outcome::Recording;
        }
        self.probing = None;
        match refusal {
            None => {
                self.status = UPDATED.to_owned();
                self.error = false;
                Outcome::Save(trigger.to_owned())
            }
            Some(reason) => {
                self.status = reason;
                self.error = true;
                Outcome::Recording
            }
        }
    }
}

fn without(modifiers: Modifiers, modifier: Modifier) -> Modifiers {
    Modifiers {
        super_key: modifiers.super_key && modifier != Modifier::Super,
        control: modifiers.control && modifier != Modifier::Control,
        alt: modifiers.alt && modifier != Modifier::Alt,
        shift: modifiers.shift && modifier != Modifier::Shift,
    }
}

/// Iced's modifiers as a combination's.
#[must_use]
pub fn combo_modifiers(modifiers: iced::keyboard::Modifiers) -> Modifiers {
    Modifiers {
        super_key: modifiers.logo(),
        control: modifiers.control(),
        alt: modifiers.alt(),
        shift: modifiers.shift(),
    }
}

/// An Iced key as the C++ names it; `None` for one it has no name for.
#[must_use]
pub fn recorder_key(key: &iced::keyboard::Key) -> Option<Key> {
    use iced::keyboard::{Key as IcedKey, key::Named};
    match key.as_ref() {
        IcedKey::Character(text) => Key::from_name(text),
        IcedKey::Named(named) => Some(match named {
            Named::Control => Key::Modifier(Modifier::Control),
            Named::Shift => Key::Modifier(Modifier::Shift),
            Named::Alt | Named::AltGraph => Key::Modifier(Modifier::Alt),
            Named::Super | Named::Meta | Named::Hyper => Key::Modifier(Modifier::Super),
            Named::Space => Key::Named("space".to_owned()),
            other => {
                let name = match other {
                    Named::Enter => "return",
                    Named::Tab => "tab",
                    Named::Backspace => "backspace",
                    Named::Delete => "delete",
                    Named::Escape => "escape",
                    Named::ArrowUp => "arrowup",
                    Named::ArrowDown => "arrowdown",
                    Named::ArrowLeft => "arrowleft",
                    Named::ArrowRight => "arrowright",
                    Named::PageUp => "pageup",
                    Named::PageDown => "pagedown",
                    Named::Home => "home",
                    Named::End => "end",
                    _ => return function_key(other),
                };
                Key::Named(name.to_owned())
            }
        }),
        IcedKey::Unidentified => None,
    }
}

fn function_key(named: iced::keyboard::key::Named) -> Option<Key> {
    use iced::keyboard::key::Named;
    const KEYS: [Named; 24] = [
        Named::F1,
        Named::F2,
        Named::F3,
        Named::F4,
        Named::F5,
        Named::F6,
        Named::F7,
        Named::F8,
        Named::F9,
        Named::F10,
        Named::F11,
        Named::F12,
        Named::F13,
        Named::F14,
        Named::F15,
        Named::F16,
        Named::F17,
        Named::F18,
        Named::F19,
        Named::F20,
        Named::F21,
        Named::F22,
        Named::F23,
        Named::F24,
    ];
    let at = KEYS.iter().position(|key| *key == named)?;
    Some(Key::Named(format!("f{}", at + 1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::keyboard::{Event, Key as IcedKey, Location, Modifiers as IcedModifiers, key::Named};

    fn press(key: IcedKey, modifiers: IcedModifiers) -> Event {
        Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: iced::keyboard::key::Physical::Unidentified(
                iced::keyboard::key::NativeCode::Unidentified,
            ),
            location: Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        }
    }

    fn release(key: IcedKey, modifiers: IcedModifiers) -> Event {
        Event::KeyReleased {
            key: key.clone(),
            modified_key: key,
            physical_key: iced::keyboard::key::Physical::Unidentified(
                iced::keyboard::key::NativeCode::Unidentified,
            ),
            location: Location::Standard,
            modifiers,
        }
    }

    const NONE: [(&str, &str, &str); 0] = [];

    #[test]
    fn a_chord_is_recorded_in_the_cpps_spelling() {
        let mut recorder = ShortcutRecorder::new("apps:firefox".into(), "Firefox".into(), None);
        assert_eq!(recorder.status, RECORDING);
        let ctrl_shift = IcedModifiers::CTRL | IcedModifiers::SHIFT;
        assert_eq!(
            recorder.key(
                &press(IcedKey::Named(Named::Control), IcedModifiers::CTRL),
                None,
                NONE
            ),
            Outcome::Recording
        );
        assert_eq!(recorder.tokens, ["Ctrl"]);
        assert_eq!(
            recorder.key(
                &press(IcedKey::Character("k".into()), ctrl_shift),
                None,
                NONE
            ),
            Outcome::Probe("control+shift+K".into())
        );
        assert_eq!(recorder.tokens, ["Ctrl", "Shift", "K"]);
        assert_eq!(recorder.status, CHECKING);
        assert_eq!(
            recorder.probed("control+shift+K", None),
            Outcome::Save("control+shift+K".into())
        );
        assert_eq!(recorder.status, UPDATED);
    }

    #[test]
    fn a_combination_the_desktop_refuses_is_shown_and_recording_goes_on() {
        let mut recorder = ShortcutRecorder::new("apps:firefox".into(), "Firefox".into(), None);
        let captured = recorder.key(
            &press(IcedKey::Character("q".into()), IcedModifiers::LOGO),
            None,
            NONE,
        );
        assert_eq!(captured, Outcome::Probe("super+Q".into()));
        assert_eq!(
            recorder.probed(
                "super+Q",
                Some("The compositor has already bound it".into())
            ),
            Outcome::Recording
        );
        assert_eq!(recorder.status, "The compositor has already bound it");
        assert!(recorder.error);

        // An answer about an earlier combination is not this one's.
        let _ = recorder.key(
            &press(IcedKey::Character("w".into()), IcedModifiers::LOGO),
            None,
            NONE,
        );
        assert_eq!(recorder.probed("super+Q", None), Outcome::Recording);
        assert_eq!(recorder.status, CHECKING);
        assert_eq!(
            recorder.probed("super+W", None),
            Outcome::Save("super+W".into())
        );
    }

    #[test]
    fn a_bare_key_needs_a_modifier_and_a_taken_one_says_by_whom() {
        let mut recorder = ShortcutRecorder::new("apps:firefox".into(), "Firefox".into(), None);
        assert_eq!(
            recorder.key(
                &press(IcedKey::Character("k".into()), IcedModifiers::empty()),
                None,
                NONE
            ),
            Outcome::Recording
        );
        assert_eq!(recorder.status, compass_core::key_combo::MODIFIER_REQUIRED);
        assert!(recorder.error);
        let bound = [("apps:files", "Files", "super+F")];
        assert_eq!(
            recorder.key(
                &press(IcedKey::Character("f".into()), IcedModifiers::LOGO),
                None,
                bound
            ),
            Outcome::Recording
        );
        assert_eq!(recorder.status, "Already bound to \"Files\"");
        assert_eq!(
            recorder.key(
                &press(IcedKey::Named(Named::F5), IcedModifiers::empty()),
                None,
                bound
            ),
            Outcome::Probe("F5".into())
        );
    }

    #[test]
    fn super_pressed_and_released_alone_is_a_shortcut() {
        let mut recorder = ShortcutRecorder::new("apps:firefox".into(), "Firefox".into(), None);
        let _ = recorder.key(
            &press(IcedKey::Named(Named::Super), IcedModifiers::LOGO),
            None,
            NONE,
        );
        assert_eq!(
            recorder.key(
                &release(IcedKey::Named(Named::Super), IcedModifiers::empty()),
                None,
                NONE
            ),
            Outcome::Probe("SUPER".into())
        );
    }

    #[test]
    fn escape_goes_back_and_backspace_clears_what_there_is() {
        let mut recorder = ShortcutRecorder::new(
            "apps:firefox".into(),
            "Firefox".into(),
            Some("control+shift+K".into()),
        );
        assert_eq!(recorder.tokens, ["Ctrl", "Shift", "K"]);
        assert_eq!(
            recorder.key(
                &press(IcedKey::Named(Named::Escape), IcedModifiers::empty()),
                None,
                NONE
            ),
            Outcome::Back
        );
        assert_eq!(
            recorder.key(
                &press(IcedKey::Named(Named::Backspace), IcedModifiers::empty()),
                None,
                NONE
            ),
            Outcome::Save(String::new())
        );
        let mut empty = ShortcutRecorder::new("apps:firefox".into(), "Firefox".into(), None);
        assert_eq!(
            empty.key(
                &press(IcedKey::Named(Named::Backspace), IcedModifiers::empty()),
                None,
                NONE
            ),
            Outcome::Back
        );
    }

    #[test]
    fn the_launchers_own_keys_and_its_hotkey_are_taken() {
        let mut recorder = ShortcutRecorder::new("apps:firefox".into(), "Firefox".into(), None);
        assert_eq!(
            recorder.key(
                &press(IcedKey::Character("b".into()), IcedModifiers::CTRL),
                Some("super+SPACE"),
                NONE
            ),
            Outcome::Recording
        );
        assert_eq!(recorder.status, "Already bound to \"Toggle action panel\"");
        assert_eq!(
            recorder.key(
                &press(IcedKey::Named(Named::Space), IcedModifiers::LOGO),
                Some("super+SPACE"),
                NONE
            ),
            Outcome::Recording
        );
        assert_eq!(recorder.status, "Already bound to \"the launcher hotkey\"");
        assert!(recorder.error);
    }

    #[test]
    fn recording_the_launcher_hotkey_does_not_conflict_with_itself() {
        let mut recorder = ShortcutRecorder::new(
            LAUNCHER_HOTKEY_SETTING.into(),
            "Launcher hotkey".into(),
            Some("super+SPACE".into()),
        );
        assert_eq!(
            recorder.key(
                &press(IcedKey::Named(Named::Space), IcedModifiers::LOGO),
                Some("super+SPACE"),
                [("apps:files", "Files", "super+F")]
            ),
            Outcome::Probe("super+SPACE".into())
        );
        assert_eq!(
            recorder.key(
                &press(IcedKey::Character("f".into()), IcedModifiers::LOGO),
                Some("super+SPACE"),
                [("apps:files", "Files", "super+F")]
            ),
            Outcome::Recording,
            "an item's shortcut is still taken"
        );
    }
}
