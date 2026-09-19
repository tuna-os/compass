//! The extension boilerplate generator: what "Create Extension" actually
//! writes to disk.
//!
//! A port of `ExtensionBoilerplateGenerator`
//! (`src/server/src/services/extension-boilerplate-generator/`).
//!
//! # The manifest is templated as text, not as JSON
//!
//! The C++ says so in a comment, and it is not an accident: round-tripping
//! through a JSON object would reorder the keys, and the file the generator
//! writes is the first thing a person reads after creating an extension. So
//! `%NAME%`-style placeholders are substituted into the template's own text
//! and the key order survives. This port does the same, for the same reason —
//! a `serde_json` round-trip here would be a regression that no test of the
//! parsed value could see.
//!
//! # The templates are embedded, as they are in Qt
//!
//! The C++ reads `:boilerplate/...`, which is the Qt resource system serving
//! bytes compiled into the binary from `extra/extension-boilerplate/`. The
//! `include_str!`/`include_bytes!` below are the same files, compiled in the
//! same way, so the generated extension is byte-identical to the C++'s.

use std::fs;
use std::path::{Path, PathBuf};

use crate::slug::slugify;

/// How a command presents itself: with a view, or without one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandMode {
    /// A command that renders a view.
    View,
    /// A command that runs and exits without rendering.
    NoView,
}

impl CommandMode {
    /// The `mode` string this appears as in an extension manifest.
    #[must_use]
    pub const fn manifest_value(self) -> &'static str {
        match self {
            Self::View => "view",
            Self::NoView => "no-view",
        }
    }

    /// The source extension a command of this mode is written with.
    ///
    /// A view command is TSX because it returns JSX; a no-view one has no
    /// markup in it and is plain TS.
    #[must_use]
    pub const fn source_extension(self) -> &'static str {
        match self {
            Self::View => "tsx",
            Self::NoView => "ts",
        }
    }
}

/// One template a new command can be started from.
#[derive(Debug, Clone, Copy)]
pub struct CommandBoilerplate {
    /// The id the form passes back to identify this template.
    pub resource: &'static str,
    /// What the template is called in the picker.
    pub name: &'static str,
    /// The mode a command made from it runs in.
    pub mode: CommandMode,
    /// The template's source, embedded.
    pub source: &'static str,
}

/// The template that starts a plain list.
const TMPL_LIST: &str = include_str!("../../../extra/extension-boilerplate/src/list.tsx");
/// The template that starts a list with a detail pane.
const TMPL_LIST_DETAIL: &str =
    include_str!("../../../extra/extension-boilerplate/src/list-detail.tsx");
/// The template that starts a list whose search text is state.
const TMPL_CONTROLLED_LIST: &str =
    include_str!("../../../extra/extension-boilerplate/src/controlled-list.tsx");
/// The template that starts a detail view.
const TMPL_SIMPLE_DETAIL: &str =
    include_str!("../../../extra/extension-boilerplate/src/simple-detail.tsx");
/// The template that starts a command with no view.
const TMPL_NO_VIEW: &str = include_str!("../../../extra/extension-boilerplate/src/no-view.tsx");

/// The manifest template, placeholders and all.
const PACKAGE_JSON: &str = include_str!("../../../extra/extension-boilerplate/assets/package.json");
/// The TypeScript configuration a new extension is given.
const TSCONFIG_JSON: &str = include_str!("../../../extra/extension-boilerplate/tsconfig.json");
/// The README a new extension is given.
const README_MD: &str = include_str!("../../../extra/extension-boilerplate/assets/README.md");
/// The placeholder icon a new extension is given.
const EXTENSION_ICON: &[u8] =
    include_bytes!("../../../extra/extension-boilerplate/assets/extension_icon.png");

/// Every template a command can be started from, in the order the picker
/// offers them.
pub const COMMAND_BOILERPLATES: &[CommandBoilerplate] = &[
    CommandBoilerplate {
        resource: ":boilerplate/tmpl-list",
        name: "Simple List",
        mode: CommandMode::View,
        source: TMPL_LIST,
    },
    CommandBoilerplate {
        resource: ":boilerplate/tmpl-list-detail",
        name: "List with Detail",
        mode: CommandMode::View,
        source: TMPL_LIST_DETAIL,
    },
    CommandBoilerplate {
        resource: ":boilerplate/tmpl-controlled-list",
        name: "Controlled List",
        mode: CommandMode::View,
        source: TMPL_CONTROLLED_LIST,
    },
    CommandBoilerplate {
        resource: ":boilerplate/tmpl-simple-detail",
        name: "Simple Detail",
        mode: CommandMode::View,
        source: TMPL_SIMPLE_DETAIL,
    },
    CommandBoilerplate {
        resource: ":boilerplate/tmpl-no-view",
        name: "No View",
        mode: CommandMode::NoView,
        source: TMPL_NO_VIEW,
    },
];

/// Find the template with `resource` as its id.
#[must_use]
pub fn boilerplate_by_resource(resource: &str) -> Option<&'static CommandBoilerplate> {
    COMMAND_BOILERPLATES
        .iter()
        .find(|tmpl| tmpl.resource == resource)
}

/// One command the new extension is created with.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandConfig {
    /// The command's title; its name is this, slugified.
    pub title: String,
    /// What the command does.
    pub description: String,
    /// Which [`CommandBoilerplate`] to start it from, by `resource`.
    pub template_id: String,
}

