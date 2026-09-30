//! The curated per-extension overrides for Raycast extensions written for
//! macOS: `extensions/raycast-linux-overrides.json`.
//!
//! The runtime's shim (`src/typescript/extension-manager/src/linux-shim/`)
//! fixes what every macOS extension gets wrong the same way: a Homebrew
//! prefix, `open`, `pbcopy`, `osascript`. What it cannot fix generically is
//! written down here, one entry per extension, and read by both halves: the
//! runtime bundles the file for its path and command maps and its load-time
//! patches, and the engine reads it for what only the engine may decide —
//! which host programs an extension may ask the person to run, and which
//! extensions are replaced by a Linux-capable one at install time.
//!
//! `docs/rust-engine/RAYCAST-LINUX-SHIM.md` documents the format and how to
//! add an entry.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The manifest as it is in the repository.
pub const MANIFEST: &str = include_str!("../../../extensions/raycast-linux-overrides.json");

/// The whole manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    /// The format's version; 1.
    pub version: u32,
    /// Programs every extension may ask to run on the host.
    #[serde(default)]
    pub host_programs: Vec<String>,
    /// The entries, by installed extension id (`store.raycast.<name>`).
    #[serde(default)]
    pub extensions: BTreeMap<String, Entry>,
}

/// One extension's overrides.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Entry {
    /// Why the entry exists: what the extension does on macOS that needs it.
    pub why: String,
    /// More programs this extension may ask to run on the host.
    #[serde(default)]
    pub host_programs: Vec<String>,
    /// Path prefixes the runtime maps, macOS to Linux (`~` is the home
    /// directory). Read by the runtime only.
    #[serde(default)]
    pub paths: BTreeMap<String, String>,
    /// Commands the runtime runs in place of others. Read by the runtime only.
    #[serde(default)]
    pub commands: BTreeMap<String, String>,
    /// Source patches the runtime applies when it loads the extension. Read by
    /// the runtime only.
    #[serde(default)]
    pub patches: Vec<Patch>,
    /// A Linux-capable extension installed in this one's place.
    #[serde(default)]
    pub redirect: Option<Redirect>,
}

/// A literal replacement in one of the extension's bundled files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Patch {
    /// The file, relative to the extension's directory (`installed.js`).
    pub file: String,
    /// The exact text to find.
    pub find: String,
    /// What replaces each occurrence.
    pub replace: String,
    /// Why.
    pub why: String,
}

/// Where an extension is replaced from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Redirect {
    /// The store: `vicinae`.
    pub store: String,
    /// The replacement's author handle in that store.
    pub author: String,
    /// Its name in that store.
    pub name: String,
    /// Why the original cannot be made to work.
    pub why: String,
}

impl Manifest {
    /// Reads a manifest.
    ///
    /// # Errors
    ///
    /// When the text is not a manifest of a version this build knows.
    pub fn parse(text: &str) -> Result<Self, String> {
        let manifest: Self = serde_json::from_str(text).map_err(|err| err.to_string())?;
        if manifest.version != 1 {
            return Err(format!(
                "version {} of the overrides manifest is not one this build reads",
                manifest.version
            ));
        }
        Ok(manifest)
    }

    /// The manifest this build ships.
    ///
    /// # Panics
    ///
    /// Never in a build that passed its tests: the bundled manifest is parsed
    /// by `the_shipped_manifest_parses`.
    #[must_use]
    pub fn shipped() -> &'static Self {
        static SHIPPED: std::sync::LazyLock<Manifest> = std::sync::LazyLock::new(|| {
            Manifest::parse(MANIFEST).expect("the shipped overrides manifest parses")
        });
        &SHIPPED
    }

    /// Whether `extension_id` may ask to run `program` on the host.
    #[must_use]
    pub fn allows_host_program(&self, extension_id: &str, program: &str) -> bool {
        self.host_programs.iter().any(|p| p == program)
            || self
                .extensions
                .get(extension_id)
                .is_some_and(|entry| entry.host_programs.iter().any(|p| p == program))
    }

    /// What a Raycast store extension called `name` is replaced with.
    #[must_use]
    pub fn raycast_redirect(&self, name: &str) -> Option<&Redirect> {
        self.extensions
            .get(&format!("{}{name}", crate::raycast_store::ID_PREFIX))?
            .redirect
            .as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_manifest_parses() {
        let manifest = Manifest::parse(MANIFEST).expect("parses");
        assert!(manifest.host_programs.iter().any(|p| p == "brew"));
        for (id, entry) in &manifest.extensions {
            assert!(
                id.starts_with(crate::raycast_store::ID_PREFIX),
                "{id} is not a Raycast store id"
            );
            assert!(!entry.why.trim().is_empty(), "{id} says why");
            for patch in &entry.patches {
                assert!(!patch.find.is_empty(), "{id}: a patch finds something");
                assert!(
                    !patch.file.contains("..") && !patch.file.starts_with('/'),
                    "{id}: a patch stays in the extension"
                );
            }
        }
    }

    #[test]
    fn brew_is_brokered_for_everyone_and_nothing_else_is() {
        let manifest = Manifest::shipped();
        assert!(manifest.allows_host_program("store.raycast.brew", "brew"));
        assert!(manifest.allows_host_program("store.vicinae.linuxbrew", "brew"));
        assert!(!manifest.allows_host_program("store.raycast.brew", "sh"));
    }

    #[test]
    fn an_entry_can_add_a_program_and_redirect() {
        let manifest = Manifest::parse(
            r#"{ "version": 1, "extensions": { "store.raycast.x": {
                "why": "w", "hostPrograms": ["op"],
                "redirect": { "store": "vicinae", "author": "a", "name": "y", "why": "w" } } } }"#,
        )
        .expect("parses");
        assert!(manifest.allows_host_program("store.raycast.x", "op"));
        assert!(!manifest.allows_host_program("store.raycast.z", "op"));
        assert_eq!(
            manifest.raycast_redirect("x").map(|r| r.name.as_str()),
            Some("y")
        );
        assert_eq!(manifest.raycast_redirect("z"), None);
    }

    #[test]
    fn an_unknown_field_or_version_is_refused() {
        assert!(Manifest::parse(r#"{ "version": 2 }"#).is_err());
        assert!(
            Manifest::parse(r#"{ "version": 1, "extensions": { "store.raycast.x": { "why": "w", "hostProgram": [] } } }"#)
                .is_err(),
            "a misspelt field would otherwise be ignored"
        );
    }
}
