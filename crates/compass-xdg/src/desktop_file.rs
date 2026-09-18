//! Desktop file identifiers, and finding an entry by one.
//!
//! Ports `src/lib/xdgpp/xdgpp/desktop-entry/file.cpp` — the `DesktopFile`
//! layer that sits above [`crate::entry`], giving an entry an *id* and looking
//! one up by that id.
//!
//! # Two id schemes, and they disagree
//!
//! The XDG Desktop Entry Specification says a desktop file ID is the path
//! below the applications directory with `/` turned into `-`. The C++ turns it
//! into `.` instead. For a file directly in the directory the two agree; for
//! anything nested they do not — `kde4/konsole.desktop` is
//! `kde4-konsole.desktop` by the specification and `kde4.konsole.desktop` by
//! the C++.
//!
//! That is not cosmetic. The id is the key an application's frecency score,
//! alias, and enabled state are stored under, so the two engines would not
//! find each other's records for any nested application.
//!
//! Both are provided here, named for what they are, and
//! [`crate::scan::desktop_file_id`] is the specification's. Which one the
//! engine should key on is a decision about migrating stored data, not one to
//! be made by whichever function a caller reached for first.

use std::path::{Path, PathBuf};

/// The suffix every desktop entry file carries.
pub const DESKTOP_SUFFIX: &str = ".desktop";

/// The C++'s desktop file id: the relative path with separators as dots.
///
/// The `.desktop` suffix is **kept**, so the id of `firefox.desktop` is
/// `firefox.desktop` and not `firefox`. That is worth knowing because it makes
/// the suffix indistinguishable from a separator: an id of
/// `kde4.konsole.desktop` could be a nested `konsole` or a flat file named
/// `kde4.konsole.desktop`, and nothing recovers the difference.
///
/// A path outside the directory keeps whatever `..` components the relative
/// path needs, exactly as the C++'s `lexically_relative` produces them.
#[must_use]
pub fn relative_id_dotted(file: &Path, app_dir: &Path) -> String {
    lexically_relative(file, app_dir).replace('/', ".")
}

/// The id of a file with no directory to be relative to.
///
/// Just the filename. This is what `fromFile` uses when no application
/// directory is given, so a file loaded by path alone is identified by its
/// name and two such files in different directories collide.
#[must_use]
pub fn standalone_id(file: &Path) -> String {
    file.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The relative path, the way `std::filesystem::lexically_relative` computes
/// it: purely textually, with `..` for each component of the base that the
/// path does not share.
fn lexically_relative(path: &Path, base: &Path) -> String {
    let path_parts: Vec<&str> = split_components(path);
    let base_parts: Vec<&str> = split_components(base);

    let common = path_parts
        .iter()
        .zip(base_parts.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let mut out: Vec<&str> = Vec::new();
    out.extend(std::iter::repeat_n("..", base_parts.len() - common));
    out.extend(path_parts[common..].iter().copied());
    out.join("/")
}

fn split_components(path: &Path) -> Vec<&str> {
    path.to_str()
        .unwrap_or_default()
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect()
}

/// The two paths a lookup by id tries, in order.
///
/// The id as given first, then the id with `.desktop` appended. That order is
/// what lets `fromId` accept both `firefox` and `firefox.desktop` — and it
/// means an id that already ends in `.desktop` is never given a second suffix,
/// because the first candidate matches.
#[must_use]
pub fn lookup_candidates(dir: &Path, id: &str) -> Vec<PathBuf> {
    vec![dir.join(id), dir.join(format!("{id}{DESKTOP_SUFFIX}"))]
}

/// Find the file an id names, searching directories in order.
///
/// Both candidates are tried in **one** directory before moving to the next,
/// so a `firefox.desktop` in an earlier directory wins over a `firefox` in a
/// later one. That is what makes the search order a precedence: a user's own
/// applications directory shadows the system's.
#[must_use]
pub fn resolve_id(id: &str, dirs: &[PathBuf], exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    for dir in dirs {
        for candidate in lookup_candidates(dir, id) {
            if exists(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// An entry together with the id it was found under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopFileId {
    /// The id.
    pub id: String,
    /// Where the file is.
    pub path: PathBuf,
}

/// Identify a file, with or without a directory to be relative to.
#[must_use]
pub fn identify(file: &Path, app_dir: Option<&Path>) -> DesktopFileId {
    let id = match app_dir {
        Some(dir) => relative_id_dotted(file, dir),
        None => standalone_id(file),
    };
    DesktopFileId {
        id,
        path: file.to_path_buf(),
    }
}
