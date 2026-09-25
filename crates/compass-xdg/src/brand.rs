//! The names Compass is known by on disk, on the bus and in the environment,
//! and the one-time move away from the names it inherited from Vicinae.
//!
//! Until the Phase 7 cutover (ADR-0012, ADR-0020) the engine lived under the
//! upstream's identifiers: `$XDG_CONFIG_HOME/compass`, `VICINAE_*` variables,
//! `vicinae://` links. Those are now `compass`, `COMPASS_*` and `compass://`.
//! What an existing install already has is carried over rather than dropped:
//!
//! - [`migrate_legacy_dirs`] renames each `vicinae` base directory to
//!   `compass` once and leaves a `vicinae` symlink behind, so an older binary
//!   started after a rollback still finds the same data.
//! - [`env_var_os`] reads `COMPASS_<NAME>` and, when that is unset, the
//!   `VICINAE_<NAME>` a script or unit file may still export, logging once that
//!   the old spelling is deprecated.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The product's name, as the user sees it.
pub const APP_NAME: &str = "Compass";

/// The application id: desktop file, Flatpak, icon and tray item.
pub const APP_ID: &str = "org.tunaos.compass";

/// The directory name under each XDG base directory and the runtime directory.
pub const DIR_NAME: &str = "compass";

/// The directory name Compass used before the cutover, and the C++ engine uses.
pub const LEGACY_DIR_NAME: &str = "vicinae";

/// The prefix of every environment variable Compass reads.
pub const ENV_PREFIX: &str = "COMPASS_";

/// The prefix still honoured, with a deprecation warning, when the
/// [`ENV_PREFIX`] spelling is unset.
pub const LEGACY_ENV_PREFIX: &str = "VICINAE_";

/// The URL scheme Compass emits.
pub const URL_SCHEME: &str = "compass";

/// Schemes accepted as synonyms of [`URL_SCHEME`]: the upstream's, which
/// extensions and the Vicinae store emit, and Raycast's.
pub const ACCEPTED_URL_SCHEMES: &[&str] = &[URL_SCHEME, "vicinae", "raycast"];

/// The value of `name` (a `COMPASS_` variable), or of its `VICINAE_`
/// spelling when `name` is unset.
///
/// An empty `COMPASS_` value counts as set: it is how a caller turns a
/// setting off on purpose, and falling through to a stale legacy value would
/// undo that.
#[must_use]
pub fn env_var_os(name: &str) -> Option<OsString> {
    env_var_os_with(name, |key| std::env::var_os(key))
}

/// [`env_var_os`] as UTF-8; a value that is not is treated as unset.
#[must_use]
pub fn env_var(name: &str) -> Option<String> {
    env_var_os(name).and_then(|value| value.into_string().ok())
}

/// [`env_var_os`] over an injected environment.
pub fn env_var_os_with(
    name: &str,
    lookup: impl FnMut(&str) -> Option<OsString>,
) -> Option<OsString> {
    let legacy = legacy_env_name(name);
    env_var_os_or_with(name, legacy.as_deref(), lookup)
}

/// `name`, or `legacy` when `name` is unset: for the few variables whose old
/// spelling is not simply `VICINAE_` for `COMPASS_` (`VICINAE_API_URL`,
/// which is now `COMPASS_VICINAE_API_URL`).
#[must_use]
pub fn env_var_os_or(name: &str, legacy: &str) -> Option<OsString> {
    env_var_os_or_with(name, Some(legacy), |key| std::env::var_os(key))
}

/// [`env_var_os_or`] over an injected environment.
pub fn env_var_os_or_with(
    name: &str,
    legacy: Option<&str>,
    mut lookup: impl FnMut(&str) -> Option<OsString>,
) -> Option<OsString> {
    if let Some(value) = lookup(name) {
        return Some(value);
    }
    let legacy = legacy?;
    let value = lookup(legacy)?;
    warn_deprecated_once(name, legacy);
    Some(value)
}

/// `VICINAE_<NAME>` for `COMPASS_<NAME>`; `None` for any other name.
#[must_use]
pub fn legacy_env_name(name: &str) -> Option<String> {
    name.strip_prefix(ENV_PREFIX)
        .map(|rest| format!("{LEGACY_ENV_PREFIX}{rest}"))
}

