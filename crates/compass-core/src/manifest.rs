//! An extension's `package.json`, as the launcher reads it.
//!
//! Ports `ExtensionManifest::fromPackageJson`
//! (`src/server/src/services/extension-registry/extension-manifest.cpp`).
//! This is what tells the host which commands an extension has, what
//! arguments they take and which preferences to ask for, so a field read
//! differently here is a command that does not appear or an argument nobody
//! is prompted for.
//!
//! # Four rules that are easy to miss
//!
//! * **A command with an unrecognised `mode` is dropped, silently.** The C++
//!   builds it, gives it `CommandModeInvalid`, and then only keeps commands
//!   whose mode is in `supportedModes` — `view` and `no-view`. A typo in a
//!   manifest makes a command vanish rather than fail.
//! * **`interval` applies only to `no-view` commands**, and a malformed one is
//!   a warning, not an error: the command still loads, without its schedule.
//! * **An interval under five seconds is refused**, so an extension cannot ask
//!   to be re-run continuously.
//! * **`id` is the directory name**, not anything in the file, and the
//!   provenance follows from its prefix. An extension installed from the
//!   Raycast store lives in a directory called `store.raycast.<something>`.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// The npm package whose presence means an extension wants the Raycast API.
pub const RAYCAST_NPM_API_PACKAGE: &str = "@raycast/api";

/// The shortest interval a scheduled command may ask for.
pub const MINIMUM_INTERVAL_SECS: u64 = 5;

/// Where an extension came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// Written or installed by hand.
    Local,
    /// The Vicinae store.
    Vicinae,
    /// The Raycast store.
    Raycast,
}

impl Provenance {
    /// The provenance an extension id implies.
    #[must_use]
    pub fn from_id(id: &str) -> Self {
        if id.starts_with("store.vicinae.") {
            Self::Vicinae
        } else if id.starts_with("store.raycast.") {
            Self::Raycast
        } else {
            Self::Local
        }
    }
}

/// How a command draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandMode {
    /// Has a UI.
    View,
    /// Runs and exits.
    NoView,
}

impl CommandMode {
    /// The mode a manifest's `mode` names, or `None` for anything else.
    ///
    /// `None` is what makes a command disappear — see the module docs.
    #[must_use]
    pub fn parse(mode: &str) -> Option<Self> {
        match mode {
            "view" => Some(Self::View),
            "no-view" => Some(Self::NoView),
            _ => None,
        }
    }
}

/// What kind of value an argument takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArgumentType {
    /// Free text. Also the default, as it is in the C++ — `arg.type` is only
    /// assigned when the string matches, so an unknown type leaves the enum's
    /// first value.
    #[default]
    Text,
    /// Free text, not echoed.
    Password,
    /// One of a list.
    Dropdown,
}

/// One entry of a dropdown, in an argument or a preference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropdownOption {
    /// What the user sees.
    pub title: String,
    /// What the command receives.
    pub value: String,
}

/// An argument a command is launched with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandArgument {
    /// The name the command reads it by.
    pub name: String,
    /// What it takes.
    pub argument_type: ArgumentType,
    /// Placeholder text.
    pub placeholder: String,
    /// Whether it must be filled in. Absent means `false`.
    pub required: bool,
    /// The options, for a dropdown.
    pub data: Option<Vec<DropdownOption>>,
}

/// What kind of preference this is, and whatever comes with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreferenceKind {
    /// A line of text.
    TextField,
    /// A line of text, not echoed.
    Password,
    /// A tick box, with its label.
    Checkbox {
        /// The text beside the box.
        label: String,
    },
    /// Pick an installed application.
    AppPicker,
    /// Pick a file, or several.
    FilePicker {
        /// Whether more than one may be picked.
        multiple: bool,
    },
    /// Pick a directory, or several.
    DirectoryPicker {
        /// Whether more than one may be picked.
        multiple: bool,
    },
    /// Pick from a list.
    Dropdown {
        /// The options.
        options: Vec<DropdownOption>,
    },
    /// A `type` this build does not know.
    ///
    /// The C++ logs a warning and leaves the preference's data unset, so it
    /// keeps the title and name and offers no way to edit it. Carried as a
    /// value here so a caller can say so rather than showing a blank row.
    Unknown {
        /// What the manifest said.
        declared: String,
    },
}

