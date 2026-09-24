//! The published `vicinae.json` schema is the one the types generate.
//!
//! `packaging/schema/vicinae.schema.json` is committed so editors and packagers can point at a
//! stable file, and so a change to it shows up in review. It is never edited by hand. After
//! changing `compass_core::config`, regenerate it with
//!
//! ```sh
//! COMPASS_UPDATE_SCHEMA=1 cargo test -p compass-core --test config_schema
//! ```

use std::path::PathBuf;

use compass_core::config::{SCHEMA_URL, json_schema, json_schema_pretty};
use serde_json::Value;

fn committed_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packaging/schema/vicinae.schema.json")
}

#[test]
fn the_committed_schema_matches_the_types() {
    let generated = json_schema_pretty();
    let path = committed_path();

    if std::env::var_os("COMPASS_UPDATE_SCHEMA").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &generated).unwrap();
        return;
    }

    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "{} is out of date with compass_core::config. Regenerate it with\n\n    \
         COMPASS_UPDATE_SCHEMA=1 cargo test -p compass-core --test config_schema\n",
        path.display()
    );
}

#[test]
fn the_schema_identifies_itself_where_configs_point() {
    let schema = json_schema();
    assert_eq!(schema["$id"], SCHEMA_URL);
    assert_eq!(
        schema["$schema"], "https://json-schema.org/draft/2020-12/schema",
        "editors key their validation off the draft"
    );
}

fn property<'a>(schema: &'a Value, root: &'a Value, path: &[&str]) -> &'a Value {
    let mut node = schema;
    for key in path {
        node = resolve(&node["properties"][key], root);
        assert!(!node.is_null(), "no property {key} in {path:?}");
    }
    node
}

/// Follows a `$ref`, or an `anyOf` of one `$ref` and `null`, to the definition.
fn resolve<'a>(node: &'a Value, root: &'a Value) -> &'a Value {
    if let Some(reference) = node["$ref"].as_str() {
        let name = reference.trim_start_matches("#/$defs/");
        return &root["$defs"][name];
    }
    node
}

#[test]
fn every_documented_key_is_in_the_schema_with_its_default() {
    let root = json_schema();
    for (path, default) in [
        (&["launcher", "hotkey"][..], Value::from("super+space")),
        (&["launcher", "close_on_focus_loss"], Value::from(false)),
        (&["launcher", "max_results"], Value::from(50)),
        (&["launcher", "keybinding"], Value::from("default")),
        (&["launcher", "wrap_navigation"], Value::from(false)),
        (&["launcher", "quick_launch"], Value::from(true)),
        (
            &["launcher", "appearance", "color_scheme"],
            Value::from("system"),
        ),
        (&["launcher", "appearance", "theme"], Value::from("system")),
        (&["launcher", "appearance", "preset"], Value::from("gnome")),
        (&["launcher", "appearance", "icons"], Value::from(true)),
        (&["launcher", "appearance", "tint"], Value::from(false)),
        (&["extensions", "auto_update"], Value::from(true)),
    ] {
        let node = property(&root, &root, path);
        assert_eq!(node["default"], default, "default of {path:?}");
        assert!(
            node["description"].as_str().is_some_and(|d| !d.is_empty()),
            "{path:?} has no description"
        );
    }
    for path in [
        &["extensions", "installed"][..],
        &["providers"],
        &["favorites"],
        &["fallbacks"],
        &["$schema"],
    ] {
        property(&root, &root, path);
    }
}

#[test]
fn the_published_example_is_a_config_this_build_fully_understands() {
    // CI validates the same file against the schema with a real JSON Schema validator
    // (scripts/packaging/check-config-schema.py); this half proves the types agree with it.
    let path = committed_path().with_file_name("example.vicinae.json");
    let text = std::fs::read_to_string(&path).unwrap();
    let config = compass_core::Config::parse(&text, &path).unwrap();
    assert_eq!(config.schema(), Some(SCHEMA_URL));
    assert!(config.unknown_fields().is_empty());
    assert!(config.launcher().unknown_fields().is_empty());
    assert!(config.launcher().appearance().unknown_fields().is_empty());
    assert!(config.extensions().unknown_fields().is_empty());
}

#[test]
fn the_schema_admits_keys_it_does_not_know() {
    // The config preserves unknown keys so a newer build's file survives an older build. A
    // schema that forbade them would mark those files invalid in every editor.
    let root = json_schema();
    assert_ne!(root["additionalProperties"], Value::Bool(false));
    let launcher = property(&root, &root, &["launcher"]);
    assert_ne!(launcher["additionalProperties"], Value::Bool(false));
}
