//! What a builtin view remembers between openings: Browse Fonts' category
//! (`fontCategory`) and Search Files' (`fileCategory`), which the C++ keeps
//! in the command's local storage.
//!
//! Here it is a small JSON object in the state directory, owned by the
//! launcher process that reads and writes it. The command's local storage is
//! the engine's encrypted database, which needs the login keyring; a filter
//! the user chose is not a secret, and it should not stop being remembered
//! on a machine without one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The file's name in the state directory.
pub const FILE_NAME: &str = "compass-view-state.json";

/// Browse Fonts' category, as `font_browser::STORAGE_KEY` names it.
pub const FONT_CATEGORY: &str = "font.fontCategory";

/// Search Files' category.
pub const FILE_CATEGORY: &str = "files.fileCategory";

/// The remembered values, and where they are kept.
#[derive(Debug, Clone, Default)]
pub struct ViewMemory {
    path: Option<PathBuf>,
    values: BTreeMap<String, String>,
}

/// `$XDG_STATE_HOME/compass/<FILE_NAME>`, falling back to
/// `~/.local/state`.
#[must_use]
pub fn default_path() -> Option<PathBuf> {
    let state = match std::env::var_os("XDG_STATE_HOME") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => compass_core::xdg_dirs::home_dir()?.join(".local/state"),
    };
    Some(state.join("compass").join(FILE_NAME))
}

impl ViewMemory {
    /// Reads what `path` holds; nothing when it is missing or unreadable.
    /// `None` keeps the values in memory only, as tests do.
    #[must_use]
    pub fn load(path: Option<PathBuf>) -> Self {
        let values = path
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self { path, values }
    }

    /// A remembered value.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// Remembers `value` under `key`, writing the file when there is one.
    pub fn set(&mut self, key: &str, value: &str) {
        if self.get(key) == Some(value) {
            return;
        }
        self.values.insert(key.to_owned(), value.to_owned());
        if let Some(path) = &self.path
            && let Err(error) = write(path, &self.values)
        {
            tracing::warn!(%error, path = %path.display(), "could not remember a view's filter");
        }
    }
}

fn write(path: &Path, values: &BTreeMap<String, String>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(values).map_err(std::io::Error::other)?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, text)?;
    std::fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_survives_a_new_process() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/compass").join(FILE_NAME);
        let mut memory = ViewMemory::load(Some(path.clone()));
        assert_eq!(memory.get(FONT_CATEGORY), None);
        memory.set(FONT_CATEGORY, "Thai");
        let reread = ViewMemory::load(Some(path));
        assert_eq!(reread.get(FONT_CATEGORY), Some("Thai"));

        let mut transient = ViewMemory::load(None);
        transient.set(FILE_CATEGORY, "Images");
        assert_eq!(transient.get(FILE_CATEGORY), Some("Images"));
    }
}