/// Something the user can configure.
#[derive(Debug, Clone, PartialEq)]
pub struct Preference {
    /// The name the extension reads it by.
    pub name: String,
    /// What the user sees.
    pub title: String,
    /// Longer text.
    pub description: String,
    /// Placeholder text.
    pub placeholder: String,
    /// Whether it must be set.
    pub required: bool,
    /// The default, as JSON — its type depends on the kind.
    pub default: Option<Value>,
    /// What kind it is.
    pub kind: PreferenceKind,
}

/// One command an extension offers.
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    /// The command's id within the extension, and its file's stem.
    pub name: String,
    /// What the user sees.
    pub title: String,
    /// Longer text.
    pub description: String,
    /// How it draws.
    pub mode: CommandMode,
    /// Preferences scoped to this command.
    pub preferences: Vec<Preference>,
    /// Extra search terms.
    pub keywords: Vec<String>,
    /// Arguments it is launched with.
    pub arguments: Vec<CommandArgument>,
    /// An icon overriding the extension's.
    pub icon: Option<String>,
    /// How often to re-run it, for a `no-view` command.
    pub interval: Option<std::time::Duration>,
    /// The file the worker loads.
    pub entrypoint: PathBuf,
    /// Whether it starts switched off.
    pub default_disabled: bool,
    /// Where the extension came from.
    pub provenance: Provenance,
}

/// An extension, as its `package.json` describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionManifest {
    /// The directory it lives in.
    pub path: PathBuf,
    /// That directory's name.
    pub id: String,
    /// The `name` field.
    pub name: String,
    /// The `title` field.
    pub title: String,
    /// The `description` field.
    pub description: String,
    /// The `icon` field.
    pub icon: String,
    /// The `author` field.
    pub author: String,
    /// The `categories` field.
    pub categories: Vec<String>,
    /// Extension-wide preferences.
    pub preferences: Vec<Preference>,
    /// The commands that survived mode validation.
    pub commands: Vec<Command>,
    /// Whether `dependencies` names [`RAYCAST_NPM_API_PACKAGE`].
    pub needs_raycast_api: bool,
    /// Where it came from.
    pub provenance: Provenance,
}

/// Why a manifest could not be read.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// There is no `package.json` in the directory.
    #[error("could not find package.json file at {0}")]
    Missing(PathBuf),

    /// It could not be read.
    #[error("failed to open {path} for read: {source}")]
    Read {
        /// Which file.
        path: PathBuf,
        /// Why.
        source: std::io::Error,
    },

    /// It is not JSON, or not an object.
    #[error("failed to parse package.json at {0}")]
    Parse(PathBuf),
}

