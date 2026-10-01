//! What is wrong with a `compass.json` that still loads.
//!
//! `compass.json` is written by hand as often as by the settings view, so two
//! mistakes are common: a key spelt wrong (`lancher`, `close_on_focus_los`),
//! which serde keeps as an unknown field and nothing reads, and a value of
//! the wrong type (`"max_results": "fifty"`), which used to make the whole
//! file fail to parse and every setting fall back to its default.
//!
//! Neither is fatal here. [`parse_value`] loads every key it can: a wrongly
//! typed value is set aside, its default used, and the value itself kept in
//! the section's unknown fields, so a later write puts back exactly what the
//! user wrote rather than deleting it. [`unknown_keys`] walks the document
//! against the published schema ([`crate::config::json_schema`], whose closed
//! sections set `additionalProperties: false`) and names each key nothing
//! reads, with a "did you mean" when one is close. Both come back as
//! [`ConfigIssue`]s, which the engine logs, `compass doctor` reports and the
//! settings view shows.
//!
//! A JSON syntax error still fails the load: there is no partial reading of a
//! file that is not JSON.

use std::fmt;

use serde_json::{Map, Value};

use crate::config::Config;

/// One problem in `compass.json` that did not stop it loading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigIssue {
    /// The dotted key the problem is at, e.g. `launcher.max_results`.
    pub key: String,
    /// What is wrong with it.
    pub problem: Problem,
}

/// What is wrong with a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// The value has the wrong type; the default is used instead.
    InvalidValue {
        /// What the parser objected to.
        reason: String,
    },
    /// Nothing reads the key.
    UnknownKey {
        /// A known key it is probably a misspelling of, as a dotted path.
        suggestion: Option<String>,
    },
    /// The key is known but names something that does not exist: a theme, a
    /// preset, a fallback command.
    UnknownName {
        /// What kind of thing it names, as a noun: `theme`, `preset`.
        what: &'static str,
        /// The name as written.
        value: String,
        /// The closest name that does exist.
        suggestion: Option<String>,
    },
}

impl ConfigIssue {
    /// An [`Problem::UnknownName`] issue for `value` at `key`, suggesting the
    /// closest of `known`.
    #[must_use]
    pub fn unknown_name<'a>(
        key: &str,
        what: &'static str,
        value: &str,
        known: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        Self {
            key: key.to_owned(),
            problem: Problem::UnknownName {
                what,
                value: value.to_owned(),
                suggestion: suggest(value, known).map(str::to_owned),
            },
        }
    }

    /// Whether the problem leaves a setting at its default rather than only
    /// an unread key in the file.
    #[must_use]
    pub const fn is_invalid_value(&self) -> bool {
        matches!(self.problem, Problem::InvalidValue { .. })
    }
}

impl fmt::Display for ConfigIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let key = &self.key;
        match &self.problem {
            Problem::InvalidValue { reason } => write!(
                f,
                "{key} has a value of the wrong type ({reason}), so its default is used"
            ),
            Problem::UnknownKey {
                suggestion: Some(suggestion),
            } => write!(
                f,
                "{key} is not a Compass setting and is ignored. Did you mean {suggestion}?"
            ),
            Problem::UnknownKey { suggestion: None } => {
                write!(f, "{key} is not a Compass setting and is ignored")
            }
            Problem::UnknownName {
                what,
                value,
                suggestion: Some(suggestion),
            } => write!(
                f,
                "{key} names the {what} \"{value}\", which does not exist. Did you mean \"{suggestion}\"?"
            ),
            Problem::UnknownName {
                what,
                value,
                suggestion: None,
            } => write!(
                f,
                "{key} names the {what} \"{value}\", which does not exist"
            ),
        }
    }
}

/// The candidate `word` is most likely a misspelling of, if any is close
/// enough to be worth suggesting.
///
/// Case is ignored, and the allowance grows with the word: one edit for a
/// short word, about a third of it for a long one.
#[must_use]
pub fn suggest<'a>(word: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let lowered = word.to_lowercase();
    let allowance = (lowered.chars().count() / 3).max(1);
    candidates
        .into_iter()
        .map(|candidate| {
            let distance = strsim::damerau_levenshtein(&lowered, &candidate.to_lowercase());
            (distance, candidate)
        })
        .filter(|(distance, _)| *distance <= allowance)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, candidate)| candidate)
}

