//! A script's `script.toml`.
//!
//! A script is a directory: the manifest, an entry file, and anything else the
//! author wants to keep next to it. As with Node extensions, the **id is the
//! directory name**, so two copies of a script in different search paths
//! shadow each other by name.
//!
//! ```toml
//! title = "Unit Converter"
//! description = "Convert lengths, weights and temperatures"
//! icon = "calculator"
//! keywords = ["convert", "units"]
//! capabilities = ["clipboard.write"]
//! # entry = "main.rhai"   (the default)
//! ```
//!
//! The manifest is read without running any script code, which is the point:
//! a host can show what a script asks for, and ask the user, before a single
//! line of it executes.

use std::path::{Component, Path, PathBuf};

use compass_extension_api::{Capability, CapabilityRegistry, ExtensionId};
use serde::Deserialize;

use crate::error::ManifestError;

/// The manifest's file name inside a script directory.
pub const MANIFEST_FILE: &str = "script.toml";

/// The entry file used when the manifest names none.
pub const DEFAULT_ENTRY: &str = "main.rhai";

/// Prefix of every script's [`ExtensionId`], so a script can never share an
/// id — and therefore a storage namespace or a grant — with a Node extension
/// that happens to have the same directory name.
pub const ID_PREFIX: &str = "script.";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    title: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    entry: Option<String>,
}

/// A parsed, validated `script.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptManifest {
    /// `script.` plus the directory name.
    pub id: ExtensionId,
    /// The script's directory.
    pub directory: PathBuf,
    /// Title shown in root search.
    pub title: String,
    /// Subtitle shown in root search.
    pub description: Option<String>,
    /// Built-in icon name.
    pub icon: Option<String>,
    /// Extra root-search terms.
    pub keywords: Vec<String>,
    /// What the script asks for, verbatim. Unknown names are kept and simply
    /// never granted, as [`CapabilityRegistry`] does for any extension.
    pub capabilities: Vec<Capability>,
    /// The entry file, inside [`directory`](Self::directory).
    pub entry: PathBuf,
}

impl ScriptManifest {
    /// Reads `script.toml` from `directory`.
    ///
    /// # Errors
    ///
    /// When the file is missing or unreadable, is not valid TOML, has a field
    /// this build does not know, or names an entry outside the directory.
    pub fn from_directory(directory: &Path) -> Result<Self, ManifestError> {
        let path = directory.join(MANIFEST_FILE);
        let text = std::fs::read_to_string(&path).map_err(|source| ManifestError::Io {
            path: path.clone(),
            source,
        })?;
        Self::parse(&text, directory).map_err(|error| match error {
            ManifestError::Invalid { message, .. } => ManifestError::Invalid { path, message },
            other => other,
        })
    }

    /// Parses manifest text as if it lived in `directory`.
    ///
    /// # Errors
    ///
    /// As [`from_directory`](Self::from_directory), minus I/O.
    pub fn parse(text: &str, directory: &Path) -> Result<Self, ManifestError> {
        let raw: RawManifest = toml::from_str(text).map_err(|error| ManifestError::Invalid {
            path: directory.join(MANIFEST_FILE),
            message: error.message().to_owned(),
        })?;
        if raw.title.trim().is_empty() {
            return Err(ManifestError::Invalid {
                path: directory.join(MANIFEST_FILE),
                message: "`title` must not be empty".to_owned(),
            });
        }

        let entry = raw.entry.unwrap_or_else(|| DEFAULT_ENTRY.to_owned());
        let relative = Path::new(&entry);
        let contained = !entry.is_empty()
            && relative
                .components()
                .all(|component| matches!(component, Component::Normal(_)));
        if !contained {
            return Err(ManifestError::EntryOutsideDirectory { entry });
        }

        let name = directory
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();

        Ok(Self {
            id: ExtensionId::new(format!("{ID_PREFIX}{name}")),
            directory: directory.to_path_buf(),
            title: raw.title,
            description: raw.description.filter(|d| !d.is_empty()),
            icon: raw.icon.filter(|i| !i.is_empty()),
            keywords: raw.keywords,
            capabilities: raw.capabilities.into_iter().map(Capability::new).collect(),
            entry: directory.join(relative),
        })
    }

    /// Records this script's declarations in `registry`, replacing earlier
    /// ones. Declaring is not granting: the host still decides what to grant.
    pub fn declare_into(&self, registry: &mut CapabilityRegistry) {
        registry.declare(&self.id, self.capabilities.iter().cloned());
    }

    /// Reads the entry file.
    ///
    /// # Errors
    ///
    /// When the file cannot be read.
    pub fn read_source(&self) -> std::io::Result<String> {
        std::fs::read_to_string(&self.entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minimal_manifest_takes_its_id_from_the_directory() {
        let manifest = ScriptManifest::parse("title = \"Hi\"", Path::new("/s/hello")).unwrap();
        assert_eq!(manifest.id.as_str(), "script.hello");
        assert_eq!(manifest.entry, Path::new("/s/hello/main.rhai"));
        assert!(manifest.capabilities.is_empty());
    }

    #[test]
    fn an_entry_cannot_leave_the_directory() {
        for entry in ["../x.rhai", "/etc/passwd", "a/../../b.rhai", ""] {
            let text = format!("title = \"x\"\nentry = \"{entry}\"");
            assert!(
                matches!(
                    ScriptManifest::parse(&text, Path::new("/s/x")),
                    Err(ManifestError::EntryOutsideDirectory { .. })
                ),
                "{entry}"
            );
        }
        let nested =
            ScriptManifest::parse("title = \"x\"\nentry = \"src/a.rhai\"", Path::new("/s/x"));
        assert_eq!(nested.unwrap().entry, Path::new("/s/x/src/a.rhai"));
    }

    #[test]
    fn unknown_fields_and_empty_titles_are_rejected() {
        assert!(ScriptManifest::parse("title = \"x\"\nnet = true", Path::new("/s/x")).is_err());
        assert!(ScriptManifest::parse("title = \" \"", Path::new("/s/x")).is_err());
    }
}