fn string(object: &Value, key: &str) -> String {
    object
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn string_array(object: &Value, key: &str) -> Vec<String> {
    object
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| item.as_str().unwrap_or_default().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn options(object: &Value) -> Vec<DropdownOption> {
    object
        .get("data")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| DropdownOption {
                    title: string(item, "title"),
                    value: string(item, "value"),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Reads one preference object.
#[must_use]
pub fn parse_preference(object: &Value) -> Preference {
    let declared = string(object, "type");
    let kind = match declared.as_str() {
        "textfield" => PreferenceKind::TextField,
        "password" => PreferenceKind::Password,
        "checkbox" => PreferenceKind::Checkbox {
            label: string(object, "label"),
        },
        "appPicker" => PreferenceKind::AppPicker,
        "file" => PreferenceKind::FilePicker {
            multiple: object
                .get("multiple")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
        "directory" => PreferenceKind::DirectoryPicker {
            multiple: object
                .get("multiple")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
        "dropdown" => PreferenceKind::Dropdown {
            options: options(object),
        },
        _ => PreferenceKind::Unknown { declared },
    };

    Preference {
        name: string(object, "name"),
        title: string(object, "title"),
        description: string(object, "description"),
        placeholder: string(object, "placeholder"),
        required: object
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        default: object.get("default").cloned(),
        kind,
    }
}

/// Reads one argument object.
#[must_use]
pub fn parse_argument(object: &Value) -> CommandArgument {
    let declared = string(object, "type");
    let argument_type = match declared.as_str() {
        "password" => ArgumentType::Password,
        "dropdown" => ArgumentType::Dropdown,
        // Including an unknown type: the C++ only assigns on a match, so the
        // enum keeps its first value.
        _ => ArgumentType::Text,
    };

    CommandArgument {
        name: string(object, "name"),
        argument_type,
        placeholder: string(object, "placeholder"),
        required: object
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        data: (declared == "dropdown").then(|| options(object)),
    }
}

/// `"30s"`, `"5m"`, `"2h"`, `"1d"` — in seconds.
///
/// # Errors
///
/// A message naming what was wrong, as the C++'s does.
pub fn parse_interval(text: &str) -> Result<std::time::Duration, String> {
    if text.is_empty() {
        return Err("interval is empty".to_owned());
    }

    let (digits, unit) = text.split_at(text.len() - 1);
    let value: u64 = digits
        .parse()
        .map_err(|_| format!("invalid interval format: {text}"))?;

    let seconds = match unit {
        "s" => value,
        "m" => value * 60,
        "h" => value * 3600,
        "d" => value * 86400,
        other => return Err(format!("unknown interval unit: {other}")),
    };

    if seconds < MINIMUM_INTERVAL_SECS {
        return Err(format!(
            "interval must be at least {MINIMUM_INTERVAL_SECS}s, got {seconds}s"
        ));
    }

    Ok(std::time::Duration::from_secs(seconds))
}

/// Reads one command object, or `None` if its mode is not supported.
#[must_use]
pub fn parse_command(object: &Value, directory: &Path, provenance: Provenance) -> Option<Command> {
    let mode = CommandMode::parse(&string(object, "mode"))?;
    let name = string(object, "name");

    let interval = (mode == CommandMode::NoView)
        .then(|| object.get("interval").and_then(Value::as_str))
        .flatten()
        // A malformed interval is dropped, not fatal: the C++ logs and moves
        // on, and the command still loads without a schedule.
        .and_then(|text| parse_interval(text).ok());

    Some(Command {
        entrypoint: directory.join(format!("{name}.js")),
        name,
        title: string(object, "title"),
        description: string(object, "description"),
        mode,
        preferences: object
            .get("preferences")
            .and_then(Value::as_array)
            .map(|items| items.iter().map(parse_preference).collect())
            .unwrap_or_default(),
        keywords: string_array(object, "keywords"),
        arguments: object
            .get("arguments")
            .and_then(Value::as_array)
            .map(|items| items.iter().map(parse_argument).collect())
            .unwrap_or_default(),
        icon: object
            .get("icon")
            .map(|icon| icon.as_str().unwrap_or_default().to_owned()),
        interval,
        default_disabled: object
            .get("disabledByDefault")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        provenance,
    })
}

impl ExtensionManifest {
    /// Reads the `package.json` in `directory`.
    ///
    /// # Errors
    ///
    /// [`Error::Missing`] when there is none, [`Error::Read`] when it cannot
    /// be opened and [`Error::Parse`] when it is not JSON.
    pub fn from_directory(directory: &Path) -> Result<Self, Error> {
        let path = directory.join("package.json");
        if !path.exists() {
            return Err(Error::Missing(path));
        }

        let text = std::fs::read_to_string(&path).map_err(|source| Error::Read {
            path: path.clone(),
            source,
        })?;

        let json: Value = serde_json::from_str(&text).map_err(|_| Error::Parse(path.clone()))?;
        Ok(Self::from_json(&json, directory))
    }

    /// Reads a parsed `package.json` for an extension in `directory`.
    #[must_use]
    pub fn from_json(json: &Value, directory: &Path) -> Self {
        let id = directory
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
        let provenance = Provenance::from_id(&id);

        let needs_raycast_api = json
            .get("dependencies")
            .and_then(Value::as_object)
            .is_some_and(|deps| deps.contains_key(RAYCAST_NPM_API_PACKAGE));

        Self {
            path: directory.to_path_buf(),
            id,
            name: string(json, "name"),
            title: string(json, "title"),
            description: string(json, "description"),
            icon: string(json, "icon"),
            author: string(json, "author"),
            categories: string_array(json, "categories"),
            preferences: json
                .get("preferences")
                .and_then(Value::as_array)
                .map(|items| items.iter().map(parse_preference).collect())
                .unwrap_or_default(),
            commands: json
                .get("commands")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| parse_command(item, directory, provenance))
                        .collect()
                })
                .unwrap_or_default(),
            needs_raycast_api,
            provenance,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(json: &str) -> ExtensionManifest {
        let value: Value = serde_json::from_str(json).expect("the fixture is JSON");
        ExtensionManifest::from_json(&value, Path::new("/ext/store.raycast.hackernews"))
    }

    #[test]
    fn the_id_is_the_directory_and_the_provenance_follows_it() {
        // Not a field of the file: an extension installed from a store lives
        // in a directory named after it.
        assert_eq!(
            Provenance::from_id("store.vicinae.clock"),
            Provenance::Vicinae
        );
        assert_eq!(Provenance::from_id("store.raycast.hn"), Provenance::Raycast);
        assert_eq!(Provenance::from_id("my-extension"), Provenance::Local);
        assert_eq!(
            Provenance::from_id("store.raycast"),
            Provenance::Local,
            "the prefix includes the trailing dot"
        );

        let parsed = manifest(r#"{"name": "hn"}"#);
        assert_eq!(parsed.id, "store.raycast.hackernews");
        assert_eq!(parsed.provenance, Provenance::Raycast);
    }

    #[test]
    fn a_command_with_an_unknown_mode_disappears() {
        // `supportedModes` holds View and NoView, and the filter is silent.
        let parsed = manifest(
            r#"{"commands": [
                {"name": "a", "mode": "view"},
                {"name": "b", "mode": "menu-bar"},
                {"name": "c", "mode": "no-view"}
            ]}"#,
        );
        let names: Vec<&str> = parsed.commands.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["a", "c"]);
    }

    #[test]
    fn the_entrypoint_is_the_command_name_with_a_js_extension() {
        let parsed = manifest(r#"{"commands": [{"name": "search", "mode": "view"}]}"#);
        assert_eq!(
            parsed.commands[0].entrypoint,
            Path::new("/ext/store.raycast.hackernews/search.js")
        );
    }

    #[test]
    fn an_interval_is_read_only_for_a_no_view_command() {
        let parsed = manifest(
            r#"{"commands": [
                {"name": "a", "mode": "no-view", "interval": "10m"},
                {"name": "b", "mode": "view", "interval": "10m"}
            ]}"#,
        );
        assert_eq!(
            parsed.commands[0].interval,
            Some(std::time::Duration::from_secs(600))
        );
        assert_eq!(
            parsed.commands[1].interval, None,
            "a view command has no schedule, whatever the manifest says"
        );
    }

    #[test]
    fn a_malformed_interval_leaves_the_command_loadable() {
        // The C++ logs a warning and carries on. Refusing the command would
        // hide it over a typo in a field it does not need.
        let parsed =
            manifest(r#"{"commands": [{"name": "a", "mode": "no-view", "interval": "soon"}]}"#);
        assert_eq!(parsed.commands.len(), 1);
        assert_eq!(parsed.commands[0].interval, None);
    }

    #[test]
    fn the_interval_units_and_the_five_second_floor_are_the_cpps() {
        assert_eq!(
            parse_interval("30s").expect("30s"),
            std::time::Duration::from_secs(30)
        );
        assert_eq!(
            parse_interval("5m").expect("5m"),
            std::time::Duration::from_secs(300)
        );
        assert_eq!(
            parse_interval("2h").expect("2h"),
            std::time::Duration::from_secs(7200)
        );
        assert_eq!(
            parse_interval("1d").expect("1d"),
            std::time::Duration::from_secs(86400)
        );

        assert!(parse_interval("4s").is_err(), "under the floor");
        assert!(parse_interval("5s").is_ok(), "exactly the floor is allowed");
        assert!(parse_interval("").is_err());
        assert!(parse_interval("10x").is_err(), "unknown unit");
        assert!(parse_interval("s").is_err(), "no number");
        assert!(parse_interval("1 0s").is_err(), "not a number");
    }

    #[test]
    fn every_preference_type_is_read_and_an_unknown_one_is_named() {
        let parsed = manifest(
            r#"{"preferences": [
                {"name": "a", "type": "textfield", "title": "A", "required": true},
                {"name": "b", "type": "password"},
                {"name": "c", "type": "checkbox", "label": "Tick me"},
                {"name": "d", "type": "appPicker"},
                {"name": "e", "type": "file", "multiple": true},
                {"name": "f", "type": "directory"},
                {"name": "g", "type": "dropdown", "data": [{"title": "One", "value": "1"}]},
                {"name": "h", "type": "something-new"}
            ]}"#,
        );

        let kinds: Vec<&PreferenceKind> = parsed.preferences.iter().map(|p| &p.kind).collect();
        assert_eq!(kinds[0], &PreferenceKind::TextField);
        assert_eq!(kinds[1], &PreferenceKind::Password);
        assert_eq!(
            kinds[2],
            &PreferenceKind::Checkbox {
                label: "Tick me".to_owned()
            }
        );
        assert_eq!(kinds[3], &PreferenceKind::AppPicker);
        assert_eq!(kinds[4], &PreferenceKind::FilePicker { multiple: true });
        assert_eq!(
            kinds[5],
            &PreferenceKind::DirectoryPicker { multiple: false },
            "absent `multiple` is false"
        );
        assert_eq!(
            kinds[6],
            &PreferenceKind::Dropdown {
                options: vec![DropdownOption {
                    title: "One".to_owned(),
                    value: "1".to_owned()
                }]
            }
        );
        assert_eq!(
            kinds[7],
            &PreferenceKind::Unknown {
                declared: "something-new".to_owned()
            },
            "an unknown type is carried, not silently turned into a text field"
        );

        assert!(parsed.preferences[0].required);
        assert!(
            !parsed.preferences[1].required,
            "absent `required` is false"
        );
    }

    #[test]
    fn an_argument_of_an_unknown_type_is_text() {
        // The C++ writes `if (type == "text") arg.type = Text;` with no else,
        // so an unrecognised type leaves the enum's first value.
        let parsed = manifest(
            r#"{"commands": [{"name": "a", "mode": "view", "arguments": [
                {"name": "q", "type": "text"},
                {"name": "p", "type": "password"},
                {"name": "d", "type": "dropdown", "data": [{"title": "T", "value": "v"}]},
                {"name": "x", "type": "quantum"}
            ]}]}"#,
        );
        let arguments = &parsed.commands[0].arguments;
        assert_eq!(arguments[0].argument_type, ArgumentType::Text);
        assert_eq!(arguments[1].argument_type, ArgumentType::Password);
        assert_eq!(arguments[2].argument_type, ArgumentType::Dropdown);
        assert_eq!(arguments[3].argument_type, ArgumentType::Text);

        assert!(
            arguments[0].data.is_none(),
            "only a dropdown carries options"
        );
        assert_eq!(arguments[2].data.as_ref().expect("options").len(), 1);
    }

    #[test]
    fn the_raycast_dependency_is_what_sets_needs_raycast_api() {
        let with = manifest(r#"{"dependencies": {"@raycast/api": "^1.0.0"}}"#);
        assert!(with.needs_raycast_api);

        let without = manifest(r#"{"dependencies": {"@vicinae/api": "^1.0.0"}}"#);
        assert!(!without.needs_raycast_api);

        let none = manifest(r#"{}"#);
        assert!(!none.needs_raycast_api);
    }

    #[test]
    fn a_missing_package_json_is_named_in_the_error() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        match ExtensionManifest::from_directory(dir.path()) {
            Err(Error::Missing(path)) => {
                assert!(path.ends_with("package.json"), "{}", path.display());
            }
            other => panic!("expected a Missing error, got {other:?}"),
        }
    }

    #[test]
    fn a_manifest_that_is_not_json_is_an_error_rather_than_an_empty_extension() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        std::fs::write(dir.path().join("package.json"), "{not json").expect("write");
        assert!(matches!(
            ExtensionManifest::from_directory(dir.path()),
            Err(Error::Parse(_))
        ));
    }

    #[test]
    fn a_manifest_with_nothing_in_it_yields_an_extension_with_nothing_in_it() {
        // Every field is optional in the C++'s reading -- `toString()` on a
        // missing key is an empty string, not an error.
        let parsed = manifest("{}");
        assert_eq!(parsed.name, "");
        assert!(parsed.commands.is_empty());
        assert!(parsed.categories.is_empty());
        assert!(parsed.preferences.is_empty());
    }
}

