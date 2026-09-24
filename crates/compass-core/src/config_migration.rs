//! Migrating the C++ engine's `settings.json` to `vicinae.json`.
//!
//! The C++ engine keeps its user configuration in `$XDG_CONFIG_HOME/vicinae/settings.json`: JSONC
//! (it writes a comment header), `snake_case` keys, and an `imports` array of further files merged
//! underneath it. The Rust engine reads `vicinae.json` beside it, whose shape is [`Config`]. This
//! module reads the former the way the C++ `config::Manager` does — imports resolved relative to
//! the importing file, a leading `~` expanded, cycles ignored, a missing import skipped, the
//! importing file winning over what it imports — and translates every key that has a
//! counterpart.
//!
//! Keys with no counterpart are not carried over as unknown fields: they belong to a different
//! program, and preserving them would make `vicinae.json` look as though it honoured them. They
//! are listed in [`Migration::skipped`] instead, so the user can see exactly what did not move.
//!
//! The C++ file is never written to.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::config::{Config, ConfigError, SCHEMA_URL};

/// Path of the C++ engine's settings relative to `$XDG_CONFIG_HOME`.
pub const LEGACY_RELATIVE_PATH: &str = "vicinae/settings.json";

/// `$XDG_CONFIG_HOME/vicinae/settings.json`, falling back to `~/.config`.
///
/// # Errors
///
/// [`ConfigError::NoConfigDir`] when neither `$XDG_CONFIG_HOME` nor `$HOME` is usable.
pub fn legacy_config_path() -> Result<PathBuf, ConfigError> {
    let dir = dirs::config_dir().ok_or(ConfigError::NoConfigDir)?;
    Ok(dir.join(LEGACY_RELATIVE_PATH))
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
    /// The dotted key in `vicinae.json`.
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
        reason: "no vicinae.json equivalent".to_owned(),
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

#[derive(Debug, Clone, Copy)]
enum Kind {
    Bool,
    String,
    Strings,
    Object,
}

impl Kind {
    fn accepts(self, value: &Value) -> bool {
        match self {
            Kind::Bool => value.is_boolean(),
            Kind::String => value.is_string(),
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
        }
    }
}

/// Settings whose meaning is the same in both engines, `settings.json` key first.
///
/// `providers` is copied whole: the C++ and Rust shapes agree on `enabled` and on `entrypoints`
/// with `enabled`, `alias` and `shortcut`, and the per-provider `preferences` the Rust root
/// manager does not read yet survive as unknown fields rather than being lost.
const DIRECT: [(&str, &str, Kind); 7] = [
    (
        "close_on_focus_loss",
        "launcher.close_on_focus_loss",
        Kind::Bool,
    ),
    ("wrap_navigation", "launcher.wrap_navigation", Kind::Bool),
    ("keybinding", "launcher.keybinding", Kind::String),
    ("global_shortcuts.toggle", "launcher.hotkey", Kind::String),
    ("favorites", "favorites", Kind::Strings),
    ("fallbacks", "fallbacks", Kind::Strings),
    ("providers", "providers", Kind::Object),
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
/// Dark is preferred when both are set and disagree, since it is the C++ default variant; the
/// light one is then reported, not silently dropped.
fn migrate_theme(
    settings: &mut Map<String, Value>,
    out: &mut Map<String, Value>,
    mapped: &mut Vec<Mapped>,
    skipped: &mut Vec<Skipped>,
) {
    let mut chosen: Option<&'static str> = None;
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
        match (theme_family(name), chosen) {
            (None, _) => skipped.push(Skipped {
                key,
                reason: format!("theme {name:?} has no Rust engine equivalent"),
            }),
            (Some(family), None) => {
                chosen = Some(family);
                insert_path(
                    out,
                    "launcher.appearance.theme",
                    Value::String(family.to_owned()),
                );
                mapped.push(Mapped {
                    from: key,
                    to: "launcher.appearance.theme".to_owned(),
                });
            }
            (Some(family), Some(already)) if family == already => mapped.push(Mapped {
                from: key,
                to: "launcher.appearance.theme".to_owned(),
            }),
            (Some(family), Some(already)) => skipped.push(Skipped {
                key,
                reason: format!(
                    "the {variant} theme is {family:?} but the dark one, {already:?}, was used"
                ),
            }),
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
