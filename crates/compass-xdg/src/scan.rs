//! Discovery of desktop entry files below an XDG applications directory.

use std::ops::Deref;
use std::path::{Path, PathBuf};

use crate::{DesktopEntry, Error, ParseOptions};

const MAX_SCAN_DEPTH: usize = 8;

/// A desktop entry file discovered below an applications directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopFile {
    id: String,
    path: PathBuf,
}

impl DesktopFile {
    /// The desktop file ID computed relative to the scanned applications directory.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The path discovered by the scanner.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads and parses this file using the process locale.
    ///
    /// # Errors
    ///
    /// See [`DesktopEntry::from_file`].
    pub fn parse(&self) -> Result<ScannedDesktopEntry, Error> {
        self.parse_with(&ParseOptions::default())
    }

    /// Reads and parses this file with explicit options.
    ///
    /// # Errors
    ///
    /// See [`DesktopEntry::from_file_with`].
    pub fn parse_with(&self, options: &ParseOptions) -> Result<ScannedDesktopEntry, Error> {
        Ok(ScannedDesktopEntry {
            id: self.id.clone(),
            entry: DesktopEntry::from_file_with(&self.path, options)?,
        })
    }
}

/// A parsed entry whose desktop file ID is guaranteed by directory scanning.
#[derive(Debug, Clone)]
pub struct ScannedDesktopEntry {
    id: String,
    entry: DesktopEntry,
}

impl ScannedDesktopEntry {
    /// The desktop file ID computed by the scanner.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Borrows the parsed entry.
    #[must_use]
    pub fn entry(&self) -> &DesktopEntry {
        &self.entry
    }

    /// Returns the parsed entry, discarding the scanner wrapper.
    #[must_use]
    pub fn into_entry(self) -> DesktopEntry {
        self.entry
    }
}

impl Deref for ScannedDesktopEntry {
    type Target = DesktopEntry;

    fn deref(&self) -> &Self::Target {
        &self.entry
    }
}

/// A directory the scanner could not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanError {
    /// The unreadable directory.
    pub path: PathBuf,
    /// The operating-system error text.
    pub message: String,
}

/// Files and non-fatal errors produced by a directory scan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesktopScan {
    /// Discovered `.desktop` files, sorted by desktop file ID.
    pub files: Vec<DesktopFile>,
    /// Directories that could not be read. A missing root is not an error.
    pub errors: Vec<ScanError>,
}

/// Recursively discovers desktop entry files below `root`.
///
/// The result is sorted by desktop file ID. Missing roots produce an empty scan; other unreadable
/// directories are reported without aborting discovery.
#[must_use]
pub fn scan_desktop_files(root: impl AsRef<Path>) -> DesktopScan {
    let root = root.as_ref();
    let mut scan = DesktopScan::default();
    collect(root, root, 0, &mut scan);
    scan.files.sort_by(|a, b| a.id.cmp(&b.id));
    scan
}

fn collect(root: &Path, dir: &Path, depth: usize, scan: &mut DesktopScan) {
    if depth > MAX_SCAN_DEPTH {
        return;
    }

    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                scan.errors.push(ScanError {
                    path: dir.to_path_buf(),
                    message: err.to_string(),
                });
            }
            return;
        }
    };

    for entry in read.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };

        if file_type.is_dir() || (file_type.is_symlink() && path.is_dir()) {
            collect(root, &path, depth + 1, scan);
            continue;
        }

        if path.extension().is_none_or(|ext| ext != "desktop") {
            continue;
        }

        if let Some(id) = desktop_file_id(root, &path) {
            scan.files.push(DesktopFile { id, path });
        }
    }
}

/// Computes the desktop file ID of `path` relative to the applications directory `root`.
///
/// Returns `None` for a path outside `root` or one containing a non-Unicode component.
///
/// # Which separator, and why this one
///
/// The XDG specification says a desktop file ID joins nested components with
/// `-`. The C++ joins them with `.`
/// ([`crate::desktop_file::relative_id_dotted`]), and **this follows the C++**.
///
/// The id is not a display string, it is a key: `xdg-app-database.cpp` stores
/// an application's frecency score, alias, and enabled state under it. The two
/// engines disagreeing means neither finds the other's records for any nested
/// application, which is where distribution-packaged KDE and GNOME
/// applications live.
///
/// So the question is which engine changes, and the answer is the one with no
/// users: the C++ has shipped and written these keys on real machines, and the
/// Rust engine has no tagged release. Matching the specification here would
/// orphan existing records to fix a divergence nobody can observe — the
/// separator is never shown, and the ambiguity it creates (`kde4.konsole.desktop`
/// could be a nested `konsole` or a flat file of that name) needs a file
/// deliberately named to collide.
///
/// Inherited rather than endorsed. If the ids are ever migrated, this is the
/// line to change and [`crate::desktop_file`] holds both spellings.
#[must_use]
pub fn desktop_file_id(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut id = String::new();
    for component in relative.components() {
        let part = component.as_os_str().to_str()?;
        if !id.is_empty() {
            id.push('.');
        }
        id.push_str(part);
    }
    (!id.is_empty()).then_some(id)
}
