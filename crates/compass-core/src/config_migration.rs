//! Migrating the C++ engine's `settings.json` to `compass.json`.
//!
//! The C++ engine keeps its user configuration in `$XDG_CONFIG_HOME/vicinae/settings.json`: JSONC
//! (it writes a comment header), `snake_case` keys, and an `imports` array of further files merged
//! underneath it. The Rust engine reads `compass.json` beside it, whose shape is [`Config`]. This
//! module reads the former the way the C++ `config::Manager` does — imports resolved relative to
//! the importing file, a leading `~` expanded, cycles ignored, a missing import skipped, the
//! importing file winning over what it imports — and translates every key that has a
//! counterpart.
//!
//! Keys with no counterpart are not carried over as unknown fields: they belong to a different
//! program, and preserving them would make `compass.json` look as though it honoured them. They
//! are listed in [`Migration::skipped`] instead, so the user can see exactly what did not move.
//!
//! The C++ file is never written to.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::config::{Config, ConfigError, SCHEMA_URL};

/// Path of the C++ engine's settings relative to `$XDG_CONFIG_HOME`.
///
/// The C++ writes `vicinae/settings.json`; the startup migration moves that
/// directory to `compass` and leaves `vicinae` as a symlink to it, so this is
/// the same file either way.
pub const LEGACY_RELATIVE_PATH: &str = "compass/settings.json";

/// Where Vicinae keeps it before Compass's engine has run once and moved its
/// directory, relative to `$XDG_CONFIG_HOME`.
pub const UNMOVED_RELATIVE_PATH: &str = "vicinae/settings.json";

/// `$XDG_CONFIG_HOME/compass/settings.json`, falling back to `~/.config`.
///
/// # Errors
///
/// [`ConfigError::NoConfigDir`] when neither `$XDG_CONFIG_HOME` nor `$HOME` is usable.
pub fn legacy_config_path() -> Result<PathBuf, ConfigError> {
    let dir = crate::xdg_dirs::config_home().ok_or(ConfigError::NoConfigDir)?;
    Ok(dir.join(LEGACY_RELATIVE_PATH))
}

/// The Vicinae `settings.json` to migrate: the one in Compass's directory
/// (where the engine moves it), else the one still in `vicinae/` when the
/// engine has not run yet. `None` when neither exists.
#[must_use]
pub fn find_legacy_config() -> Option<PathBuf> {
    let dir = crate::xdg_dirs::config_home()?;
    [LEGACY_RELATIVE_PATH, UNMOVED_RELATIVE_PATH]
        .into_iter()
        .map(|relative| dir.join(relative))
        .find(|path| path.is_file())
}

/// Keys only a Vicinae settings file has at its top level. A `compass.json`
/// holding any of them was carried over from Vicinae verbatim (the move of
/// `vicinae.json`), and is translated by [`translate_vicinae_keys`].
const VICINAE_ONLY_KEYS: &[&str] = &[
    "launcher_window",
    "theme",
    "close_on_focus_loss",
    "wrap_navigation",
    "keybinding",
    "pop_to_root_on_close",
    "imports",
    "telemetry",
    "search_files_in_root",
    "escape_key_behavior",
    "pop_on_backspace",
    "activate_on_single_click",
    "consider_preedit",
];

/// The top-level keys of `compass.json` that are Compass's own and that a
/// Vicinae file never has.
const COMPASS_ONLY_KEYS: &[&str] = &["launcher", "extensions"];

/// Translates the Vicinae keys in a `compass.json` object, as [`migrate_value`]
/// translates `settings.json`, keeping every key that is already Compass's
/// (`launcher`, `extensions`, and anything both files share), which win over
/// a translated value. `None` when the object has no Vicinae key.
#[must_use]
pub fn translate_vicinae_keys(document: &Map<String, Value>) -> Option<Migration> {
    let vicinae = document.iter().any(|(key, value)| {
        VICINAE_ONLY_KEYS.contains(&key.as_str())
            || (key == "global_shortcuts" && value.get("toggle").is_some())
    });
    if !vicinae {
        return None;
    }
    let mut upstream = document.clone();
    let mut own = Map::new();
    for key in COMPASS_ONLY_KEYS {
        if let Some(value) = upstream.remove(*key) {
            own.insert((*key).to_owned(), value);
        }
    }
    let mut migration = migrate_value(upstream);
    if !own.is_empty() {
        let mut merged = serde_json::to_value(&migration.config)
            .ok()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        merge(&mut merged, own);
        migration.config = crate::config_issues::parse_value(Value::Object(merged)).0;
    }
    Some(migration)
}

