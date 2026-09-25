//! The built-in icon set: the names `Icon.*` resolves to.
//!
//! Ports `BuiltinIconService`
//! (`src/server/src/services/builtin-icon/`). An extension writes
//! `Icon.AlarmRinging` and the host has to turn that into a picture; the name
//! in between is `"alarm-ringing"`, and the file is
//! `extra/builtin-icons/alarm-ringing.svg`.
//!
//! As with [`crate::glyph`], the table is not written down twice: `build.rs`
//! parses the 852 names out of the `Icon` enum in `@vicinae/api`
//! (`src/typescript/api/src/api/icon.ts`), so the host and the API extensions
//! import cannot disagree about which icons exist or what they are called.
//!
//! # Where the C++ looks, and why this cannot
//!
//! `pathForName` returns `":icons/<name>"`, a **Qt resource** path — the SVGs
//! are compiled into the binary. There is no such thing here, so
//! [`file_name`] gives the file's name and leaves the directory to whoever
//! ships the assets. Inventing a path would be inventing a packaging decision.

include!(concat!(env!("OUT_DIR"), "/builtin_icon_table.rs"));

/// The icon shown when a name is not one of ours.
///
/// `BuiltinIconService::unknownIcon()` is `QuestionMarkCircle`, and the
/// TypeScript API falls back to the same one when an extension's `Image` has
/// no source at all.
pub const UNKNOWN: &str = "question-mark-circle";

/// Every icon name, in the C++ enum's order.
#[must_use]
pub fn names() -> &'static [&'static str] {
    ICON_NAMES
}

/// Whether `name` is one of the built-in icons.
#[must_use]
pub fn is_builtin(name: &str) -> bool {
    ICON_NAMES.contains(&name)
}

/// The file `name` is drawn from, without a directory.
///
/// `None` for a name that is not built in, rather than a path that does not
/// exist: a caller that wants the fallback asks for [`UNKNOWN`] on purpose.
#[must_use]
pub fn file_name(name: &str) -> Option<String> {
    is_builtin(name).then(|| format!("{name}.svg"))
}

/// Where the icon files are: `$COMPASS_BUILTIN_ICONS`, else the first
/// `compass/builtin-icons` under `$XDG_DATA_HOME` or `$XDG_DATA_DIRS` (the
/// Flatpak installs them under `/app/share`).
#[must_use]
pub fn directory() -> Option<std::path::PathBuf> {
    directory_in(
        std::env::var_os("COMPASS_BUILTIN_ICONS").map(Into::into),
        crate::xdg_dirs::data_home(),
        std::env::var("XDG_DATA_DIRS").ok().as_deref(),
    )
}

/// [`directory`], from its inputs.
#[must_use]
pub fn directory_in(
    override_dir: Option<std::path::PathBuf>,
    data_home: Option<std::path::PathBuf>,
    data_dirs: Option<&str>,
) -> Option<std::path::PathBuf> {
    if override_dir.is_some() {
        return override_dir;
    }
    let data_dirs = data_dirs
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or("/usr/local/share:/usr/share");
    data_home
        .into_iter()
        .chain(data_dirs.split(':').map(std::path::PathBuf::from))
        .map(|dir| dir.join("compass/builtin-icons"))
        .find(|dir| dir.is_dir())
}

/// The file `name` is drawn from, when it is built in and installed.
#[must_use]
pub fn path(name: &str) -> Option<std::path::PathBuf> {
    let path = directory()?.join(file_name(name)?);
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icons_are_found_under_the_first_data_dir_that_has_them() {
        let none = tempfile::tempdir().unwrap();
        let app = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(app.path().join("compass/builtin-icons")).unwrap();
        let dirs = format!("{}:{}", none.path().display(), app.path().display());
        assert_eq!(
            directory_in(None, None, Some(&dirs)),
            Some(app.path().join("compass/builtin-icons")),
            "the Flatpak's /app/share is one of $XDG_DATA_DIRS"
        );
        assert_eq!(
            directory_in(Some("/icons".into()), None, Some(&dirs)),
            Some(std::path::PathBuf::from("/icons"))
        );
    }
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("two levels below the repository root")
            .to_path_buf()
    }

    #[test]
    fn every_name_has_an_svg_and_every_svg_has_a_name() {
        // The check that matters: a name with no file is a blank icon, and a
        // file with no name is an asset nobody can ask for. Both are silent.
        let dir = repo().join("extra/builtin-icons");
        let files: BTreeSet<String> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                (path.extension()? == "svg")
                    .then(|| path.file_stem()?.to_str().map(ToOwned::to_owned))
                    .flatten()
            })
            .collect();

        assert!(files.len() > 800, "only {} icons on disk", files.len());

        let named: BTreeSet<String> = names().iter().map(|n| (*n).to_owned()).collect();

        let missing: Vec<&String> = named.difference(&files).collect();
        assert!(
            missing.is_empty(),
            "{} icon names have no file: {missing:?}",
            missing.len()
        );

        let unnamed: Vec<&String> = files.difference(&named).collect();
        assert!(
            unnamed.is_empty(),
            "{} icon files have no name in the enum: {unnamed:?}",
            unnamed.len()
        );
    }

    #[test]
    fn the_names_are_unique_and_kebab_case() {
        let mut seen = BTreeSet::new();
        for name in names() {
            assert!(seen.insert(*name), "{name} appears twice");
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{name} is not the kebab-case spelling the files use"
            );
        }
    }

    #[test]
    fn the_unknown_icon_is_one_of_the_names() {
        // The fallback has to exist, or the failure path fails too.
        assert!(is_builtin(UNKNOWN), "{UNKNOWN} is not in the table");
        assert!(
            repo()
                .join("extra/builtin-icons")
                .join("question-mark-circle.svg")
                .exists(),
            "the fallback icon has no file"
        );
    }

    #[test]
    fn a_name_that_is_not_ours_resolves_to_nothing() {
        assert!(!is_builtin("definitely-not-an-icon"));
        assert_eq!(file_name("definitely-not-an-icon"), None);
        assert_eq!(
            file_name("alarm-ringing"),
            Some("alarm-ringing.svg".to_owned())
        );
    }

    #[test]
    fn the_table_is_as_long_as_the_api_enum() {
        // `icon.ts` has one `Name = "name",` line per icon.
        let source = std::fs::read_to_string(repo().join("src/typescript/api/src/api/icon.ts"))
            .expect("read the API's Icon enum");
        let lines = source.matches(" = \"").count();
        assert_eq!(
            names().len(),
            lines,
            "the parser found {} of the enum's {lines} members",
            names().len()
        );
    }
}
