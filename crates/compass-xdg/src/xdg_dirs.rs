//! XDG base directory lookups.

use std::path::{Path, PathBuf};

/// `/.flatpak-info`, present inside a Flatpak sandbox and nowhere else.
const FLATPAK_INFO: &str = "/.flatpak-info";

/// Host data roots that a Flatpak sandbox exposes at a path `$XDG_DATA_DIRS` does not name.
///
/// THIS LIST IS THE REASON THE LAUNCHER CAN SEE ANY APPLICATIONS AT ALL, and it exists because
/// two of Flatpak's conventions disagree with the base-directory spec:
///
///   * `$XDG_DATA_DIRS` inside a sandbox is the RUNTIME's, plus `/run/host/share` and
///     `/run/host/user-share`. Those last two are populated only by `--filesystem=host`. With
///     the narrower `--filesystem=host-os:ro` the manifest asks for, the host's `/usr` lands at
///     `/run/host/usr` instead, and nothing in `$XDG_DATA_DIRS` points there.
///   * `$XDG_DATA_HOME` is redirected to `~/.var/app/<id>/data`, so it no longer names the
///     user's own `~/.local/share` — which `--filesystem=xdg-data/applications:ro` does grant,
///     at its real path.
///
/// The result was an index of zero applications on a machine with 88 desktop entries (#95): the
/// permissions were right and every path we looked in was the wrong one.
///
/// Each entry mirrors a `--filesystem=` grant in `packaging/flatpak/org.tunaos.compass.yaml`,
/// and `flatpak_manifest_grants_are_searched` asserts that correspondence, so a grant added to
/// the manifest without a path added here fails the build rather than silently indexing nothing.
const SANDBOX_HOST_ROOTS: &[&str] = &[
    // `--filesystem=host-os:ro`. The host's /usr, at the path Flatpak actually mounts it.
    "/run/host/usr/share",
    "/run/host/usr/local/share",
    // `--filesystem=/var/lib/flatpak/exports/share/{applications,icons}:ro`
    "/var/lib/flatpak/exports/share",
    // `--filesystem=/home/linuxbrew/.linuxbrew/share/applications:ro`
    "/home/linuxbrew/.linuxbrew/share",
];

/// Host data roots below the user's home, for the same reason as [`SANDBOX_HOST_ROOTS`].
///
/// Separate because they need `$HOME`, which the caller supplies so this stays testable — the
/// crate forbids `unsafe_code`, so a test cannot set an environment variable.
const SANDBOX_HOME_ROOTS: &[&str] = &[
    // `--filesystem=xdg-data/{applications,icons}:ro`, at the real path rather than the
    // redirected `$XDG_DATA_HOME`.
    ".local/share",
    // `--filesystem=xdg-data/flatpak/exports/share/{applications,icons}:ro`
    ".local/share/flatpak/exports/share",
];

/// Whether this process is inside a Flatpak sandbox.
#[must_use]
pub fn in_flatpak() -> bool {
    Path::new(FLATPAK_INFO).exists()
}

/// Extra data roots to search inside a Flatpak sandbox; empty everywhere else.
///
/// Deliberately NOT filtered by existence. A missing directory is already harmless to the
/// scanner, and `compass doctor` reports each searched directory with its file count or
/// `absent` — which is how #95 was finally diagnosed. Filtering here would delete exactly the
/// evidence that makes the next instance diagnosable.
#[must_use]
pub fn sandbox_data_roots() -> Vec<PathBuf> {
    sandbox_data_roots_for(in_flatpak(), home_dir().as_deref())
}

/// [`sandbox_data_roots`] with the sandbox signal and home supplied explicitly.
///
/// Public so `compass doctor` can report the directories the index really searches rather than
/// rebuilding the list and drifting from it -- which is how #95 stayed invisible: the doctor's
/// own report listed six directories, none of them the ones that would have had the answer.
#[must_use]
pub fn sandbox_data_roots_for(in_flatpak: bool, home: Option<&Path>) -> Vec<PathBuf> {
    if !in_flatpak {
        return Vec::new();
    }

    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(home) = home {
        roots.extend(SANDBOX_HOME_ROOTS.iter().map(|rel| home.join(rel)));
    }
    roots.extend(SANDBOX_HOST_ROOTS.iter().map(PathBuf::from));
    roots
}

/// `$HOME`, as the user's real home rather than anything Flatpak redirects.
#[must_use]
pub fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

/// Joins `leaf` onto every root, in order, dropping duplicates and keeping the first occurrence.
fn dirs_for(
    data_home: Option<PathBuf>,
    data_dirs: Vec<PathBuf>,
    sandbox_roots: Vec<PathBuf>,
    leaf: &str,
) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();

    if let Some(home) = data_home {
        dirs.push(home.join(leaf));
    }
    for dir in data_dirs {
        dirs.push(dir.join(leaf));
    }
    for root in sandbox_roots {
        dirs.push(root.join(leaf));
    }

    let mut seen = std::collections::HashSet::new();
    dirs.retain(|dir| seen.insert(dir.clone()));
    dirs
}