/// Deserialises `document` into a [`Config`], setting aside every value of
/// the wrong type rather than failing on the first.
///
/// Each value set aside is reported as [`Problem::InvalidValue`] and kept in
/// the unknown fields of the section that holds it, so it survives a round
/// trip. `document` must be an object; anything else yields the default
/// configuration and one issue at the root.
#[must_use]
pub fn parse_value(mut document: Value) -> (Config, Vec<ConfigIssue>) {
    let mut set_aside: Vec<(Vec<String>, Value)> = Vec::new();
    let mut issues = Vec::new();
    // Each pass removes one value, so this ends; the bound is a backstop
    // against a deserialiser that reports a path it does not then accept.
    for _ in 0..512 {
        let error = match serde_path_to_error::deserialize::<_, Config>(&document) {
            Ok(mut config) => {
                for (path, value) in set_aside {
                    config.keep_invalid(&path, value);
                }
                return (config, issues);
            }
            Err(error) => error,
        };
        let reason = error.inner().to_string();
        let path = stash_point(error.path());
        let removed = (!path.is_empty())
            .then(|| remove_at(&mut document, &path))
            .flatten();
        let Some(removed) = removed else {
            issues.push(ConfigIssue {
                key: path.join("."),
                problem: Problem::InvalidValue { reason },
            });
            return (Config::default(), issues);
        };
        issues.push(ConfigIssue {
            key: path.join("."),
            problem: Problem::InvalidValue { reason },
        });
        set_aside.push((path, removed));
    }
    (Config::default(), issues)
}

/// The keys of the path serde stopped at, cut back to the deepest one whose
/// section keeps unknown fields: a bad element of a list sets the whole list
/// aside, and a provider that is not an object sets aside `providers`.
fn stash_point(path: &serde_path_to_error::Path) -> Vec<String> {
    let mut keys = Vec::new();
    for segment in path.iter() {
        match segment {
            serde_path_to_error::Segment::Map { key } => keys.push(key.clone()),
            _ => break,
        }
    }
    while let Some((_, parent)) = keys.split_last() {
        if keeps_unknown_fields(parent) {
            return keys;
        }
        keys.pop();
    }
    keys
}

/// Whether the object at `parent` is one of [`Config`]'s sections, which
/// keep the keys they do not know.
pub(crate) fn keeps_unknown_fields(parent: &[String]) -> bool {
    let parent: Vec<&str> = parent.iter().map(String::as_str).collect();
    matches!(
        parent.as_slice(),
        [] | ["launcher"]
            | ["launcher", "appearance" | "clock"]
            | ["extensions" | "input_server" | "tray" | "global_shortcuts"]
            | ["providers", _]
            | ["providers", _, "entrypoints", _]
    )
}

fn remove_at(document: &mut Value, path: &[String]) -> Option<Value> {
    let (last, parents) = path.split_last()?;
    let mut node = document.as_object_mut()?;
    for key in parents {
        node = node.get_mut(key)?.as_object_mut()?;
    }
    node.remove(last)
}

/// Every key in `document` that nothing reads, by the closed sections of the
/// published schema, each with the closest known key as a suggestion.
///
/// A key a Vicinae `settings.json` uses at the top level
/// (`close_on_focus_loss`, `theme`) is pointed at where Compass keeps it.
#[must_use]
pub fn unknown_keys(document: &Value) -> Vec<ConfigIssue> {
    let schema = crate::config::json_schema();
    let defs = schema.get("$defs").cloned().unwrap_or(Value::Null);
    let mut out = Vec::new();
    walk(document, &schema, &defs, &mut Vec::new(), &mut out);
    out
}

/// Follows a `$ref`, or an `anyOf` whose first non-null branch is one.
fn resolve<'a>(node: &'a Value, defs: &'a Value) -> &'a Value {
    if let Some(name) = node
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| reference.rsplit('/').next())
        && let Some(found) = defs.get(name)
    {
        return resolve(found, defs);
    }
    if let Some(branch) = node
        .get("anyOf")
        .and_then(Value::as_array)
        .and_then(|branches| {
            branches
                .iter()
                .find(|branch| branch.get("type").and_then(Value::as_str) != Some("null"))
        })
    {
        return resolve(branch, defs);
    }
    node
}