/// Everything the generator needs to write an extension.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Config {
    /// Who is writing it.
    pub author: String,
    /// The extension's title; its directory is this, slugified.
    pub title: String,
    /// What the extension does.
    pub description: String,
    /// The commands to create it with. An empty list is allowed.
    pub commands: Vec<CommandConfig>,
}

/// The one JSON object per command, as text so key order survives.
const COMMAND_JSON_TEMPLATE: &str = r#"    {
      "name": "%NAME%",
      "title": "%TITLE%",
      "description": "%DESCRIPTION%",
      "mode": "%MODE%"
    }"#;

/// Qt's `QString::simplified`: trim, and collapse internal whitespace runs to
/// a single space.
///
/// Every value substituted into the manifest goes through this, so a title
/// pasted with a stray newline in it cannot break the JSON it lands in.
#[must_use]
pub fn simplified(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The `@vicinae/api` dependency a generated extension is pinned to.
///
/// A release tag `vX.Y.Z` becomes the caret range `^X.Y.Z`; anything else
/// becomes `latest`. Every real build has a valid tag, so the fallback is for
/// a working tree built off a tag-less checkout.
#[must_use]
pub fn api_dependency_version(git_tag: &str) -> String {
    let is_valid = git_tag.starts_with('v') && git_tag[1..].split('.').count() == 3;
    if is_valid {
        format!("^{}", &git_tag[1..])
    } else {
        "latest".to_string()
    }
}

/// Render the `commands` array of the manifest for `commands`.
fn render_command_list(entries: &[String]) -> String {
    format!("[\n{}\n  ]", entries.join(",\n    "))
}

/// Write a template file, and leave it readable and writable by its owner
/// only.
///
/// Qt's `QFile::copy` refuses to overwrite, so a destination that already
/// exists is left alone rather than replaced — which is what happens when two
/// commands in one config slugify to the same name.
fn user_copy(destination: &Path, contents: &[u8]) -> std::io::Result<()> {
    if destination.exists() {
        return Ok(());
    }
    fs::write(destination, contents)?;
    set_owner_only(destination)
}

#[cfg(unix)]
/// Set `path` to owner read/write, as `QFileDevice::ReadOwner | WriteOwner`.
fn set_owner_only(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
/// Permissions are not modelled this way off unix; leave the file as written.
fn set_owner_only(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Generate an extension under `target_dir`, returning the directory created.
///
/// `git_tag` is the build's version, used to pin `@vicinae/api`.
///
/// # Errors
///
/// Returns the message the form shows when `target_dir` is not a directory,
/// when the extension directory already exists, when a command names a
/// template that does not exist, or when a file cannot be written.
pub fn generate(target_dir: &Path, config: &Config, git_tag: &str) -> Result<PathBuf, String> {
    let ext_name = slugify(&config.title);

    if !target_dir.is_dir() {
        return Err(format!(
            "{} is not a directory. The boilerplate generator will not create the containing directory for you",
            target_dir.display()
        ));
    }

    let ext_dir = target_dir.join(&ext_name);

    if ext_dir.exists() {
        return Err(format!(
            "{} already exists. Won't override.",
            ext_dir.display()
        ));
    }

    let src_dir = ext_dir.join("src");
    let assets_dir = ext_dir.join("assets");
    fs::create_dir_all(&src_dir).map_err(|err| err.to_string())?;
    fs::create_dir_all(&assets_dir).map_err(|err| err.to_string())?;

    let mut command_entries = Vec::with_capacity(config.commands.len());

    for command in &config.commands {
        let name = slugify(&command.title);
        let Some(tmpl) = boilerplate_by_resource(&command.template_id) else {
            return Err(format!("Unknown template with id {}", command.template_id));
        };

        let entry = COMMAND_JSON_TEMPLATE
            .replace("%NAME%", &simplified(&name))
            .replace("%TITLE%", &simplified(&command.title))
            .replace("%DESCRIPTION%", &simplified(&command.description))
            .replace("%MODE%", tmpl.mode.manifest_value());

        let filename = format!("{name}.{}", tmpl.mode.source_extension());
        user_copy(&src_dir.join(filename), tmpl.source.as_bytes())
            .map_err(|err| err.to_string())?;
        command_entries.push(entry);
    }

    let manifest = PACKAGE_JSON
        .replace("%NAME%", &ext_name)
        .replace("%TITLE%", &simplified(&config.title))
        .replace("%DESCRIPTION%", &simplified(&config.description))
        .replace("%AUTHOR%", &simplified(&config.author))
        .replace("%VICINAE_VERSION%", &api_dependency_version(git_tag))
        .replace("%COMMAND_LIST%", &render_command_list(&command_entries));

    fs::write(ext_dir.join("package.json"), manifest.as_bytes())
        .map_err(|_| "Failed to write manifest".to_string())?;
    fs::write(
        ext_dir.join(".gitignore"),
        b"node_modules\nvicinae-env.d.ts\n",
    )
    .map_err(|_| "Failed to write gitignore".to_string())?;

    user_copy(&ext_dir.join("tsconfig.json"), TSCONFIG_JSON.as_bytes())
        .map_err(|err| err.to_string())?;
    user_copy(&assets_dir.join("extension_icon.png"), EXTENSION_ICON)
        .map_err(|err| err.to_string())?;
    user_copy(&ext_dir.join("README.md"), README_MD.as_bytes()).map_err(|err| err.to_string())?;

    Ok(ext_dir)
}