/// `/usr/local/share:/usr/share`, the specified fallback for an unset `$XDG_DATA_DIRS`.
pub const DEFAULT_DATA_DIRS: &str = "/usr/local/share:/usr/share";

/// The application directories, in precedence order: `$XDG_DATA_HOME/applications` first, then
/// each entry of `$XDG_DATA_DIRS` with `/applications` appended, then — inside a Flatpak only —
/// [`sandbox_data_roots`].
///
/// Duplicates are removed, keeping the first occurrence, so a `$XDG_DATA_DIRS` that repeats
/// `$XDG_DATA_HOME` does not index everything twice.
#[must_use]
pub fn application_dirs() -> Vec<PathBuf> {
    dirs_for(
        data_home(),
        data_dirs(),
        sandbox_data_roots(),
        "applications",
    )
}

/// The icon directories, in precedence order: `$XDG_DATA_HOME/icons` first, then each entry of
/// `$XDG_DATA_DIRS` with `/icons` appended, then — inside a Flatpak only —
/// [`sandbox_data_roots`].
///
/// The sandbox roots matter here for the same reason as in [`application_dirs`]: an entry whose
/// icon cannot be found still indexes, so this fails as a launcher full of blank rows rather
/// than as an empty one.
///
/// Duplicates are removed, keeping the first occurrence.
#[must_use]
pub fn icon_dirs() -> Vec<PathBuf> {
    dirs_for(data_home(), data_dirs(), sandbox_data_roots(), "icons")
}

/// `$XDG_DATA_HOME`, falling back to `~/.local/share`.
#[must_use]
pub fn data_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_DATA_HOME") {
        Some(value) if !value.is_empty() => Some(PathBuf::from(value)),
        _ => dirs::data_dir(),
    }
}

/// `$XDG_CONFIG_HOME`, falling back to `~/.config`.
#[must_use]
pub fn config_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(value) if !value.is_empty() => Some(PathBuf::from(value)),
        _ => dirs::config_dir(),
    }
}

/// `$XDG_CACHE_HOME`, falling back to `~/.cache`.
#[must_use]
pub fn cache_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_CACHE_HOME") {
        Some(value) if !value.is_empty() => Some(PathBuf::from(value)),
        _ => dirs::cache_dir(),
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

#[cfg(test)]
mod tests {
    use super::*;

    // The public lookups read the process environment, and this crate forbids `unsafe_code`, so
    // a test cannot set an environment variable to drive them. Everything below therefore
    // exercises the pure inner functions, which is why they take their inputs explicitly.

    #[test]
    fn outside_a_flatpak_nothing_extra_is_searched() {
        let roots = sandbox_data_roots_for(false, Some(Path::new("/home/someone")));
        assert!(
            roots.is_empty(),
            "a host install must not grow /run/host entries: {roots:?}"
        );
    }

    #[test]
    fn inside_a_flatpak_the_host_roots_are_searched() {
        let roots = sandbox_data_roots_for(true, Some(Path::new("/home/someone")));

        // The two that #95 was actually about, named individually rather than by count so a
        // silent removal fails rather than being absorbed by another entry being added.
        assert!(
            roots.contains(&PathBuf::from("/run/host/usr/share")),
            "the host's /usr, where --filesystem=host-os:ro puts it: {roots:?}"
        );
        assert!(
            roots.contains(&PathBuf::from("/home/someone/.local/share")),
            "the user's real data dir, which $XDG_DATA_HOME no longer names: {roots:?}"
        );
    }

    #[test]
    fn a_sandbox_with_no_home_still_searches_the_host_roots() {
        // `dirs::home_dir` returning None must not cost us the system entries as well.
        let roots = sandbox_data_roots_for(true, None);

        // Exactly the host roots and nothing else. Spelled as equality rather than as "nothing
        // under /home", which is what this first said and which was wrong: one of the host
        // roots IS under /home, because that is where Homebrew installs.
        let expected: Vec<PathBuf> = SANDBOX_HOST_ROOTS.iter().map(PathBuf::from).collect();
        assert_eq!(roots, expected);
    }