fn warn_deprecated_once(name: &str, legacy: &str) {
    static WARNED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
    let first = WARNED
        .lock()
        .map(|mut warned| warned.insert(legacy.to_owned()))
        .unwrap_or(false);
    if first {
        tracing::warn!("{legacy} is deprecated and will stop being read; set {name} instead");
    }
}

/// `$XDG_STATE_HOME`, falling back to `~/.local/state`.
#[must_use]
pub fn state_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_STATE_HOME") {
        Some(value) if !value.is_empty() => Some(PathBuf::from(value)),
        _ => crate::home_dir().map(|home| home.join(".local/state")),
    }
}

/// What [`migrate_legacy_dir`] did under one base directory.
#[derive(Debug, PartialEq, Eq)]
pub enum Migration {
    /// There was no `vicinae` directory: nothing to carry over.
    NothingToMove,
    /// `vicinae` is already a symlink to `compass`, or `compass` exists and
    /// `vicinae` is a symlink elsewhere that is not ours to empty. Nothing
    /// was touched.
    AlreadyMigrated {
        /// Whether a `vicinae` entry (directory or symlink) is also there.
        legacy_present: bool,
    },
    /// `vicinae` was renamed to `compass`, and `vicinae` is now a symlink to
    /// it unless `symlink_error` says why it could not be made.
    Moved {
        /// The old path, now (normally) a symlink.
        from: PathBuf,
        /// The new path, holding the data.
        to: PathBuf,
        /// Why the compatibility symlink is missing, if it is.
        symlink_error: Option<String>,
    },
    /// `compass` already existed (the pre-cutover engine already kept a few
    /// Compass-only files there), so each entry of `vicinae` that `compass`
    /// did not already have was moved into it. When nothing was left behind,
    /// `vicinae` became a symlink to `compass` as in [`Migration::Moved`].
    Merged {
        /// The old directory.
        from: PathBuf,
        /// The existing directory the entries went into.
        to: PathBuf,
        /// The entry names moved.
        moved: Vec<String>,
        /// The entry names both had: left in `vicinae`, never overwritten.
        conflicts: Vec<String>,
        /// Why the compatibility symlink is missing, when there were no
        /// conflicts and it still could not be made.
        symlink_error: Option<String>,
    },
    /// The move itself failed; the data is still at `from`.
    Failed {
        /// The directory that could not be moved.
        from: PathBuf,
        /// The error.
        error: String,
    },
}

/// Moves `<base>/vicinae` to `<base>/compass` and leaves `<base>/vicinae` as
/// a relative symlink to it.
///
/// Nothing in an existing `compass` is ever overwritten. When `compass`
/// already exists -- the engine kept its Rhai scripts, host-command grants
/// and some caches there before the cutover, so on a real install it usually
/// does -- the entries of `vicinae` it lacks are moved in one by one, and
/// `vicinae` becomes the symlink only if that emptied it. A `vicinae` that is
/// itself a symlink (a dotfiles checkout) is moved as the link it is when
/// `compass` is free, and left alone otherwise.
pub fn migrate_legacy_dir(base: &Path) -> Migration {
    let legacy = base.join(LEGACY_DIR_NAME);
    let current = base.join(DIR_NAME);
    let Ok(legacy_meta) = legacy.symlink_metadata() else {
        return Migration::NothingToMove;
    };
    // A dangling `vicinae` symlink points at nothing worth moving.
    if !legacy.exists() {
        return Migration::NothingToMove;
    }

    if current.symlink_metadata().is_err() {
        if let Err(error) = std::fs::rename(&legacy, &current) {
            return Migration::Failed {
                from: legacy,
                error: error.to_string(),
            };
        }
        let symlink_error = symlink_dir(Path::new(DIR_NAME), &legacy)
            .err()
            .map(|error| error.to_string());
        return Migration::Moved {
            from: legacy,
            to: current,
            symlink_error,
        };
    }

    if legacy_meta.file_type().is_symlink() || !legacy_meta.is_dir() || !current.is_dir() {
        return Migration::AlreadyMigrated {
            legacy_present: true,
        };
    }
    merge_into(legacy, current)
}