/// Finding installed extensions on disk.
///
/// Ports `ExtensionRegistry::extensionDirectories` and `scanAll`
/// (`src/server/src/services/extension-registry/extension-registry.cpp`)
/// together with `Omnicast::dataSearchPaths`.
///
/// # Shadowing is by directory name, and the first one wins
///
/// The same extension can exist in several places — a user's own copy under
/// `$XDG_DATA_HOME/vicinae/extensions` and a packaged one under
/// `/usr/share/vicinae/extensions`. The C++ keeps the first it sees and logs
/// that the later one is "shadowed by extension with same directory name in
/// higher precedence directory", so the user's copy wins. Reproduced, and
/// reported rather than only logged.
pub mod registry {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use super::{Error, ExtensionManifest};

    /// The application's own data directory name under each XDG root.
    ///
    /// Still `vicinae`: it is where the C++ writes, where an installed
    /// extension already is, and renaming it would strand every extension a
    /// user has. Changing it is a migration, not a port.
    pub const DATA_DIR_NAME: &str = "vicinae";

    /// The subdirectory extensions live in.
    pub const EXTENSIONS_SUBDIR: &str = "extensions";

    /// Where extensions are looked for, highest precedence first.
    ///
    /// `$XDG_DATA_HOME/vicinae/extensions`, then each `$XDG_DATA_DIRS`
    /// entry's. A system directory that is the same path as the user one is
    /// left out, as `dataSearchPaths` leaves it out.
    #[must_use]
    pub fn search_paths_for(data_home: Option<&Path>, data_dirs: &[PathBuf]) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        let user = data_home.map(|home| home.join(DATA_DIR_NAME).join(EXTENSIONS_SUBDIR));

