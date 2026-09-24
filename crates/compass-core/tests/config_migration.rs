//! Migrating the C++ engine's `settings.json` to `vicinae.json`.

use std::path::Path;

use compass_core::config::{Config, SCHEMA_URL};
use compass_core::config_migration::{MigrationError, migrate_file};
use serde_json::{Value, json};

fn write(dir: &Path, name: &str, text: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    path
}

fn as_json(config: &Config) -> Value {
    serde_json::from_str(&config.to_json_pretty().unwrap()).unwrap()
}

/// What the C++ `config::Manager::writeUser` produces: its comment header, then snake_case JSON.
const CPP_SETTINGS: &str = r#"// This configuration is merged with the default vicinae configuration file, which you can obtain by running the `vicinae config default` command.
// Every item defined in this file takes precedence over the values defined in the default config or any other imported file.
//
// Learn more about configuration at https://docs.vicinae.com/config

{
  "$schema": "https://vicinae.com/schemas/config.json",
  "close_on_focus_loss": true,
  "wrap_navigation": true,
  "keybinding": "emacs",
  "pop_to_root_on_close": true,
  "favorites": ["applications:firefox", "clipboard:history"],
  "fallbacks": ["files:search"],
  "global_shortcuts": { "toggle": "super+space", "inhibit_apps": ["steam"] },
  "font": { "normal": { "size": 11 } },
  "theme": {
    "dark": { "name": "catppuccin-mocha", "icon_theme": "Papirus" },
    "light": { "name": "catppuccin-latte" }
  },
  "launcher_window": { "opacity": 0.9, "blur": { "enabled": false } },
  "providers": {
    "applications": {
      "enabled": true,
      "preferences": { "paths": ["/opt/apps"] },
      "entrypoints": {
        "firefox": { "alias": "ff", "shortcut": "ctrl+shift+f" },
        "gimp": { "enabled": false }
      }
    }
  },
  "keybinds": { "open-search-filter": "control+P" },
}
"#;

#[test]
fn every_shared_setting_is_carried_across() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "settings.json", CPP_SETTINGS);

    let migration = migrate_file(&path).unwrap();

    assert_eq!(
        as_json(&migration.config),
        json!({
            "$schema": SCHEMA_URL,
            "launcher": {
                "hotkey": "super+space",
                "close_on_focus_loss": true,
                "keybinding": "emacs",
                "wrap_navigation": true,
                "appearance": { "theme": "catppuccin" }
            },
            "providers": {
                "applications": {
                    "enabled": true,
                    "preferences": { "paths": ["/opt/apps"] },
                    "entrypoints": {
                        "firefox": { "alias": "ff", "shortcut": "ctrl+shift+f" },
                        "gimp": { "enabled": false }
                    }
                }
            },
            "favorites": ["applications:firefox", "clipboard:history"],
            "fallbacks": ["files:search"]
        })
    );
    assert_eq!(migration.sources, vec![path]);
    assert!(migration.missing_imports.is_empty());
}

#[test]
fn settings_with_no_equivalent_are_reported_not_smuggled_in() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "settings.json", CPP_SETTINGS);

    let migration = migrate_file(&path).unwrap();

    assert_eq!(
        migration.unmapped(),
        vec![
            "font.normal.size",
            "global_shortcuts.inhibit_apps",
            "keybinds.open-search-filter",
            "launcher_window.blur.enabled",
            "launcher_window.opacity",
            "pop_to_root_on_close",
            "theme.dark.icon_theme",
        ]
    );
    assert!(
        migration.config.unknown_fields().is_empty(),
        "C++-only keys must not reappear as unknown vicinae.json fields"
    );
    assert!(
        migration
            .mapped
            .iter()
            .any(|m| m.from == "global_shortcuts.toggle" && m.to == "launcher.hotkey")
    );
}

#[test]
fn the_migrated_file_round_trips_through_the_rust_reader() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "settings.json", CPP_SETTINGS);
    let migration = migrate_file(&path).unwrap();

    let out = dir.path().join("vicinae.json");
    migration.config.save_to(&out).unwrap();
    let reread = Config::load_from(&out).unwrap();

    assert_eq!(reread, migration.config);
    assert_eq!(reread.launcher().keybinding(), "emacs");
    assert_eq!(reread.launcher().hotkey(), "super+space");
    assert!(reread.launcher().close_on_focus_loss());
    assert_eq!(reread.launcher().appearance().theme(), "catppuccin");
    assert_eq!(reread.schema(), Some(SCHEMA_URL));
    let root = reread.root_config();
    assert_eq!(root.favorites.len(), 2);
    assert_eq!(
        root.providers["applications"].entrypoints["firefox"]
            .alias
            .as_deref(),
        Some("ff")
    );
}

