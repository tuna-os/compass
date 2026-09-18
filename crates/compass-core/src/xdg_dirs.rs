//! The handful of XDG base-directory lookups the core needs.
//!
//! Everything here reads process environment variables. Nothing else in this crate does, so a
//! test that avoids these functions cannot accidentally touch the invoking user's home directory.

use std::path::{Path, PathBuf};

/// `/usr/local/share:/usr/share`, the specified fallback for an unset `$XDG_DATA_DIRS`.
pub const DEFAULT_DATA_DIRS: &str = "/usr/local/share:/usr/share";

/// The application directories, in precedence order.
///
/// DELEGATES rather than reimplementing, and that is the fix for a class of bug rather than a
/// style preference. This function used to be a character-for-character copy of
/// [`compass_xdg::application_dirs`], with `compass-xdg` owning the matching `icon_dirs`. So the
/// applications the launcher indexes and the icons it draws for them were resolved by two
/// separate copies of the same logic, and #95 -- a Flatpak sandbox exposing host data under
/// paths neither copy knew about -- had to be fixed in both or the launcher would have found
/// applications and then drawn none of their icons.
#[must_use]
pub fn application_dirs() -> Vec<PathBuf> {
    compass_xdg::application_dirs()
}

/// The extra data roots a Flatpak sandbox needs, with its inputs supplied explicitly.
///
/// Re-exported rather than having `vicinae` depend on `compass-xdg` directly: `compass-core` is
/// already the seam that crate sits behind, and `vicinae doctor` needs this to report the
/// directories the index really searches.
#[must_use]
pub fn sandbox_data_roots_for(in_flatpak: bool, home: Option<&Path>) -> Vec<PathBuf> {
    compass_xdg::sandbox_data_roots_for(in_flatpak, home)
}

/// `$XDG_DATA_HOME`, falling back to `~/.local/share`.
#[must_use]
pub fn data_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_DATA_HOME") {
        Some(value) if !value.is_empty() => Some(PathBuf::from(value)),
        _ => dirs::data_dir(),
    }
}

/// `$XDG_DATA_DIRS`, falling back to [`DEFAULT_DATA_DIRS`].
#[must_use]
pub fn data_dirs() -> Vec<PathBuf> {
    let raw = match std::env::var("XDG_DATA_DIRS") {
        Ok(value) if !value.is_empty() => value,
        _ => DEFAULT_DATA_DIRS.to_owned(),
    };

    raw.split(':')
        .filter(|part| !part.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// The desktop names in `$XDG_CURRENT_DESKTOP`, e.g. `["GNOME"]`.
#[must_use]
pub fn current_desktops() -> Vec<String> {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The directories in `$PATH`.
#[must_use]
pub fn exec_search_path() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default()
}