/// Translates the Vicinae keys of the `compass.json` at `path` in place, keeping
/// the file as it was in `<path>.vicinae.bak`. `Ok(None)` when there is no
/// file, it is not a JSON object, or it has no Vicinae key.
///
/// # Errors
///
/// When the file cannot be read, the backup made or the result written.
pub fn translate_file_in_place(path: &Path) -> Result<Option<Migration>, MigrationError> {
    if !path.is_file() {
        return Ok(None);
    }
    let document = read_object(path)?;
    let Some(migration) = translate_vicinae_keys(&document) else {
        return Ok(None);
    };
    let mut backup = path.as_os_str().to_owned();
    backup.push(".vicinae.bak");
    std::fs::copy(path, PathBuf::from(&backup)).map_err(|source| MigrationError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let text = migration
        .config
        .to_json_pretty()
        .map_err(|error| MigrationError::Parse {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    crate::atomic_write(path, text.as_bytes()).map_err(|source| MigrationError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(Some(migration))
}

/// Everything that can stop a migration.
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    /// A settings file, or a file it imports, could not be read.
    #[error("could not read {path}")]
    Read {
        /// The offending path.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// A settings file is not valid JSONC.
    #[error("invalid settings at {path}: {message}")]
    Parse {
        /// The offending path.
        path: PathBuf,
        /// What the parser objected to, with its position.
        message: String,
    },

    /// A settings file is valid JSONC but not an object.
    #[error("{path} is not a JSON object")]
    NotAnObject {
        /// The offending path.
        path: PathBuf,
    },
}

/// One setting carried across.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Mapped {
    /// The dotted key in `settings.json`.
    pub from: String,
    /// The dotted key in `compass.json`.
    pub to: String,
}

/// One setting left behind, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Skipped {
    /// The dotted key in `settings.json`.
    pub key: String,
    /// Why it was not carried across.
    pub reason: String,
}

/// The result of a migration: the new configuration and an account of every key.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Migration {
    /// The migrated configuration, with `$schema` pointing at the published schema.
    pub config: Config,
    /// Every file read, the settings file first and then its imports in the order they were met.
    pub sources: Vec<PathBuf>,
    /// Imports that were named but do not exist. The C++ skips these with a warning, and so does
    /// this.
    pub missing_imports: Vec<PathBuf>,
    /// Settings carried across.
    pub mapped: Vec<Mapped>,
    /// Settings left behind.
    pub skipped: Vec<Skipped>,
}

impl Migration {
    /// Settings with no counterpart, as dotted keys. Shorthand over [`Migration::skipped`].
    #[must_use]
    pub fn unmapped(&self) -> Vec<&str> {
        self.skipped.iter().map(|s| s.key.as_str()).collect()
    }
}

/// Migrates the settings file at `path`, following its imports.
///
/// # Errors
///
/// [`MigrationError`] when the file or one of its existing imports cannot be read or parsed.
pub fn migrate_file(path: impl AsRef<Path>) -> Result<Migration, MigrationError> {
    let path = path.as_ref();
    let mut sources = Vec::new();
    let mut missing_imports = Vec::new();
    let mut visited = HashSet::from([canonical(path)]);
    let merged = load_with_imports(path, &mut visited, &mut sources, &mut missing_imports)?;
    let mut migration = migrate_value(merged);
    migration.sources = sources;
    migration.missing_imports = missing_imports;
    Ok(migration)
}

/// Migrates an already merged `settings.json` object.
///
/// `imports` and `$schema` are consumed rather than reported: they describe the file, not a
/// setting.
#[must_use]
pub fn migrate_value(mut settings: Map<String, Value>) -> Migration {
    let mut out = Map::new();
    let mut mapped = Vec::new();
    let mut skipped = Vec::new();

    settings.remove("$schema");
    settings.remove("imports");

    for (from, to, kind) in DIRECT {
        let Some(value) = remove_path(&mut settings, from) else {
            continue;
        };
        if value.is_null() {
            continue;
        }
        if kind.accepts(&value) {
            insert_path(&mut out, to, value);
            mapped.push(Mapped {
                from: from.to_owned(),
                to: to.to_owned(),
            });
        } else {
            skipped.push(Skipped {
                key: from.to_owned(),
                reason: format!("expected {}", kind.describe()),
            });
        }
    }

    migrate_theme(&mut settings, &mut out, &mut mapped, &mut skipped);

    let mut leftover = Vec::new();
    flatten_leaves(&Value::Object(settings), String::new(), &mut leftover);
    skipped.extend(leftover.into_iter().map(|key| Skipped {
        key,
        reason: "no compass.json equivalent".to_owned(),
    }));

    out.insert("$schema".to_owned(), Value::String(SCHEMA_URL.to_owned()));
    let config = match serde_json::from_value::<Config>(Value::Object(out.clone())) {
        Ok(config) => config,
        Err(error) => {
            // Every key but `providers` is type-checked on the way in, so a shape the Rust types
            // reject can only have come from there.
            out.remove("providers");
            mapped.retain(|m| m.from != "providers");
            skipped.push(Skipped {
                key: "providers".to_owned(),
                reason: format!("not in the expected shape: {error}"),
            });
            serde_json::from_value(Value::Object(out)).unwrap_or_default()
        }
    };

    skipped.sort_by(|a, b| a.key.cmp(&b.key));
    Migration {
        config,
        sources: Vec::new(),
        missing_imports: Vec::new(),
        mapped,
        skipped,
    }
}

/// Where Compass keeps what a top-level key of a Vicinae `settings.json`
/// sets, for a key someone carried into `compass.json` by hand:
/// `close_on_focus_loss` is `launcher.close_on_focus_loss`, and a section
/// (`launcher_window`) points at the Compass section its keys moved to.
#[must_use]
pub fn compass_key_for(key: &str) -> Option<&'static str> {
    if key == "theme" {
        return Some("launcher.appearance.theme");
    }
    if let Some((_, to, _)) = DIRECT.iter().find(|(from, _, _)| *from == key) {
        return Some(to);
    }
    DIRECT
        .iter()
        .find(|(from, _, _)| {
            from.strip_prefix(key)
                .is_some_and(|rest| rest.starts_with('.'))
        })
        .map(|(_, to, _)| to.rsplit_once('.').map_or(*to, |(section, _)| section))
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    Bool,
    String,
    Strings,
    Object,
    /// A non-negative whole number.
    Count,
}

