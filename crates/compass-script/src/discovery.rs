//! Finding scripts on disk.
//!
//! Mirrors `compass_core::manifest::registry` for Node extensions: an ordered
//! list of search paths, one directory per script, shadowing by directory
//! name with the first path winning, dot-directories skipped, and a missing
//! search path not an error. They live under `compass/scripts`, beside
//! `compass/extensions`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::ScriptError;
use crate::manifest::ScriptManifest;

/// The application's directory name under each XDG data root.
pub const DATA_DIR_NAME: &str = "compass";

/// The subdirectory scripts live in.
pub const SCRIPTS_SUBDIR: &str = "scripts";

/// Where scripts are looked for, highest precedence first:
/// `$XDG_DATA_HOME/compass/scripts`, then each `$XDG_DATA_DIRS` entry's.
#[must_use]
pub fn search_paths_for(data_home: Option<&Path>, data_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let user = data_home.map(|home| home.join(DATA_DIR_NAME).join(SCRIPTS_SUBDIR));
    let mut paths: Vec<PathBuf> = user.iter().cloned().collect();
    for dir in data_dirs {
        let path = dir.join(DATA_DIR_NAME).join(SCRIPTS_SUBDIR);
        if Some(&path) != user.as_ref() && !paths.contains(&path) {
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

/// The user's own script directory, the one a host should create and watch.
#[must_use]
pub fn user_scripts_dir() -> Option<PathBuf> {
    compass_xdg::xdg_dirs::data_home().map(|home| home.join(DATA_DIR_NAME).join(SCRIPTS_SUBDIR))
}

/// A script found on disk, manifest and source together, so a reload can tell
/// whether anything changed without reading the file again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredScript {
    /// The parsed manifest.
    pub manifest: ScriptManifest,
    /// The entry file's text.
    pub source: String,
}

/// What a scan found.
#[derive(Debug, Default)]
pub struct Scan {
    /// Scripts that loaded, in precedence order.
    pub scripts: Vec<DiscoveredScript>,
    /// Directories skipped because a higher-precedence one has the same name,
    /// as `(shadowed, winner)` pairs.
    pub shadowed: Vec<(PathBuf, PathBuf)>,
    /// Directories that look like scripts but would not load, and why.
    pub failed: Vec<(PathBuf, ScriptError)>,
}

/// Loads one script directory.
///
/// # Errors
///
/// When the manifest or the entry file cannot be used.
pub fn load(directory: &Path) -> Result<DiscoveredScript, ScriptError> {
    let manifest = ScriptManifest::from_directory(directory)?;
    let source = manifest.read_source().map_err(|source| ScriptError::Io {
        path: manifest.entry.clone(),
        source,
    })?;
    Ok(DiscoveredScript { manifest, source })
}

/// Scans `paths` in order, keeping the first script of each name.
#[must_use]
pub fn scan(paths: &[PathBuf]) -> Scan {
    let mut scan = Scan::default();
    let mut seen: BTreeMap<String, PathBuf> = BTreeMap::new();

    for root in paths {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        let mut directories: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.path())
            .collect();
        directories.sort();

        for path in directories {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            if let Some(winner) = seen.get(name) {
                scan.shadowed.push((path.clone(), winner.clone()));
                continue;
            }
            match load(&path) {
                Ok(script) => {
                    seen.insert(name.to_owned(), path);
                    scan.scripts.push(script);
                }
                Err(error) => scan.failed.push((path, error)),
            }
        }
    }
    scan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(root: &Path, name: &str, title: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("script.toml"), format!("title = \"{title}\"")).unwrap();
        std::fs::write(dir.join("main.rhai"), "fn search(q) { [] }").unwrap();
    }

    #[test]
    fn search_paths_put_the_user_first_and_drop_duplicates() {
        let paths = search_paths_for(
            Some(Path::new("/home/u/.local/share")),
            &[
                PathBuf::from("/usr/share"),
                PathBuf::from("/home/u/.local/share"),
                PathBuf::from("/usr/share"),
            ],
        );
        assert_eq!(
            paths,
            [
                PathBuf::from("/home/u/.local/share/compass/scripts"),
                PathBuf::from("/usr/share/compass/scripts"),
            ]
        );
    }

    #[test]
    fn the_first_path_wins_and_broken_or_hidden_directories_are_reported_or_skipped() {
        let user = tempfile::tempdir().unwrap();
        let system = tempfile::tempdir().unwrap();
        script(user.path(), "notes", "Mine");
        script(system.path(), "notes", "Packaged");
        script(system.path(), "uuid", "UUID");
        script(system.path(), ".staging-x", "Hidden");
        std::fs::create_dir_all(system.path().join("broken")).unwrap();

        let found = scan(&[
            user.path().to_path_buf(),
            system.path().to_path_buf(),
            PathBuf::from("/does/not/exist"),
        ]);
        let titles: Vec<&str> = found
            .scripts
            .iter()
            .map(|s| s.manifest.title.as_str())
            .collect();
        assert_eq!(titles, ["Mine", "UUID"]);
        assert_eq!(found.shadowed.len(), 1);
        assert_eq!(found.failed.len(), 1);
        assert!(found.failed[0].0.ends_with("broken"));
    }
}
