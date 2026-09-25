//! The form an extension command's preferences are set in.
//!
//! The engine answers a run with the preferences a command reads when a
//! required one has no value; this is the launcher's side of that: what the
//! person has entered, whether it is enough to run, and the values to keep.

use crate::backend::{PreferenceInput, PreferenceInputKind};

/// What one field holds now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldValue {
    /// A line of text (or a secret one).
    Text(String),
    /// A tick box.
    Checked(bool),
    /// A dropdown's chosen value, if one is.
    Choice(Option<String>),
    /// A field the form cannot edit; its stored value, untouched.
    Kept(Option<serde_json::Value>),
}

/// What the form's values are for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Kept for every run.
    Preferences,
    /// This one run's arguments.
    Arguments,
    /// A command's preferences, opened by the extension
    /// (`openCommandPreferences`): kept, and the command not run.
    CommandPreferences,
    /// A shortcut's arguments, to open it with; the form's `command_id` is
    /// the shortcut's id.
    ShortcutArguments,
    /// Creating, editing or duplicating a shortcut; `command_id` is the
    /// shortcut edited or duplicated, empty when creating.
    ShortcutForm {
        /// Why the form is open.
        mode: compass_core::shortcut_form::Mode,
        /// Whether saving goes back to Manage Shortcuts rather than the root.
        from_manage: bool,
    },
    /// A snippet's arguments, to copy or paste it with; `command_id` is the
    /// snippet's id.
    SnippetArguments {
        /// Paste rather than copy.
        paste: bool,
    },
    /// Creating, editing or duplicating a snippet; `command_id` is the
    /// snippet edited or duplicated, empty when creating.
    SnippetForm {
        /// Why the form is open.
        mode: compass_core::shortcut_form::Mode,
        /// Whether saving goes back to Manage Snippets rather than the root.
        from_manage: bool,
    },
    /// A script command's arguments, or its confirmation; `command_id` is
    /// the script's id.
    ScriptArguments,
    /// The Create Extension form.
    CreateExtension,
    /// A media command's optional argument (the player, or the volume
    /// step); `command_id` is the command's entrypoint.
    MediaArguments,
    /// The emoji picker's keywords for one glyph; `command_id` is the glyph.
    GlyphKeywords,
    /// A clipboard history entry's keywords; `command_id` is the entry.
    ClipboardKeywords,
}

impl Purpose {
    /// The line under the form saying what the keys do.
    #[must_use]
    pub fn hint(self) -> &'static str {
        match self {
            Self::Preferences | Self::Arguments => "Enter: save and run    Esc: back",
            Self::CommandPreferences | Self::GlyphKeywords | Self::ClipboardKeywords => {
                "Enter: save    Esc: back"
            }
            Self::ShortcutArguments => "Enter: open    Esc: back",
            Self::ShortcutForm { .. } => "Enter: save    Esc: back",
            Self::SnippetArguments { paste: false } => "Enter: copy    Esc: back",
            Self::SnippetArguments { paste: true } => "Enter: paste    Esc: back",
            Self::SnippetForm { .. } => "Ctrl+Enter: save    Esc: back",
            Self::ScriptArguments | Self::MediaArguments => "Enter: run    Esc: cancel",
            Self::CreateExtension => "Enter: create extension    Esc: back",
        }
    }
}

/// The form.
#[derive(Debug, Clone)]
pub struct PreferencesPage {
    /// What submitting it does.
    pub purpose: Purpose,
    /// The command to run once it is saved.
    pub command_id: String,
    /// The command's title.
    pub title: String,
    /// The preferences, in the manifest's order.
    pub fields: Vec<PreferenceInput>,
    /// What each field holds, by position.
    pub values: Vec<FieldValue>,
    /// Why it cannot be saved yet, or why saving failed.
    pub notice: Option<String>,
    /// Each text area's editor, by field position: it keeps the cursor and
    /// selection, and its text is mirrored into `values`.
    pub editors: std::collections::BTreeMap<usize, iced::widget::text_editor::Content>,
}