impl Kind {
    fn accepts(self, value: &Value) -> bool {
        match self {
            Kind::Bool => value.is_boolean(),
            Kind::String => value.is_string(),
            Kind::Count => value.is_u64(),
            Kind::Strings => value
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_string)),
            Kind::Object => value.is_object(),
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Kind::Bool => "a boolean",
            Kind::String => "a string",
            Kind::Strings => "an array of strings",
            Kind::Object => "an object",
            Kind::Count => "a whole number",
        }
    }
}

/// Settings whose meaning is the same in both engines, `settings.json` key first.
///
/// `providers` is copied whole: the C++ and Rust shapes agree on `enabled` and on `entrypoints`
/// with `enabled`, `alias` and `shortcut`, and the per-provider `preferences` the Rust root
/// manager does not read yet survive as unknown fields rather than being lost.
const DIRECT: [(&str, &str, Kind); 16] = [
    (
        "pop_to_root_on_close",
        "launcher.pop_to_root_on_close",
        Kind::Bool,
    ),
    // Read by Compass as Vicinae writes them.
    ("font", "font", Kind::Object),
    ("favicon_service", "favicon_service", Kind::String),
    (
        "launcher_window.clock.enabled",
        "launcher.clock.enabled",
        Kind::Bool,
    ),
    (
        "launcher_window.clock.format",
        "launcher.clock.format",
        Kind::String,
    ),
    (
        "launcher_window.clock.interval",
        "launcher.clock.interval",
        Kind::Count,
    ),
    (
        "close_on_focus_loss",
        "launcher.close_on_focus_loss",
        Kind::Bool,
    ),
    ("wrap_navigation", "launcher.wrap_navigation", Kind::Bool),
    ("keybinding", "launcher.keybinding", Kind::String),
    ("global_shortcuts.toggle", "launcher.hotkey", Kind::String),
    (
        "global_shortcuts.inhibit_apps",
        "global_shortcuts.inhibit_apps",
        Kind::Strings,
    ),
    ("favorites", "favorites", Kind::Strings),
    ("fallbacks", "fallbacks", Kind::Strings),
    ("providers", "providers", Kind::Object),
    ("input_server.enabled", "input_server.enabled", Kind::Bool),
    ("tray.enabled", "tray.enabled", Kind::Bool),
];

