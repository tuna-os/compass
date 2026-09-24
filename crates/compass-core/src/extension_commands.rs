//! Installed extensions' commands, as root-search entries.
//!
//! The manifest registry ([`crate::manifest::registry`]) says what is
//! installed; this turns each command into a [`RootItem`] under its
//! extension's provider, so root search finds `Search Repositories` the way it
//! finds an application. Running one is the engine's job, not this module's.
//!
//! # Ids
//!
//! The C++ `ExtensionCommand::uniqueId` is `EntrypointId{"@<author>/<id>",
//! <command name>}`, which [`entrypoint_id`] renders as
//! `@<author>/<id>:<name>`. Kept, so frecency and aliases recorded by either
//! engine name the same command. The provider holds no colon, so
//! [`crate::root_items::split_entrypoint_id`] splits it back.

use std::path::PathBuf;

use crate::manifest::{CommandArgument, CommandMode, ExtensionManifest, Preference, Provenance};
use crate::root_items::{RootItem, RootItemMeta, entrypoint_id};

/// One command from an installed extension.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionCommand {
    /// `@<author>/<extension id>:<command name>`.
    pub id: String,
    /// `@<author>/<extension id>`, the root item's provider.
    pub provider_id: String,
    /// The extension's directory name.
    pub extension_id: String,
    /// The extension's directory.
    pub extension_dir: PathBuf,
    /// The command's name within the extension.
    pub name: String,
    /// What the row says.
    pub title: String,
    /// The extension's title, as the row's subtitle.
    pub extension_title: String,
    /// Extra search terms.
    pub keywords: Vec<String>,
    /// Whether it draws a view or runs and exits.
    pub mode: CommandMode,
    /// The file the worker loads.
    pub entrypoint: PathBuf,
    /// Whether the manifest switches it off by default.
    pub default_disabled: bool,
    /// The manifest's `name`, which the runtime calls the extension name.
    pub extension_name: String,
    /// The manifest's `author`.
    pub author: String,
    /// Whether it came from the Raycast store (`isRaycast`).
    pub is_raycast: bool,
    /// The extension's preferences, then the command's own.
    pub preferences: Vec<Preference>,
    /// The arguments it is launched with, in the manifest's order.
    pub arguments: Vec<CommandArgument>,
}

impl ExtensionCommand {
    /// Every command of every manifest, in the registry's precedence order.
    #[must_use]
    pub fn from_manifests(manifests: &[ExtensionManifest]) -> Vec<Self> {
        manifests
            .iter()
            .flat_map(|manifest| {
                let provider_id = format!("@{}/{}", manifest.author, manifest.id);
                manifest.commands.iter().map(move |command| Self {
                    id: entrypoint_id(&provider_id, &command.name),
                    provider_id: provider_id.clone(),
                    extension_id: manifest.id.clone(),
                    extension_dir: manifest.path.clone(),
                    name: command.name.clone(),
                    title: command.title.clone(),
                    extension_title: manifest.title.clone(),
                    keywords: command.keywords.clone(),
                    mode: command.mode,
                    entrypoint: command.entrypoint.clone(),
                    default_disabled: command.default_disabled,
                    extension_name: manifest.name.clone(),
                    author: manifest.author.clone(),
                    is_raycast: command.provenance == Provenance::Raycast,
                    preferences: manifest
                        .preferences
                        .iter()
                        .chain(&command.preferences)
                        .cloned()
                        .collect(),
                    arguments: command.arguments.clone(),
                })
            })
            .collect()
    }