fn merge_into(legacy: PathBuf, current: PathBuf) -> Migration {
    let entries = match std::fs::read_dir(&legacy) {
        Ok(entries) => entries,
        Err(error) => {
            return Migration::Failed {
                from: legacy,
                error: error.to_string(),
            };
        }
    };
    let mut names: Vec<OsString> = entries
        .filter_map(|entry| Some(entry.ok()?.file_name()))
        .collect();
    names.sort();

    let mut moved = Vec::with_capacity(names.len());
    let mut conflicts = Vec::new();
    for name in names {
        let target = current.join(&name);
        let label = name.to_string_lossy().into_owned();
        if target.symlink_metadata().is_ok() {
            conflicts.push(label);
            continue;
        }
        match std::fs::rename(legacy.join(&name), &target) {
            Ok(()) => moved.push(label),
            Err(_) => conflicts.push(label),
        }
    }

    let symlink_error = if conflicts.is_empty() {
        std::fs::remove_dir(&legacy)
            .and_then(|()| symlink_dir(Path::new(DIR_NAME), &legacy))
            .err()
            .map(|error| error.to_string())
    } else {
        None
    };
    Migration::Merged {
        from: legacy,
        to: current,
        moved,
        conflicts,
        symlink_error,
    }
}

#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink_dir(target: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

#[cfg(not(any(unix, windows)))]
fn symlink_dir(_target: &Path, _link: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "symlinks are not supported on this platform",
    ))
}

/// The configuration file's name inside the config directory.
pub const CONFIG_FILE_NAME: &str = "compass.json";

/// The configuration file's name before the cutover.
pub const LEGACY_CONFIG_FILE_NAME: &str = "vicinae.json";

/// Renames `<config_dir>/vicinae.json` to `compass.json` and leaves a
/// `vicinae.json` symlink to it, on the same terms as [`migrate_legacy_dir`]:
/// an existing `compass.json` is never touched, and a `vicinae.json` that is a
/// symlink into a dotfiles checkout moves as the link it is.
pub fn migrate_legacy_config_file(config_dir: &Path) -> Migration {
    let legacy = config_dir.join(LEGACY_CONFIG_FILE_NAME);
    let current = config_dir.join(CONFIG_FILE_NAME);
    let legacy_meta = legacy.symlink_metadata().ok();

    if current.symlink_metadata().is_ok() {
        return Migration::AlreadyMigrated {
            legacy_present: legacy_meta.is_some(),
        };
    }
    // `is_file` follows a symlink: a dangling one has nothing worth moving.
    if legacy_meta.is_none() || !legacy.is_file() {
        return Migration::NothingToMove;
    }
    if let Err(error) = std::fs::rename(&legacy, &current) {
        return Migration::Failed {
            from: legacy,
            error: error.to_string(),
        };
    }
    let symlink_error = symlink_file(Path::new(CONFIG_FILE_NAME), &legacy)
        .err()
        .map(|error| error.to_string());
    Migration::Moved {
        from: legacy,
        to: current,
        symlink_error,
    }
}

#[cfg(unix)]
fn symlink_file(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink_file(target: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[cfg(not(any(unix, windows)))]
fn symlink_file(_target: &Path, _link: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "symlinks are not supported on this platform",
    ))
}

/// Everything the cutover carries over, from the environment: each base
/// directory ([`legacy_migration_bases`]), then the configuration file inside
/// the config directory. Called once, as the engine starts.
pub fn migrate_legacy_install() -> Vec<(PathBuf, Migration)> {
    let mut out = migrate_legacy_dirs(&legacy_migration_bases());
    if let Some(config_dir) = crate::config_home().map(|home| home.join(DIR_NAME)) {
        let migration = migrate_legacy_config_file(&config_dir);
        out.push((config_dir, migration));
    }
    out
}

/// [`migrate_legacy_dir`] under each of `bases`. Nothing is logged here:
/// this runs before the process has a subscriber, so the caller hands each
/// result to [`log_migration`] once it does.
///
/// The bases are the config, data, cache and state homes
/// ([`legacy_migration_bases`]); the runtime directory holds only sockets
/// that a restart recreates, so it is not migrated.
pub fn migrate_legacy_dirs(bases: &[PathBuf]) -> Vec<(PathBuf, Migration)> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(bases.len());
    for base in bases {
        // `XDG_CACHE_HOME=$XDG_DATA_HOME` is legal; one base is moved once.
        if !seen.insert(base.clone()) {
            continue;
        }
        let migration = migrate_legacy_dir(base);
        out.push((base.clone(), migration));
    }
    out
}

