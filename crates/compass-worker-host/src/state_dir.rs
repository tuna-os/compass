//! Per-extension durable state directory.
//!
//! `base` is `~/.local/state/compass` (Flatpak-mapped `~/.var/app/.../state`).
//! `extension_id` is the registered id. Directory is created with default
//! permissions when absent; existing permissions are left untouched.

use std::path::{Path, PathBuf};

#[must_use]
pub fn state_dir(base: &Path, extension_id: &str) -> PathBuf {
    let path = base.join("extensions").join(extension_id);
    let _ = std::fs::create_dir_all(&path);
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_dir_is_extensions_slash_id_and_is_created() {
        let base = tempfile::tempdir().expect("tempdir");
        let path = state_dir(base.path(), "com.example.clock");
        assert_eq!(path, base.path().join("extensions/com.example.clock"));
        assert!(path.is_dir(), "state_dir must create the directory");
        let again = state_dir(base.path(), "com.example.clock");
        assert_eq!(again, path);
    }
}
