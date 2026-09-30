//! What the engine does when a keyword is typed: the decisions in
//! `SnippetService::handleKeywordTrigger` and `handleUndo`
//! (`src/server/src/services/snippet/snippet-service.hpp`), and the Snippets
//! extension's preferences that steer them.
//!
//! The machine's side — reading the focused window, running `{shell}`
//! placeholders, the clipboard — is the engine's; this is what it decides
//! with the answers.

use serde_json::{Map, Value};

use super::wire::{ExpansionMode, InjectExpand, InjectUndo, LayoutInfo};
use crate::snippet_expander::Expansion;
use crate::snippet_store::StoredExpansion;

/// The Snippets provider's id, under which its preferences are kept
/// (`providers.snippets.preferences`).
pub const PROVIDER_ID: &str = "snippets";

/// `SnippetService::DEFAULT_KEY_DELAY_US`.
pub const DEFAULT_KEY_DELAY_US: u32 = 2000;

/// `SnippetService::DEFAULT_PRE_PASTE_DELAY_MS`.
pub const DEFAULT_PRE_PASTE_DELAY_MS: u32 = 0;

/// The Snippets extension's preferences, as `preferenceValuesChanged`
/// applies them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// `enabled`: expand at all.
    pub enabled: bool,
    /// `undo`: Backspace straight after an expansion restores the keyword.
    pub undo: bool,
    /// `prePasteDelay`, in milliseconds, clamped to 0–5000.
    pub pre_paste_delay_ms: u32,
    /// `keyDelay`, in microseconds (the preference is milliseconds, 0–50).
    pub key_delay_us: u32,
    /// `layout`: the XKB layout triggers are read in; `None` for the
    /// system's.
    pub layout: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            undo: true,
            pre_paste_delay_ms: DEFAULT_PRE_PASTE_DELAY_MS,
            key_delay_us: DEFAULT_KEY_DELAY_US,
            layout: None,
        }
    }
}

impl Settings {
    /// Reads `providers.snippets.preferences`.
    ///
    /// The delays are text fields holding numbers, read the way Qt reads
    /// them: `toString(default).toInt()`, so a string that is not a number is
    /// 0, and a value that is not a string at all is the default.
    #[must_use]
    pub fn from_preferences(preferences: Option<&Map<String, Value>>) -> Self {
        let defaults = Self::default();
        let Some(preferences) = preferences else {
            return defaults;
        };
        let flag = |name: &str, default: bool| {
            preferences
                .get(name)
                .and_then(Value::as_bool)
                .unwrap_or(default)
        };
        let number = |name: &str, default: u32| -> i64 {
            match preferences.get(name).and_then(Value::as_str) {
                Some(text) => text.trim().parse::<i64>().unwrap_or(0),
                None => i64::from(default),
            }
        };
        let clamp = |value: i64, max: i64| u32::try_from(value.clamp(0, max)).unwrap_or(0);
        let layout = preferences
            .get("layout")
            .and_then(Value::as_str)
            .filter(|layout| !layout.is_empty())
            .map(str::to_owned);
        Self {
            enabled: flag("enabled", defaults.enabled),
            undo: flag("undo", defaults.undo),
            pre_paste_delay_ms: clamp(number("prePasteDelay", DEFAULT_PRE_PASTE_DELAY_MS), 5000),
            key_delay_us: clamp(number("keyDelay", DEFAULT_KEY_DELAY_US / 1000), 50) * 1000,
            layout,
        }
    }

    /// The `setKeymap` call for [`Self::layout`], when one is set.
    #[must_use]
    pub fn keymap(&self) -> Option<LayoutInfo> {
        self.layout.as_ref().map(|layout| LayoutInfo {
            layout: layout.clone(),
            ..LayoutInfo::default()
        })
    }
}

/// The mode a stored expansion registers with.
#[must_use]
pub fn mode(expansion: &StoredExpansion) -> ExpansionMode {
    if expansion.word {
        ExpansionMode::Word
    } else {
        ExpansionMode::Keydown
    }
}

/// The focused application, as far as the engine could tell.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontmost {
    /// Its desktop id (`org.gnome.Ptyxis.desktop`), when the window was
    /// recognised.
    pub app_id: Option<String>,
    /// Whether it is a terminal, which pastes with Ctrl+Shift+V.
    pub terminal: bool,
}

/// Whether a keyword limited to `apps` may expand in `frontmost`: always
/// when the list is empty, otherwise only in a listed application — and not
/// at all when the focused one is unknown.
#[must_use]
pub fn allowed(apps: &[String], frontmost: &Frontmost) -> bool {
    apps.is_empty()
        || frontmost
            .app_id
            .as_ref()
            .is_some_and(|id| apps.iter().any(|app| app == id))
}

/// What an expansion remembers so the next Backspace can undo it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoRecord {
    /// The keyword that expanded.
    pub trigger: String,
    /// How many characters the expansion typed.
    pub expanded_chars: usize,
}

/// One expansion, ready to carry out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// What goes on the clipboard and is pasted.
    pub text: String,
    /// The `injectExpand` call.
    pub request: InjectExpand,
    /// What to remember for undo: nothing when the snippet placed the caret,
    /// since walking it back left would make the count wrong.
    pub undo: Option<UndoRecord>,
}