/// Logs what one migration did; staying put is not worth a line.
pub fn log_migration(base: &Path, migration: &Migration) {
    match migration {
        Migration::NothingToMove | Migration::AlreadyMigrated { .. } => {}
        Migration::Moved {
            from,
            to,
            symlink_error: None,
        } => tracing::info!(
            from = %from.display(),
            to = %to.display(),
            "moved pre-rename data; the old path is now a symlink to it"
        ),
        Migration::Moved {
            from,
            to,
            symlink_error: Some(error),
        } => tracing::warn!(
            from = %from.display(),
            to = %to.display(),
            %error,
            "moved pre-rename data, but could not leave a symlink at the old path"
        ),
        Migration::Merged {
            from,
            to,
            moved,
            conflicts,
            symlink_error,
        } => {
            tracing::info!(
                from = %from.display(),
                to = %to.display(),
                moved = ?moved,
                "moved pre-rename data into the existing directory"
            );
            if !conflicts.is_empty() {
                tracing::warn!(
                    from = %from.display(),
                    conflicts = ?conflicts,
                    "these entries exist in both places and were left where they are"
                );
            }
            if let Some(error) = symlink_error {
                tracing::warn!(
                    from = %from.display(),
                    %error,
                    "could not leave a symlink at the old path"
                );
            }
        }
        Migration::Failed { from, error } => tracing::warn!(
            from = %from.display(),
            %error,
            base = %base.display(),
            "could not move pre-rename data; starting without it"
        ),
    }
}

