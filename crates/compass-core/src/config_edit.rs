//! Writing `compass.json` back without reformatting it.
//!
//! The settings view, `compass theme set` and `compass config set` each change
//! one key, but serialising the whole [`crate::config::Config`] again puts the
//! keys in the types' order and the indentation in serde's. Someone who keeps
//! the file by hand then finds it rearranged after every change. Instead the
//! file on disk is edited in place through `jsonc-parser`'s concrete syntax
//! tree: an unchanged value is not touched, a changed one is replaced where it
//! stands, a removed key is removed, and a new key is appended to its object.

use jsonc_parser::ParseOptions;
use jsonc_parser::cst::{CstInputValue, CstObject, CstRootNode};
use serde_json::{Map, Value};

/// `existing` with its values brought in line with `value`, keeping its
/// layout; `None` when `existing` is not a JSON object this can edit, in
/// which case the caller writes the file afresh.
#[must_use]
pub fn rewrite(existing: &str, value: &Value) -> Option<String> {
    let Value::Object(wanted) = value else {
        return None;
    };
    let root = CstRootNode::parse(existing, &ParseOptions::default()).ok()?;
    let object = root.object_value()?;
    sync(&object, wanted);
    let mut out = root.to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Some(out)
}

fn sync(object: &CstObject, wanted: &Map<String, Value>) {
    for property in object.properties() {
        let Some(name) = property.name().and_then(|name| name.decoded_value().ok()) else {
            continue;
        };
        let Some(value) = wanted.get(&name) else {
            property.remove();
            continue;
        };
        let current = property.value().and_then(|node| node.to_serde_value());
        if current.as_ref() == Some(value) {
            continue;
        }
        match (property.object_value(), value) {
            (Some(child), Value::Object(map)) => sync(&child, map),
            _ => property.set_value(input(value)),
        }
    }
    for (key, value) in wanted {
        if object.get(key).is_none() {
            object.append(key, input(value));
        }
    }
}

fn input(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(value) => CstInputValue::Bool(*value),
        Value::Number(number) => CstInputValue::Number(number.to_string()),
        Value::String(text) => CstInputValue::String(text.clone()),
        Value::Array(items) => CstInputValue::Array(items.iter().map(input).collect()),
        Value::Object(map) => CstInputValue::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), input(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_changed_value_is_replaced_where_it_stands() {
        let existing = "{\n    \"tray\": { \"enabled\": true },\n    \"launcher\": {\n        \"appearance\": { \"theme\": \"nord\" },\n        \"hotkey\": \"alt+space\"\n    }\n}\n";
        let wanted = json!({
            "launcher": {"hotkey": "alt+space", "appearance": {"theme": "dracula"}},
            "tray": {"enabled": true},
        });
        let out = rewrite(existing, &wanted).unwrap();
        assert_eq!(out, existing.replace("nord", "dracula"));
    }

    #[test]
    fn keys_are_added_at_the_end_and_removed_in_place() {
        let existing = "{\n  \"b\": 1,\n  \"a\": 2\n}\n";
        let out = rewrite(existing, &json!({"b": 1, "c": [true]})).unwrap();
        let reread: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(reread, json!({"b": 1, "c": [true]}));
        assert!(out.find("\"b\"").unwrap() < out.find("\"c\"").unwrap());
        assert!(!out.contains("\"a\""));
    }

    #[test]
    fn something_that_is_not_an_object_is_left_to_the_caller() {
        assert_eq!(rewrite("[1, 2]", &json!({})), None);
        assert_eq!(rewrite("{", &json!({})), None);
    }
}