    /// The preference values a launch passes: what the user stored, else each
    /// preference's default. A value stored for a name the manifest no longer
    /// declares is dropped.
    ///
    /// # Errors
    ///
    /// The required preferences with neither, for the launcher to ask for.
    pub fn preferences_with(
        &self,
        stored: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<serde_json::Value, Vec<&Preference>> {
        let set = |preference: &Preference| {
            stored
                .get(&preference.name)
                .filter(|value| !value.is_null() && value.as_str() != Some(""))
                .or(preference.default.as_ref())
                .cloned()
        };
        let missing: Vec<&Preference> = self
            .preferences
            .iter()
            .filter(|preference| preference.required && set(preference).is_none())
            .collect();
        if !missing.is_empty() {
            return Err(missing);
        }
        Ok(serde_json::Value::Object(
            self.preferences
                .iter()
                .filter_map(|preference| {
                    set(preference).map(|value| (preference.name.clone(), value))
                })
                .collect(),
        ))
    }

    /// The argument values a launch passes, from what the launcher gave:
    /// each declared argument's non-empty value. Anything not declared is
    /// dropped. `Ok` for a command without arguments however it was called.
    ///
    /// # Errors
    ///
    /// `None` when the launcher gave nothing and the command declares
    /// arguments, so the launcher should ask for them; otherwise the
    /// required arguments left empty.
    pub fn arguments_with(
        &self,
        given: Option<&serde_json::Map<String, serde_json::Value>>,
    ) -> Result<serde_json::Value, Option<Vec<&CommandArgument>>> {
        if self.arguments.is_empty() {
            return Ok(serde_json::Value::Object(serde_json::Map::new()));
        }
        let Some(given) = given else {
            return Err(None);
        };
        let value = |argument: &CommandArgument| {
            given
                .get(&argument.name)
                .and_then(serde_json::Value::as_str)
                .filter(|text| !text.is_empty())
                .map(|text| serde_json::Value::String(text.to_owned()))
        };
        let missing: Vec<&CommandArgument> = self
            .arguments
            .iter()
            .filter(|argument| argument.required && value(argument).is_none())
            .collect();
        if !missing.is_empty() {
            return Err(Some(missing));
        }
        Ok(serde_json::Value::Object(
            self.arguments
                .iter()
                .filter_map(|argument| value(argument).map(|v| (argument.name.clone(), v)))
                .collect(),
        ))
    }

    /// The preference values a launch passes when the user has set none:
    /// each preference's default. Compass has no preference editor yet, so a
    /// required preference without a default cannot be satisfied, and those
    /// come back as the error, by title, for the caller to name.
    ///
    /// # Errors
    ///
    /// The titles of required preferences that have no default.
    pub fn default_preferences(&self) -> Result<serde_json::Value, Vec<String>> {
        let missing: Vec<String> = self
            .preferences
            .iter()
            .filter(|preference| preference.required && preference.default.is_none())
            .map(|preference| preference.title.clone())
            .collect();
        if !missing.is_empty() {
            return Err(missing);
        }
        Ok(serde_json::Value::Object(
            self.preferences
                .iter()
                .filter_map(|preference| {
                    preference
                        .default
                        .clone()
                        .map(|value| (preference.name.clone(), value))
                })
                .collect(),
        ))
    }

    /// The root-search entry: titled by the command, subtitled by its
    /// extension, and hidden while the manifest has it disabled.
    #[must_use]
    pub fn root_item(&self) -> RootItem {
        RootItem {
            id: self.id.clone(),
            title: self.title.clone(),
            unlocalized_title: None,
            subtitle: self.extension_title.clone(),
            keywords: self.keywords.clone(),
            meta: RootItemMeta {
                provider_id: self.provider_id.clone(),
                enabled: !self.default_disabled,
                ..RootItemMeta::default()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(arguments: &str) -> ExtensionCommand {
        let json: serde_json::Value = serde_json::from_str(&format!(
            r#"{{"name": "x", "author": "a", "commands": [
                {{"name": "c", "mode": "view", "arguments": {arguments}}}
            ]}}"#
        ))
        .expect("json");
        let manifest = ExtensionManifest::from_json(&json, std::path::Path::new("/ext/x"));
        ExtensionCommand::from_manifests(&[manifest])
            .pop()
            .expect("a command")
    }

    #[test]
    fn arguments_are_asked_for_until_given_and_required_ones_must_be_filled() {
        let none = command("[]");
        assert_eq!(none.arguments_with(None), Ok(serde_json::json!({})));

        let search = command(
            r#"[{"name": "query", "type": "text", "placeholder": "Query", "required": true},
                {"name": "sort", "type": "dropdown", "data": [{"title": "Stars", "value": "stars"}]}]"#,
        );
        assert_eq!(search.arguments_with(None), Err(None), "ask for them");

        let blank = serde_json::json!({"query": "", "sort": "stars"});
        let missing = search
            .arguments_with(blank.as_object())
            .expect_err("query is required");
        assert_eq!(
            missing.map(|m| m.iter().map(|a| a.name.clone()).collect::<Vec<_>>()),
            Some(vec!["query".to_owned()])
        );

        let given = serde_json::json!({"query": "compass", "sort": "", "extra": "x"});
        assert_eq!(
            search.arguments_with(given.as_object()),
            Ok(serde_json::json!({"query": "compass"})),
            "an empty optional one and an undeclared one are left out"
        );
    }
}