        if let Some(user) = &user {
            paths.push(user.clone());
        }
        for dir in data_dirs {
            let path = dir.join(DATA_DIR_NAME).join(EXTENSIONS_SUBDIR);
            if Some(&path) != user.as_ref() {
                paths.push(path);
            }
        }
        paths
    }

    /// [`search_paths_for`], from this process's environment.
    #[must_use]
    pub fn search_paths() -> Vec<PathBuf> {
        search_paths_for(
            compass_xdg::xdg_dirs::data_home().as_deref(),
            &compass_xdg::xdg_dirs::data_dirs(),
        )
    }

    /// What a scan found.
    #[derive(Debug, Default)]
    pub struct Scan {
        /// The manifests that loaded, in precedence order.
        pub extensions: Vec<ExtensionManifest>,
        /// Directories skipped because a higher-precedence one has the same
        /// name, as `(shadowed, winner)` pairs.
        pub shadowed: Vec<(PathBuf, PathBuf)>,
        /// Directories whose manifest would not load, and why.
        pub failed: Vec<(PathBuf, Error)>,
    }

    /// Scans `paths` in order, keeping the first extension of each name.
    ///
    /// A directory whose name starts with a dot is skipped — that is how the
    /// C++ ignores the `.staging-*` directories an install writes. A missing
    /// search path is not an error: most machines have only one.
    #[must_use]
    pub fn scan(paths: &[PathBuf]) -> Scan {
        let mut scan = Scan::default();
        let mut installed: BTreeMap<String, PathBuf> = BTreeMap::new();

        for root in paths {
            let Ok(entries) = std::fs::read_dir(root) else {
                continue;
            };

            let mut directories: Vec<PathBuf> = entries
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .map(|entry| entry.path())
                .collect();
            // `directory_iterator` has no defined order; sorting makes a scan
            // reproducible, which matters for the shadowing report.
            directories.sort();

            for path in directories {
                let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                if name.starts_with('.') {
                    continue;
                }

                if let Some(winner) = installed.get(name) {
                    scan.shadowed.push((path.clone(), winner.clone()));
                    continue;
                }

                match ExtensionManifest::from_directory(&path) {
                    Ok(manifest) => {
                        installed.insert(name.to_owned(), path);
                        scan.extensions.push(manifest);
                    }
                    Err(error) => scan.failed.push((path, error)),
                }
            }
        }

        scan
    }
}