#[test]
fn imports_merge_underneath_the_importing_file() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "extra/base.jsonc",
        r#"{
            // an import of its own, relative to *this* file
            "imports": ["deeper.json"],
            "keybinding": "vim",
            "wrap_navigation": true,
            "global_shortcuts": { "toggle": "alt+space" }
        }"#,
    );
    write(
        dir.path(),
        "extra/deeper.json",
        r#"{ "fallbacks": ["files:search"], "keybinding": "default" }"#,
    );
    let path = write(
        dir.path(),
        "settings.json",
        r#"{
            "imports": ["extra/base.jsonc", "missing.json", "settings.json"],
            "keybinding": "emacs",
            "global_shortcuts": { "inhibit_apps": [] }
        }"#,
    );

    let migration = migrate_file(&path).unwrap();
    let launcher = migration.config.launcher();

    assert_eq!(launcher.keybinding(), "emacs", "the importing file wins");
    assert!(
        launcher.wrap_navigation(),
        "keys only an import sets arrive"
    );
    assert_eq!(
        launcher.hotkey(),
        "alt+space",
        "nested objects merge rather than replace"
    );
    assert_eq!(
        migration.config.root_config().fallbacks,
        vec!["files:search"]
    );
    assert_eq!(
        migration.missing_imports,
        vec![dir.path().join("missing.json")]
    );
    assert_eq!(migration.sources.len(), 3, "the self-import is ignored");
}

#[test]
fn a_value_of_the_wrong_type_is_skipped_with_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "settings.json",
        r#"{
            "close_on_focus_loss": "yes",
            "favorites": ["a:b", 3],
            "providers": { "applications": { "enabled": "sometimes" } },
            "wrap_navigation": true
        }"#,
    );

    let migration = migrate_file(&path).unwrap();

    assert!(migration.config.launcher().wrap_navigation());
    assert!(!migration.config.launcher().close_on_focus_loss());
    assert!(migration.config.root_config().favorites.is_empty());
    assert!(migration.config.root_config().providers.is_empty());
    let reasons: Vec<(&str, &str)> = migration
        .skipped
        .iter()
        .map(|s| (s.key.as_str(), s.reason.as_str()))
        .collect();
    assert!(reasons.contains(&("close_on_focus_loss", "expected a boolean")));
    assert!(reasons.contains(&("favorites", "expected an array of strings")));
    assert!(
        reasons
            .iter()
            .any(|(key, reason)| *key == "providers" && reason.contains("expected shape"))
    );
}

#[test]
fn themes_map_to_their_family_and_disagreement_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "settings.json",
        r#"{ "theme": { "dark": { "name": "tokyo-night-storm" }, "light": { "name": "nord-light" } } }"#,
    );
    let migration = migrate_file(&path).unwrap();
    assert_eq!(
        migration.config.launcher().appearance().theme(),
        "tokyo-night"
    );
    assert_eq!(migration.unmapped(), vec!["theme.light.name"]);

    let path = write(
        dir.path(),
        "unknown.json",
        r#"{ "theme": { "light": { "name": "rose-pine-dawn" } } }"#,
    );
    let migration = migrate_file(&path).unwrap();
    assert_eq!(
        migration.config.launcher().appearance().theme_override(),
        None
    );
    assert!(migration.skipped[0].reason.contains("rose-pine-dawn"));
}

#[test]
fn an_empty_settings_file_migrates_to_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "settings.json", "// nothing yet\n");
    let migration = migrate_file(&path).unwrap();
    assert_eq!(as_json(&migration.config), json!({ "$schema": SCHEMA_URL }));
    assert!(migration.mapped.is_empty() && migration.skipped.is_empty());
}

#[test]
fn a_broken_settings_file_is_an_error_naming_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "settings.json", "{ \"keybinding\": }");
    let error = migrate_file(&path).unwrap_err();
    assert!(matches!(error, MigrationError::Parse { .. }), "{error:?}");
    assert!(error.to_string().contains("settings.json"));

    let path = write(dir.path(), "array.json", "[1, 2]");
    assert!(matches!(
        migrate_file(&path).unwrap_err(),
        MigrationError::NotAnObject { .. }
    ));
}

#[test]
fn loading_falls_back_to_the_cpp_settings_only_when_there_is_no_vicinae_json() {
    let dir = tempfile::tempdir().unwrap();
    let primary = dir.path().join("vicinae.json");
    let legacy = write(dir.path(), "settings.json", r#"{ "keybinding": "emacs" }"#);

    let config = Config::load_or_migrate(&primary, Some(&legacy)).unwrap();
    assert_eq!(config.launcher().keybinding(), "emacs");
    assert!(!primary.exists(), "loading never writes");

    std::fs::write(&primary, r#"{ "launcher": { "keybinding": "vim" } }"#).unwrap();
    let config = Config::load_or_migrate(&primary, Some(&legacy)).unwrap();
    assert_eq!(config.launcher().keybinding(), "vim", "vicinae.json wins");

    let config = Config::load_or_migrate(&dir.path().join("none.json"), None).unwrap();
    assert_eq!(config, Config::default());
}

#[test]
fn a_broken_cpp_settings_file_does_not_stop_the_rust_engine() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = write(dir.path(), "settings.json", "{ not json");
    let config = Config::load_or_migrate(&dir.path().join("vicinae.json"), Some(&legacy)).unwrap();
    assert_eq!(config, Config::default());
}