fn walk(
    value: &Value,
    schema: &Value,
    defs: &Value,
    path: &mut Vec<String>,
    out: &mut Vec<ConfigIssue>,
) {
    let Value::Object(map) = value else {
        return;
    };
    let schema = resolve(schema, defs);
    let empty = Map::new();
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let additional = schema.get("additionalProperties");
    for (key, child) in map {
        path.push(key.clone());
        if let Some(property) = properties.get(key) {
            walk(child, property, defs, path, out);
        } else {
            match additional {
                Some(Value::Bool(false)) => out.push(ConfigIssue {
                    key: path.join("."),
                    problem: Problem::UnknownKey {
                        suggestion: suggestion_for(path, properties),
                    },
                }),
                Some(item @ Value::Object(_)) => walk(child, item, defs, path, out),
                _ => {}
            }
        }
        path.pop();
    }
}

fn suggestion_for(path: &[String], properties: &Map<String, Value>) -> Option<String> {
    let (key, parent) = path.split_last()?;
    if parent.is_empty()
        && let Some(compass) = crate::config_migration::compass_key_for(key)
    {
        return Some(compass.to_owned());
    }
    let known = suggest(key, properties.keys().map(String::as_str))?;
    let mut full: Vec<&str> = parent.iter().map(String::as_str).collect();
    full.push(known);
    Some(full.join("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_typo_is_matched_to_the_key_it_misspells() {
        assert_eq!(
            suggest("lancher", ["launcher", "extensions", "tray"]),
            Some("launcher")
        );
        assert_eq!(
            suggest(
                "close_on_focus_los",
                ["hotkey", "close_on_focus_loss", "clock"]
            ),
            Some("close_on_focus_loss")
        );
        assert_eq!(suggest("zzzz", ["launcher", "tray"]), None);
    }

    #[test]
    fn unknown_keys_are_named_with_a_suggestion() {
        let document = json!({
            "lancher": {"hotkey": "super+space"},
            "launcher": {"close_on_focus_los": true, "appearance": {"theem": "nord"}},
            "providers": {"apps": {"enabeld": false, "preferences": {"anything": 1}}},
            "font": {"normal": {"family": "Inter", "size": 11}},
        });
        let issues = unknown_keys(&document);
        let shown: Vec<String> = issues.iter().map(ToString::to_string).collect();
        assert_eq!(
            shown,
            [
                "lancher is not a Compass setting and is ignored. Did you mean launcher?",
                "launcher.appearance.theem is not a Compass setting and is ignored. Did you mean launcher.appearance.theme?",
                "launcher.close_on_focus_los is not a Compass setting and is ignored. Did you mean launcher.close_on_focus_loss?",
                "providers.apps.enabeld is not a Compass setting and is ignored. Did you mean providers.apps.enabled?",
            ],
            "provider preferences and the font are open"
        );
    }

    #[test]
    fn a_vicinae_key_points_at_where_compass_keeps_it() {
        let issues = unknown_keys(&json!({"close_on_focus_loss": true}));
        assert_eq!(
            issues[0].problem,
            Problem::UnknownKey {
                suggestion: Some("launcher.close_on_focus_loss".to_owned())
            }
        );
    }

    #[test]
    fn one_wrong_value_does_not_cost_the_rest_of_the_file() {
        let (config, issues) = parse_value(json!({
            "launcher": {"max_results": "fifty", "close_on_focus_loss": true,
                         "appearance": {"icons": "yes", "theme": "nord"}},
            "favorites": ["a", 3],
            "tray": {"enabled": false},
        }));
        assert!(config.launcher().close_on_focus_loss());
        assert_eq!(config.launcher().appearance().theme(), "nord");
        assert!(!config.tray().enabled());
        assert_eq!(
            config.launcher().max_results(),
            crate::config::DEFAULT_MAX_RESULTS
        );
        let keys: Vec<&str> = issues.iter().map(|issue| issue.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "favorites",
                "launcher.appearance.icons",
                "launcher.max_results"
            ]
        );
        assert!(issues.iter().all(ConfigIssue::is_invalid_value));

        let written = serde_json::to_value(&config).unwrap();
        assert_eq!(
            written["launcher"]["max_results"], "fifty",
            "kept as written"
        );
        assert_eq!(written["favorites"], json!(["a", 3]));
    }
}
