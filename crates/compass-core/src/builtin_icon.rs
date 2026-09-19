//! The built-in icon set: the names `Icon.*` resolves to.
//!
//! Ports `BuiltinIconService`
//! (`src/server/src/services/builtin-icon/`). An extension writes
//! `Icon.AlarmRinging` and the host has to turn that into a picture; the name
//! in between is `"alarm-ringing"`, and the file is
//! `src/server/icons/alarm-ringing.svg`.
//!
//! As with [`crate::glyph`], the table is not written down twice: `build.rs`
//! parses the 852 mappings out of `builtin-icon.cpp`, so the two engines
//! cannot disagree about which icons exist or what they are called.
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

#[cfg(test)]
mod tests {
    use super::*;
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
        let dir = repo().join("src/server/icons");
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
                .join("src/server/icons")
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
    fn the_table_is_as_long_as_the_cpp_map() {
        // `builtin-icon.cpp` has one `{BuiltinIcon::X, "x"}` line per icon.
        let source = std::fs::read_to_string(
            repo().join("src/server/src/services/builtin-icon/builtin-icon.cpp"),
        )
        .expect("read the C++ mapping");
        let lines = source.matches("{BuiltinIcon::").count();
        assert_eq!(
            names().len(),
            lines,
            "the parser found {} of the C++'s {lines} mappings",
            names().len()
        );
    }
}