/// The config, data, cache and state homes, from the environment.
#[must_use]
pub fn legacy_migration_bases() -> Vec<PathBuf> {
    [
        crate::config_home(),
        crate::data_home(),
        crate::cache_home(),
        state_home(),
    ]
    .into_iter()
    .flatten()
    .collect()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, OsString> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), OsString::from(v)))
            .collect()
    }

    #[test]
    fn the_compass_variable_wins() {
        let vars = env(&[("COMPASS_X", "new"), ("VICINAE_X", "old")]);
        let got = env_var_os_with("COMPASS_X", |k| vars.get(k).cloned());
        assert_eq!(got, Some("new".into()));
    }

    #[test]
    fn the_vicinae_variable_is_the_fallback() {
        let vars = env(&[("VICINAE_X", "old")]);
        let got = env_var_os_with("COMPASS_X", |k| vars.get(k).cloned());
        assert_eq!(got, Some("old".into()));
    }

    #[test]
    fn an_empty_compass_value_is_not_overridden_by_the_legacy_one() {
        let vars = env(&[("COMPASS_X", ""), ("VICINAE_X", "old")]);
        let got = env_var_os_with("COMPASS_X", |k| vars.get(k).cloned());
        assert_eq!(got, Some("".into()));
    }

    #[test]
    fn an_explicit_legacy_name_is_the_fallback() {
        let vars = env(&[("VICINAE_API_URL", "old")]);
        let got = env_var_os_or_with("COMPASS_VICINAE_API_URL", Some("VICINAE_API_URL"), |k| {
            vars.get(k).cloned()
        });
        assert_eq!(got, Some("old".into()));
    }

    #[test]
    fn only_compass_names_have_a_legacy_spelling() {
        assert_eq!(
            legacy_env_name("COMPASS_API").as_deref(),
            Some("VICINAE_API")
        );
        assert_eq!(legacy_env_name("HOME"), None);
        let vars = env(&[("VICINAE_HOME", "x")]);
        assert_eq!(env_var_os_with("HOME", |k| vars.get(k).cloned()), None);
    }

    #[test]
    fn a_legacy_dir_is_moved_and_left_as_a_symlink() {
        let base = tempfile::tempdir().unwrap();
        let legacy = base.path().join("vicinae");
        std::fs::create_dir_all(legacy.join("scripts")).unwrap();
        std::fs::write(legacy.join("vicinae.json"), "{}").unwrap();

        let migration = migrate_legacy_dir(base.path());

        let current = base.path().join("compass");
        assert_eq!(
            migration,
            Migration::Moved {
                from: legacy.clone(),
                to: current.clone(),
                symlink_error: None,
            }
        );
        assert!(current.is_dir() && !current.is_symlink());
        assert_eq!(
            std::fs::read_to_string(current.join("vicinae.json")).unwrap(),
            "{}"
        );
        assert!(legacy.is_symlink());
        assert_eq!(std::fs::read_link(&legacy).unwrap(), Path::new("compass"));
        assert!(
            legacy.join("scripts").is_dir(),
            "an old binary still finds its data"
        );
    }

    #[test]
    fn into_an_existing_compass_dir_only_missing_entries_move() {
        let base = tempfile::tempdir().unwrap();
        let legacy = base.path().join("vicinae");
        let current = base.path().join("compass");
        std::fs::create_dir_all(legacy.join("extensions/ext")).unwrap();
        std::fs::write(legacy.join("vicinae.db"), "db").unwrap();
        std::fs::write(legacy.join("shared"), "old").unwrap();
        std::fs::create_dir_all(current.join("scripts")).unwrap();
        std::fs::write(current.join("shared"), "new").unwrap();

        let migration = migrate_legacy_dir(base.path());

        assert_eq!(
            migration,
            Migration::Merged {
                from: legacy.clone(),
                to: current.clone(),
                moved: vec!["extensions".into(), "vicinae.db".into()],
                conflicts: vec!["shared".into()],
                symlink_error: None,
            }
        );
        assert_eq!(
            std::fs::read_to_string(current.join("shared")).unwrap(),
            "new"
        );
        assert_eq!(
            std::fs::read_to_string(legacy.join("shared")).unwrap(),
            "old"
        );
        assert!(current.join("extensions/ext").is_dir());
        assert!(current.join("scripts").is_dir());
        assert!(
            legacy.is_dir() && !legacy.is_symlink(),
            "a conflict keeps the old directory as a directory"
        );
    }

    #[test]
    fn a_merge_without_conflicts_leaves_the_symlink() {
        let base = tempfile::tempdir().unwrap();
        let legacy = base.path().join("vicinae");
        let current = base.path().join("compass");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("vicinae.db"), "db").unwrap();
        std::fs::create_dir_all(current.join("scripts")).unwrap();

        let migration = migrate_legacy_dir(base.path());

        assert!(
            matches!(
                &migration,
                Migration::Merged { conflicts, symlink_error: None, .. } if conflicts.is_empty()
            ),
            "{migration:?}"
        );
        assert_eq!(std::fs::read_link(&legacy).unwrap(), Path::new("compass"));
        assert_eq!(
            std::fs::read_to_string(legacy.join("vicinae.db")).unwrap(),
            "db"
        );
    }

    #[test]
    fn a_symlinked_legacy_dir_beside_an_existing_compass_is_left_alone() {
        let base = tempfile::tempdir().unwrap();
        let dotfiles = tempfile::tempdir().unwrap();
        std::fs::write(dotfiles.path().join("a"), "a").unwrap();
        std::os::unix::fs::symlink(dotfiles.path(), base.path().join("vicinae")).unwrap();
        std::fs::create_dir(base.path().join("compass")).unwrap();
        assert_eq!(
            migrate_legacy_dir(base.path()),
            Migration::AlreadyMigrated {
                legacy_present: true
            }
        );
        assert!(dotfiles.path().join("a").is_file());
        assert!(!base.path().join("compass/a").exists());
    }

    #[test]
    fn nothing_happens_without_a_legacy_dir() {
        let base = tempfile::tempdir().unwrap();
        assert_eq!(migrate_legacy_dir(base.path()), Migration::NothingToMove);
        assert!(!base.path().join("compass").exists());
        assert!(!base.path().join("vicinae").exists());
    }

    #[test]
    fn a_second_run_is_a_no_op() {
        let base = tempfile::tempdir().unwrap();
        std::fs::create_dir(base.path().join("vicinae")).unwrap();
        assert!(matches!(
            migrate_legacy_dir(base.path()),
            Migration::Moved { .. }
        ));
        assert_eq!(
            migrate_legacy_dir(base.path()),
            Migration::AlreadyMigrated {
                legacy_present: true
            }
        );
        assert_eq!(
            std::fs::read_link(base.path().join("vicinae")).unwrap(),
            Path::new("compass")
        );
    }

    #[test]
    fn a_symlinked_legacy_dir_moves_as_the_link_it_is() {
        let base = tempfile::tempdir().unwrap();
        let dotfiles = tempfile::tempdir().unwrap();
        std::fs::write(dotfiles.path().join("vicinae.json"), "{}").unwrap();
        std::os::unix::fs::symlink(dotfiles.path(), base.path().join("vicinae")).unwrap();

        assert!(matches!(
            migrate_legacy_dir(base.path()),
            Migration::Moved {
                symlink_error: None,
                ..
            }
        ));
        assert_eq!(
            std::fs::read_link(base.path().join("compass")).unwrap(),
            dotfiles.path()
        );
        assert!(base.path().join("vicinae/vicinae.json").is_file());
        assert!(dotfiles.path().join("vicinae.json").is_file());
    }

    #[test]
    fn a_dangling_legacy_symlink_is_left_alone() {
        let base = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/nonexistent/compass-test", base.path().join("vicinae"))
            .unwrap();
        assert_eq!(migrate_legacy_dir(base.path()), Migration::NothingToMove);
        assert!(!base.path().join("compass").exists());
    }

    #[test]
    fn every_base_is_migrated_once() {
        let root = tempfile::tempdir().unwrap();
        let bases: Vec<PathBuf> = ["config", "data", "cache", "state"]
            .iter()
            .map(|name| root.path().join(name))
            .collect();
        for base in &bases[..3] {
            std::fs::create_dir_all(base.join("vicinae")).unwrap();
        }
        std::fs::create_dir_all(&bases[3]).unwrap();

        let mut with_duplicate = bases.clone();
        with_duplicate.push(bases[0].clone());
        let results = migrate_legacy_dirs(&with_duplicate);

        assert_eq!(results.len(), 4, "the repeated base is visited once");
        for (base, migration) in &results[..3] {
            assert!(matches!(migration, Migration::Moved { .. }), "{base:?}");
            assert!(base.join("compass").is_dir());
        }
        assert_eq!(results[3].1, Migration::NothingToMove);
    }

    #[test]
    fn the_config_file_is_renamed_and_left_as_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("vicinae.json"), "{\"a\":1}").unwrap();

        assert!(matches!(
            migrate_legacy_config_file(dir.path()),
            Migration::Moved {
                symlink_error: None,
                ..
            }
        ));
        let current = dir.path().join("compass.json");
        assert!(current.is_file() && !current.is_symlink());
        assert_eq!(
            std::fs::read_link(dir.path().join("vicinae.json")).unwrap(),
            Path::new("compass.json")
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("vicinae.json")).unwrap(),
            "{\"a\":1}"
        );
        assert!(matches!(
            migrate_legacy_config_file(dir.path()),
            Migration::AlreadyMigrated {
                legacy_present: true
            }
        ));
    }

    #[test]
    fn an_existing_compass_json_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("vicinae.json"), "old").unwrap();
        std::fs::write(dir.path().join("compass.json"), "new").unwrap();
        assert!(matches!(
            migrate_legacy_config_file(dir.path()),
            Migration::AlreadyMigrated { .. }
        ));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("compass.json")).unwrap(),
            "new"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("vicinae.json")).unwrap(),
            "old"
        );
    }

    #[test]
    fn a_symlinked_legacy_config_file_moves_as_the_link_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let dotfiles = tempfile::tempdir().unwrap();
        let target = dotfiles.path().join("v.json");
        std::fs::write(&target, "{}").unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join("vicinae.json")).unwrap();
        assert!(matches!(
            migrate_legacy_config_file(dir.path()),
            Migration::Moved { .. }
        ));
        assert_eq!(
            std::fs::read_link(dir.path().join("compass.json")).unwrap(),
            target
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("vicinae.json")).unwrap(),
            "{}"
        );
    }

    #[test]
    fn a_dangling_legacy_config_symlink_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/nonexistent/v.json", dir.path().join("vicinae.json")).unwrap();
        assert_eq!(
            migrate_legacy_config_file(dir.path()),
            Migration::NothingToMove
        );
        assert!(!dir.path().join("compass.json").exists());
    }
}