#[cfg(test)]
mod registry_tests {
    use super::registry::{scan, search_paths_for};
    use std::path::{Path, PathBuf};

    /// An extension directory with a minimal manifest.
    fn install(root: &Path, name: &str, title: &str) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).expect("the extension directory");
        std::fs::write(
            dir.join("package.json"),
            format!(r#"{{"name": "{name}", "title": "{title}"}}"#),
        )
        .expect("the manifest");
        dir
    }

    #[test]
    fn the_user_directory_comes_before_the_system_ones() {
        let paths = search_paths_for(
            Some(Path::new("/home/u/.local/share")),
            &[
                PathBuf::from("/usr/share"),
                PathBuf::from("/usr/local/share"),
            ],
        );
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/home/u/.local/share/vicinae/extensions"),
                PathBuf::from("/usr/share/vicinae/extensions"),
                PathBuf::from("/usr/local/share/vicinae/extensions"),
            ]
        );
    }

    #[test]
    fn a_system_directory_that_is_the_user_one_is_not_searched_twice() {
        // `if (p != user)` in `dataSearchPaths`. Without it an extension in a
        // repeated directory would shadow itself and be reported as such.
        let paths = search_paths_for(
            Some(Path::new("/data")),
            &[PathBuf::from("/data"), PathBuf::from("/usr/share")],
        );
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/data/vicinae/extensions"),
                PathBuf::from("/usr/share/vicinae/extensions"),
            ]
        );
    }

    #[test]
    fn the_first_directory_of_a_name_wins_and_the_rest_are_reported() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let user = dir.path().join("user");
        let system = dir.path().join("system");
        let mine = install(&user, "hackernews", "Mine");
        let theirs = install(&system, "hackernews", "Packaged");

        let result = scan(&[user, system]);
        assert_eq!(result.extensions.len(), 1);
        assert_eq!(
            result.extensions[0].title, "Mine",
            "the user's copy is the one that loads"
        );
        assert_eq!(result.shadowed, vec![(theirs, mine)]);
    }

    #[test]
    fn a_dotted_directory_is_skipped_because_that_is_how_an_install_stages() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let root = dir.path().join("extensions");
        install(&root, ".staging-hackernews", "Half installed");
        install(&root, "hackernews", "Installed");

        let result = scan(&[root]);
        let titles: Vec<&str> = result.extensions.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, vec!["Installed"]);
    }

    #[test]
    fn a_directory_without_a_manifest_is_reported_rather_than_skipped() {
        // The C++ logs it as "Failed to load bundle at". Silently ignoring it
        // would leave a user wondering why their extension does not appear.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let root = dir.path().join("extensions");
        std::fs::create_dir_all(root.join("broken")).expect("a directory");
        install(&root, "working", "Works");

        let result = scan(std::slice::from_ref(&root));
        assert_eq!(result.extensions.len(), 1);
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].0, root.join("broken"));
    }

    #[test]
    fn a_search_path_that_does_not_exist_is_not_an_error() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let root = dir.path().join("extensions");
        install(&root, "one", "One");

        let result = scan(&[dir.path().join("nowhere"), root]);
        assert_eq!(result.extensions.len(), 1);
        assert!(result.failed.is_empty());
    }

    #[test]
    fn a_file_among_the_directories_is_ignored() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let root = dir.path().join("extensions");
        install(&root, "one", "One");
        std::fs::write(root.join("README"), "not an extension").expect("a file");

        let result = scan(&[root]);
        assert_eq!(result.extensions.len(), 1);
        assert!(result.failed.is_empty(), "{:?}", result.failed);
    }
}