    #[test]
    fn the_spec_directories_still_come_first() {
        // Precedence is not cosmetic: a user's own override of a system .desktop file has to
        // win, and the sandbox roots include that same system directory by another path.
        let dirs = dirs_for(
            Some(PathBuf::from("/data/home")),
            vec![PathBuf::from("/usr/share")],
            vec![PathBuf::from("/run/host/usr/share")],
            "applications",
        );
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/data/home/applications"),
                PathBuf::from("/usr/share/applications"),
                PathBuf::from("/run/host/usr/share/applications"),
            ]
        );
    }

    #[test]
    fn a_root_repeated_across_sources_is_searched_once() {
        let dirs = dirs_for(
            Some(PathBuf::from("/usr/share")),
            vec![PathBuf::from("/usr/share")],
            vec![PathBuf::from("/usr/share")],
            "applications",
        );
        assert_eq!(dirs, vec![PathBuf::from("/usr/share/applications")]);
    }

    /// Every `--filesystem=` grant the Flatpak manifest asks for is a directory we search.
    ///
    /// THIS IS THE TEST THAT WOULD HAVE CAUGHT #95. The manifest asked for read access to the
    /// application catalogue and got it; the index then looked somewhere else entirely, and
    /// nothing connected the two. Asking for a permission and never reading the path it grants
    /// is the shape of that bug, and it is checkable.
    ///
    /// `host-os` is excluded because it is a Flatpak keyword rather than a path — what it
    /// exposes, `/run/host/usr`, is asserted by name above.
    #[test]
    fn flatpak_manifest_grants_are_searched() {
        let manifest = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../packaging/flatpak/org.tunaos.compass.yaml"
        );
        let text =
            std::fs::read_to_string(manifest).unwrap_or_else(|e| panic!("read {manifest}: {e}"));

        let home = Path::new("/home/someone");
        let searched: Vec<PathBuf> = dirs_for(
            None,
            Vec::new(),
            sandbox_data_roots_for(true, Some(home)),
            "applications",
        );

        let mut checked = 0;
        for line in text.lines() {
            let Some(grant) = line.trim().strip_prefix("- --filesystem=") else {
                continue;
            };
            let grant = grant.split(':').next().unwrap_or_default();
            if !grant.ends_with("/applications") {
                continue;
            }

            // `xdg-data/x` is Flatpak's spelling of the user's real `~/.local/share/x`.
            let path = match grant.strip_prefix("xdg-data/") {
                Some(rest) => home.join(".local/share").join(rest),
                None => PathBuf::from(grant),
            };

            assert!(
                searched.contains(&path),
                "the manifest grants {grant}, which resolves to {} -- but nothing searches it. \
                 Add its root to SANDBOX_HOST_ROOTS or SANDBOX_HOME_ROOTS.\nsearched: {searched:#?}",
                path.display()
            );
            checked += 1;
        }

        assert!(
            checked >= 4,
            "parsed only {checked} application grants from the manifest; the parser has probably \
             drifted from its format and this test is no longer checking anything"
        );
    }

    /// Every granted Flatpak `exports` tree needs its `app` tree granted too.
    ///
    /// This is #105, and the test above could not have caught it: that one
    /// checks a grant is searched, and the exports directories *were* granted
    /// and *were* searched. The trouble is what is in them. `flatpak-dir.c`'s
    /// `export_dir` writes symlinks with the prefix
    /// `../app/<id>/current/active/export`, so every entry in
    /// `exports/share/applications` points into the deploy tree and the file a
    /// launcher reads is there, not here.
    ///
    /// Grant one without the other and the sandbox sees a directory of links
    /// resolving to nothing. No error is raised anywhere -- the scan still
    /// lists the names, and the read fails one step later -- so every Flatpak
    /// application vanishes from search with nothing to show for it. That is
    /// why this is an assertion about the manifest rather than a comment in it.
    #[test]
    fn a_granted_flatpak_exports_tree_has_its_deploy_tree_granted() {
        let manifest = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../packaging/flatpak/org.tunaos.compass.yaml"
        );
        let text =
            std::fs::read_to_string(manifest).unwrap_or_else(|e| panic!("read {manifest}: {e}"));

        let grants: Vec<&str> = text
            .lines()
            .filter_map(|line| line.trim().strip_prefix("- --filesystem="))
            .map(|grant| grant.split(':').next().unwrap_or_default())
            .collect();

        let mut checked = 0;
        for grant in &grants {
            // The installation root is everything before `/exports/`, which is
            // `/var/lib/flatpak` for the system one and `xdg-data/flatpak` for
            // the user's.
            let Some((root, _)) = grant.split_once("/exports/") else {
                continue;
            };
            let deploy = format!("{root}/app");
            assert!(
                grants.contains(&deploy.as_str()),
                "the manifest grants {grant}, whose entries are symlinks into {deploy} -- but \
                 {deploy} is not granted, so they resolve to nothing inside the sandbox and \
                 every Flatpak application disappears from search (#105).\ngrants: {grants:#?}"
            );
            checked += 1;
        }

        assert!(
            checked >= 4,
            "found only {checked} flatpak exports grants; the parser has probably drifted from \
             the manifest's format and this test is no longer checking anything"
        );
    }
}