/// C++ theme ids and the Rust theme family each belongs to.
///
/// The C++ names a theme per variant (`theme.dark.name`, `theme.light.name`); the Rust engine
/// names a family and picks the variant from the colour scheme. `vicinae-*` and `libadwaita-*`
/// are the defaults on either side, so they become `system`.
const THEME_FAMILIES: [(&str, &str); 17] = [
    ("vicinae-dark", "system"),
    ("vicinae-light", "system"),
    ("libadwaita-dark", "system"),
    ("libadwaita-light", "system"),
    ("catppuccin-mocha", "catppuccin"),
    ("catppuccin-macchiato", "catppuccin"),
    ("catppuccin-frappe", "catppuccin"),
    ("catppuccin-latte", "catppuccin"),
    ("dracula", "dracula"),
    ("nord", "nord"),
    ("nord-light", "nord"),
    ("gruvbox-dark", "gruvbox"),
    ("gruvbox-light", "gruvbox"),
    ("solarized-dark", "solarized"),
    ("solarized-light", "solarized"),
    ("tokyo-night", "tokyo-night"),
    ("tokyo-night-storm", "tokyo-night"),
];

fn theme_family(name: &str) -> Option<&'static str> {
    THEME_FAMILIES
        .iter()
        .find(|(id, _)| *id == name)
        .map(|(_, family)| *family)
}