impl PreferencesPage {
    /// A form over `fields`, filled with their current values.
    #[must_use]
    pub fn new(
        purpose: Purpose,
        command_id: String,
        title: String,
        fields: Vec<PreferenceInput>,
    ) -> Self {
        let values: Vec<FieldValue> = fields
            .iter()
            .map(|field| match &field.kind {
                PreferenceInputKind::Text
                | PreferenceInputKind::Password
                | PreferenceInputKind::TextArea => FieldValue::Text(match &field.value {
                    Some(serde_json::Value::String(text)) => text.clone(),
                    Some(serde_json::Value::Null) | None => String::new(),
                    Some(other) => other.to_string(),
                }),
                PreferenceInputKind::Checkbox { .. } => FieldValue::Checked(
                    field
                        .value
                        .as_ref()
                        .is_some_and(|value| value.as_bool() == Some(true)),
                ),
                PreferenceInputKind::Dropdown { .. } => FieldValue::Choice(
                    field
                        .value
                        .as_ref()
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                ),
                PreferenceInputKind::Unsupported { .. } => FieldValue::Kept(field.value.clone()),
            })
            .collect();
        let editors = fields
            .iter()
            .zip(&values)
            .enumerate()
            .filter(|(_, (field, _))| field.kind == PreferenceInputKind::TextArea)
            .map(|(index, (_, value))| {
                let text = match value {
                    FieldValue::Text(text) => text.as_str(),
                    _ => "",
                };
                (index, iced::widget::text_editor::Content::with_text(text))
            })
            .collect();
        Self {
            purpose,
            command_id,
            title,
            fields,
            values,
            notice: None,
            editors,
        }
    }

    /// Whether the form has a text area, where Enter is a newline and
    /// submitting takes Ctrl+Enter.
    #[must_use]
    pub fn has_text_area(&self) -> bool {
        !self.editors.is_empty()
    }

    /// An edit in the text area at `index`, applied to its editor and, when
    /// it changed the text, to the field's value.
    pub fn edit_text_area(&mut self, index: usize, action: iced::widget::text_editor::Action) {
        let Some(editor) = self.editors.get_mut(&index) else {
            return;
        };
        let changed = action.is_edit();
        editor.perform(action);
        if changed && let Some(value) = self.values.get_mut(index) {
            *value = FieldValue::Text(editor.text());
            self.notice = None;
        }
    }

    /// The values to keep, or the titles of required fields still empty.
    ///
    /// # Errors
    ///
    /// The titles of required fields with nothing in them.
    pub fn submission(&self) -> Result<serde_json::Map<String, serde_json::Value>, Vec<String>> {
        let mut missing = Vec::new();
        let mut out = serde_json::Map::new();
        for (field, value) in self.fields.iter().zip(&self.values) {
            let json = match value {
                FieldValue::Text(text) if text.trim().is_empty() => None,
                FieldValue::Text(text) => Some(serde_json::Value::String(text.clone())),
                FieldValue::Checked(checked) => Some(serde_json::Value::Bool(*checked)),
                FieldValue::Choice(choice) => choice.clone().map(serde_json::Value::String),
                FieldValue::Kept(kept) => kept.clone(),
            };
            match json {
                Some(json) => {
                    out.insert(field.name.clone(), json);
                }
                None if field.required => missing.push(field.title.clone()),
                // Cleared: kept as empty so the engine drops the stored value.
                None => {
                    out.insert(field.name.clone(), serde_json::Value::String(String::new()));
                }
            }
        }
        if missing.is_empty() {
            Ok(out)
        } else {
            Err(missing)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str, required: bool, kind: PreferenceInputKind) -> PreferenceInput {
        PreferenceInput {
            name: name.into(),
            title: name.to_uppercase(),
            description: String::new(),
            placeholder: String::new(),
            required,
            kind,
            value: None,
        }
    }

    #[test]
    fn required_fields_must_be_filled_and_the_rest_submit_as_typed() {
        let mut page = PreferencesPage::new(
            Purpose::Preferences,
            "@a/b:c".into(),
            "C".into(),
            vec![
                field("token", true, PreferenceInputKind::Password),
                field(
                    "private",
                    false,
                    PreferenceInputKind::Checkbox {
                        label: "Private".into(),
                    },
                ),
                field(
                    "sort",
                    false,
                    PreferenceInputKind::Dropdown {
                        options: vec![("Stars".into(), "stars".into())],
                    },
                ),
            ],
        );
        assert_eq!(page.submission(), Err(vec!["TOKEN".to_owned()]));

        page.values[0] = FieldValue::Text("ghp_x".into());
        page.values[1] = FieldValue::Checked(true);
        page.values[2] = FieldValue::Choice(Some("stars".into()));
        assert_eq!(
            page.submission().map(serde_json::Value::Object),
            Ok(serde_json::json!({"token": "ghp_x", "private": true, "sort": "stars"}))
        );
    }

    #[test]
    fn current_values_fill_the_form() {
        let mut token = field("token", true, PreferenceInputKind::Text);
        token.value = Some(serde_json::json!("saved"));
        let mut private = field(
            "private",
            false,
            PreferenceInputKind::Checkbox {
                label: String::new(),
            },
        );
        private.value = Some(serde_json::json!(true));
        let page = PreferencesPage::new(
            Purpose::Preferences,
            "id".into(),
            "T".into(),
            vec![token, private],
        );
        assert_eq!(
            page.values,
            [FieldValue::Text("saved".into()), FieldValue::Checked(true)]
        );
    }
}