/// Plans the expansion of `keyword` into `expansion`.
///
/// The keyword and, for a word snippet, the separator typed after it are
/// erased; the separator comes back as a space at the end of the text,
/// whichever separator it was, as in the C++.
#[must_use]
pub fn plan(
    keyword: &str,
    word: bool,
    expansion: &Expansion,
    terminal: bool,
    settings: &Settings,
) -> Plan {
    let mut text = expansion.to_text();
    if word {
        text.push(' ');
    }
    let length = text.chars().count();
    let cursor_left_moves = expansion
        .cursor_position
        .map_or(0, |position| length.saturating_sub(position));
    let chars_to_delete = keyword.len() + usize::from(word);
    let undo = expansion.cursor_position.is_none().then(|| UndoRecord {
        trigger: keyword.to_owned(),
        expanded_chars: length,
    });
    Plan {
        request: InjectExpand {
            chars_to_delete: u32::try_from(chars_to_delete).unwrap_or(u32::MAX),
            pre_paste_delay_us: settings.pre_paste_delay_ms.saturating_mul(1000),
            terminal,
            cursor_left_moves: u32::try_from(cursor_left_moves).unwrap_or(u32::MAX),
        },
        text,
        undo,
    }
}

/// The `injectUndo` call for a Backspace after `trigger` expanded, or
/// nothing when undo is off, nothing was recorded, or the record is for
/// another keyword.
///
/// One character fewer than the expansion is erased: the Backspace that
/// asked for the undo already took one.
#[must_use]
pub fn undo(record: Option<&UndoRecord>, trigger: &str, settings: &Settings) -> Option<InjectUndo> {
    let record = record.filter(|record| settings.undo && record.trigger == trigger)?;
    Some(InjectUndo {
        backspace_count: u32::try_from(record.expanded_chars.saturating_sub(1)).unwrap_or(u32::MAX),
        trigger_text: record.trigger.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippet_expander::ResultPart;
    use serde_json::json;

    fn prefs(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    fn expansion(text: &str, cursor: Option<usize>) -> Expansion {
        Expansion {
            parts: vec![ResultPart::literal(text)],
            cursor_position: cursor,
        }
    }

    #[test]
    fn preferences_default_as_the_extension_declares_them() {
        assert_eq!(Settings::from_preferences(None), Settings::default());
        let settings = Settings::default();
        assert!(settings.enabled && settings.undo);
        assert_eq!(settings.key_delay_us, 2000);
        assert_eq!(settings.pre_paste_delay_ms, 0);
    }

    #[test]
    fn preferences_are_read_and_clamped() {
        let settings = Settings::from_preferences(Some(&prefs(json!({
            "enabled": false,
            "undo": false,
            "prePasteDelay": "9000",
            "keyDelay": "7",
            "layout": "fr",
        }))));
        assert_eq!(
            settings,
            Settings {
                enabled: false,
                undo: false,
                pre_paste_delay_ms: 5000,
                key_delay_us: 7000,
                layout: Some("fr".into()),
            }
        );
        assert_eq!(settings.keymap().unwrap().layout, "fr");
    }

    #[test]
    fn a_delay_that_is_not_a_number_is_zero_and_one_that_is_not_text_is_the_default() {
        let settings = Settings::from_preferences(Some(&prefs(json!({
            "prePasteDelay": "soon",
            "keyDelay": 12,
            "layout": "",
        }))));
        assert_eq!(settings.pre_paste_delay_ms, 0);
        assert_eq!(settings.key_delay_us, DEFAULT_KEY_DELAY_US);
        assert_eq!(settings.layout, None, "empty is the system layout");
    }

    #[test]
    fn a_keyword_limited_to_apps_expands_only_in_them() {
        let apps = vec!["org.gnome.Ptyxis.desktop".to_owned()];
        let ptyxis = Frontmost {
            app_id: Some("org.gnome.Ptyxis.desktop".into()),
            terminal: true,
        };
        assert!(allowed(&apps, &ptyxis));
        assert!(!allowed(&apps, &Frontmost::default()), "unknown app");
        assert!(allowed(&[], &Frontmost::default()), "unlimited");
    }

    #[test]
    fn a_keydown_plan_erases_the_keyword_and_remembers_the_undo() {
        let plan = plan(
            ";sig",
            false,
            &expansion("Best, Zoë", None),
            false,
            &Settings::default(),
        );
        assert_eq!(plan.text, "Best, Zoë");
        assert_eq!(plan.request.chars_to_delete, 4);
        assert_eq!(plan.request.cursor_left_moves, 0);
        assert_eq!(
            plan.undo,
            Some(UndoRecord {
                trigger: ";sig".into(),
                expanded_chars: 9
            })
        );
    }

    #[test]
    fn a_word_plan_erases_the_separator_and_ends_in_a_space() {
        let settings = Settings {
            pre_paste_delay_ms: 20,
            ..Settings::default()
        };
        let plan = plan(";sig", true, &expansion("Best", None), true, &settings);
        assert_eq!(plan.text, "Best ");
        assert_eq!(plan.request.chars_to_delete, 5);
        assert_eq!(plan.request.pre_paste_delay_us, 20_000);
        assert!(plan.request.terminal);
    }

    #[test]
    fn a_cursor_placeholder_walks_back_and_forgoes_undo() {
        let plan = plan(
            "div",
            false,
            &expansion("<div></div>", Some(5)),
            false,
            &Settings::default(),
        );
        assert_eq!(plan.request.cursor_left_moves, 6);
        assert_eq!(plan.undo, None);
    }

    #[test]
    fn undo_erases_all_but_one_character_and_types_the_keyword() {
        let record = UndoRecord {
            trigger: ";sig".into(),
            expanded_chars: 9,
        };
        let settings = Settings::default();
        assert_eq!(
            undo(Some(&record), ";sig", &settings),
            Some(InjectUndo {
                backspace_count: 8,
                trigger_text: ";sig".into()
            })
        );
        assert_eq!(undo(Some(&record), ";other", &settings), None);
        assert_eq!(undo(None, ";sig", &settings), None);
        let off = Settings {
            undo: false,
            ..Settings::default()
        };
        assert_eq!(undo(Some(&record), ";sig", &off), None);
    }
}