/// `theme.dark.name` and `theme.light.name` become `launcher.appearance.theme`.
///
/// Vicinae names a theme per variant; Compass names one family and picks its light or dark
/// variant from the colour scheme. A chosen theme wins over a default one (`vicinae-light`,
/// `libadwaita-dark`), whichever variant it was set for. When both variants name different
/// chosen themes, the dark one is kept and the light one is reported by its own name.
fn migrate_theme(
    settings: &mut Map<String, Value>,
    out: &mut Map<String, Value>,
    mapped: &mut Vec<Mapped>,
    skipped: &mut Vec<Skipped>,
) {
    const KEY: &str = "launcher.appearance.theme";
    let mut named: Vec<(String, String, &'static str)> = Vec::new();
    for variant in ["dark", "light"] {
        let key = format!("theme.{variant}.name");
        let Some(value) = remove_path(settings, &key) else {
            continue;
        };
        let Some(name) = value.as_str() else {
            if !value.is_null() {
                skipped.push(Skipped {
                    key,
                    reason: "expected a string".to_owned(),
                });
            }
            continue;
        };
        match theme_family(name) {
            Some(family) => named.push((key, name.to_owned(), family)),
            None => skipped.push(Skipped {
                key,
                reason: format!("Compass has no theme like {name:?}"),
            }),
        }
    }
    let Some((_, chosen_name, chosen)) = named
        .iter()
        .find(|(_, _, family)| *family != "system")
        .or_else(|| named.first())
        .cloned()
    else {
        return;
    };
    insert_path(out, KEY, Value::String(chosen.to_owned()));
    for (key, name, family) in named {
        if family == chosen || family == "system" {
            mapped.push(Mapped {
                from: key,
                to: KEY.to_owned(),
            });
        } else {
            skipped.push(Skipped {
                key,
                reason: format!(
                    "Compass uses one theme for light and dark, so {name:?} was not carried over; \
                     {chosen_name:?} was, as {chosen:?}"
                ),
            });
        }
    }
}

fn remove_path(map: &mut Map<String, Value>, dotted: &str) -> Option<Value> {
    match dotted.split_once('.') {
        None => map.remove(dotted),
        Some((head, rest)) => {
            let child = map.get_mut(head)?.as_object_mut()?;
            let value = remove_path(child, rest);
            if child.is_empty() {
                map.remove(head);
            }
            value
        }
    }
}

fn insert_path(map: &mut Map<String, Value>, dotted: &str, value: Value) {
    match dotted.split_once('.') {
        None => {
            map.insert(dotted.to_owned(), value);
        }
        Some((head, rest)) => {
            let child = map
                .entry(head.to_owned())
                .or_insert_with(|| Value::Object(Map::new()));
            if !child.is_object() {
                *child = Value::Object(Map::new());
            }
            if let Value::Object(child) = child {
                insert_path(child, rest, value);
            }
        }
    }
}

fn flatten_leaves(value: &Value, prefix: String, out: &mut Vec<String>) {
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten_leaves(child, path, out);
            }
        }
        Value::Null => {}
        _ if !prefix.is_empty() => out.push(prefix),
        _ => {}
    }
}

/// Deep merge as the C++ reads a key twice: objects merge, anything else is replaced.
fn merge(base: &mut Map<String, Value>, over: Map<String, Value>) {
    for (key, value) in over {
        match (base.get_mut(&key), value) {
            (Some(Value::Object(base)), Value::Object(over)) => merge(base, over),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// `config::Manager::resolvePath`: `~` expanded, relative paths taken from the importing file.
fn resolve_import(import: &str, importer: &Path) -> PathBuf {
    let expanded = match import.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => match dirs::home_dir() {
            Some(home) => home.join(rest.trim_start_matches('/')),
            None => PathBuf::from(import),
        },
        _ => PathBuf::from(import),
    };
    if expanded.is_absolute() {
        expanded
    } else {
        importer
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(expanded)
    }
}

fn read_object(path: &Path) -> Result<Map<String, Value>, MigrationError> {
    let text = std::fs::read_to_string(path).map_err(|source| MigrationError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let value: Value =
        jsonc_parser::parse_to_serde_value(&text, &Default::default()).map_err(|error| {
            MigrationError::Parse {
                path: path.to_path_buf(),
                message: error.to_string(),
            }
        })?;
    match value {
        Value::Object(map) => Ok(map),
        Value::Null => Ok(Map::new()),
        _ => Err(MigrationError::NotAnObject {
            path: path.to_path_buf(),
        }),
    }
}

fn load_with_imports(
    path: &Path,
    visited: &mut HashSet<PathBuf>,
    sources: &mut Vec<PathBuf>,
    missing: &mut Vec<PathBuf>,
) -> Result<Map<String, Value>, MigrationError> {
    let mut settings = read_object(path)?;
    sources.push(path.to_path_buf());

    let imports: Vec<String> = settings
        .get("imports")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();

    for import in imports {
        let resolved = resolve_import(&import, path);
        if !resolved.exists() {
            tracing::warn!(import = %resolved.display(), "imported settings file not found");
            missing.push(resolved);
            continue;
        }
        if !visited.insert(canonical(&resolved)) {
            tracing::warn!(import = %resolved.display(), "circular settings import ignored");
            continue;
        }
        let mut imported = load_with_imports(&resolved, visited, sources, missing)?;
        merge(&mut imported, settings);
        settings = imported;
    }

    Ok(settings)
}
