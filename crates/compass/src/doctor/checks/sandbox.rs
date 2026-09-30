//! Flatpak sandbox detection and the application directories the index searches.
//!
//! Part of [`super`]; see that module for the purity rule every function here
//! keeps to.

use std::path::{Path, PathBuf};

use compass_ipc::{DoctorCheck, DoctorStatus};

use super::check;
use crate::doctor::env::Env;
use crate::doctor::fs::FsProbe;

/// Marker file the Flatpak runtime places in every sandbox.
pub const FLATPAK_INFO_PATH: &str = "/.flatpak-info";

/// Whether we are running inside a Flatpak sandbox.
///
/// Informational either way — both answers are supported configurations — but
/// which one it is changes how apps get launched and what has to be in the
/// manifest, so a bug report needs it stated.
pub fn flatpak<F: FsProbe>(fs: &F) -> DoctorCheck {
    const NAME: &str = "flatpak.sandbox";
    if fs.exists(Path::new(FLATPAK_INFO_PATH)) {
        check(
            NAME,
            DoctorStatus::Ok,
            format!(
                "inside a Flatpak sandbox ({FLATPAK_INFO_PATH} present). Host applications are \
                 launched through `flatpak-spawn --host` with an OpenURI fallback, and \
                 .desktop/icon indexing needs --filesystem=host-os:ro plus read access to \
                 ~/.local/share/{{applications,icons}}"
            ),
        )
    } else {
        check(
            NAME,
            DoctorStatus::Ok,
            format!(
                "not sandboxed ({FLATPAK_INFO_PATH} absent); applications are launched directly"
            ),
        )
    }
}

/// The bundled data directory of a Flatpak'd app -- ours, when we are the app.
const FLATPAK_APP_SHARE: &str = "/app/share/applications";

/// The XDG application directories this session searches, in order.
///
/// Takes the filesystem probe as well as the environment because inside a Flatpak the list is
/// not derivable from `$XDG_DATA_DIRS` alone: see [`compass_core::xdg_dirs::sandbox_data_roots_for`]. The
/// roots come from `compass-core`, which owns the `compass-xdg` seam, rather than being spelled again here, so this reports what the
/// index actually searches -- a third copy of the list is what made #95 invisible in a report
/// that was otherwise looking straight at it.
#[must_use]
pub fn application_dir_paths<F: FsProbe>(env: &Env, fs: &F) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();

    let data_home = env
        .get("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| env.get("HOME").map(|h| Path::new(h).join(".local/share")));
    if let Some(home) = data_home {
        dirs.push(home.join("applications"));
    }

    let data_dirs = env.list("XDG_DATA_DIRS");
    let data_dirs: Vec<&str> = if data_dirs.is_empty() {
        vec!["/usr/local/share", "/usr/share"]
    } else {
        data_dirs
    };
    for dir in data_dirs {
        dirs.push(Path::new(dir).join("applications"));
    }

    for root in compass_core::xdg_dirs::sandbox_data_roots_for(
        fs.exists(Path::new(FLATPAK_INFO_PATH)),
        env.get("HOME").map(Path::new),
    ) {
        dirs.push(root.join("applications"));
    }

    // Order-preserving dedupe: XDG_DATA_DIRS routinely repeats an entry, and
    // counting the same directory twice would inflate the .desktop total.
    let mut seen = std::collections::BTreeSet::new();
    dirs.retain(|dir| seen.insert(dir.clone()));
    dirs
}

/// Which application directories exist, are readable, and how much is in them.
pub fn application_dirs<F: FsProbe>(env: &Env, fs: &F) -> DoctorCheck {
    const NAME: &str = "xdg.application-dirs";

    let dirs = application_dir_paths(env, fs);
    if dirs.is_empty() {
        return check(
            NAME,
            DoctorStatus::Fail,
            "no application directories resolve: neither XDG_DATA_HOME nor HOME is set and \
             XDG_DATA_DIRS is empty. App search will return nothing",
        );
    }

    let mut lines = Vec::new();
    let mut total = 0usize;
    let mut unreadable = 0usize;

    for dir in &dirs {
        if !fs.exists(dir) {
            lines.push(format!("{} — absent", dir.display()));
            continue;
        }
        match fs.dir_entries(dir) {
            Ok(entries) => {
                let count = entries.iter().filter(|e| e.ends_with(".desktop")).count();
                // OUR OWN ENTRY DOES NOT COUNT AS BEING ABLE TO SEE APPLICATIONS.
                //
                // `/app/share` is the Flatpak's own bundled data, so inside our sandbox it
                // always holds exactly one file: `org.tunaos.compass.desktop`. Counting it
                // meant `total` could never be zero however little else was found, which is
                // precisely what happened in #95 -- 88 applications on the machine, none of
                // them visible, and this check reporting `ok` on a total of 1.
                if dir == Path::new(FLATPAK_APP_SHARE) {
                    lines.push(format!(
                        "{} — {count} .desktop files (ours; not counted)",
                        dir.display()
                    ));
                    continue;
                }
                total += count;
                lines.push(format!("{} — {count} .desktop files", dir.display()));
            }
            Err(err) => {
                unreadable += 1;
                lines.push(format!("{} — unreadable: {err}", dir.display()));
            }
        }
    }

    let status = if total == 0 {
        DoctorStatus::Fail
    } else if unreadable > 0 {
        DoctorStatus::Warn
    } else {
        DoctorStatus::Ok
    };

    let headline = match status {
        DoctorStatus::Ok => format!("{total} .desktop files across {} directories", dirs.len()),
        DoctorStatus::Warn => format!(
            "{total} .desktop files, but {unreadable} of {} directories could not be read, so \
             some applications will be missing from search",
            dirs.len()
        ),
        DoctorStatus::Fail => format!(
            "no .desktop files found in any of the {} application directories; app search will \
             return nothing",
            dirs.len()
        ),
    };

    check(NAME, status, format!("{headline}\n{}", lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::checks::tests::detail;
    use crate::doctor::fs::FakeFs;

    // ----- flatpak.sandbox ------------------------------------------------

    #[test]
    fn flatpak_inside_the_sandbox_mentions_flatpak_spawn() {
        let c = flatpak(&FakeFs::new().with_file(FLATPAK_INFO_PATH));
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("flatpak-spawn --host"));
        assert!(detail(&c).contains("host-os:ro"));
    }

    #[test]
    fn flatpak_outside_the_sandbox_says_so() {
        let c = flatpak(&FakeFs::new());
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("not sandboxed"));
    }

    // ----- xdg.application-dirs -------------------------------------------

    #[test]
    fn application_dirs_default_to_the_spec_paths() {
        let env = Env::from_pairs([("HOME", "/home/tester")]);
        let dirs = application_dir_paths(&env, &FakeFs::new());
        assert_eq!(
            dirs,
            [
                PathBuf::from("/home/tester/.local/share/applications"),
                PathBuf::from("/usr/local/share/applications"),
                PathBuf::from("/usr/share/applications"),
            ]
        );
    }

    #[test]
    fn application_dirs_prefer_xdg_data_home_over_home() {
        let env = Env::from_pairs([
            ("HOME", "/home/tester"),
            ("XDG_DATA_HOME", "/custom/data"),
            ("XDG_DATA_DIRS", "/usr/share"),
        ]);
        assert_eq!(
            application_dir_paths(&env, &FakeFs::new()),
            [
                PathBuf::from("/custom/data/applications"),
                PathBuf::from("/usr/share/applications"),
            ]
        );
    }

    #[test]
    fn application_dirs_are_deduplicated() {
        let env = Env::from_pairs([
            ("XDG_DATA_HOME", "/usr/share"),
            ("XDG_DATA_DIRS", "/usr/share:/opt/share:/usr/share"),
        ]);
        assert_eq!(
            application_dir_paths(&env, &FakeFs::new()),
            [
                PathBuf::from("/usr/share/applications"),
                PathBuf::from("/opt/share/applications"),
            ]
        );
    }

    #[test]
    fn application_dirs_with_no_home_and_no_data_dirs_still_have_the_system_defaults() {
        assert_eq!(
            application_dir_paths(&Env::empty(), &FakeFs::new()),
            [
                PathBuf::from("/usr/local/share/applications"),
                PathBuf::from("/usr/share/applications"),
            ]
        );
    }

    #[test]
    fn application_dirs_counts_only_desktop_files() {
        let env = Env::from_pairs([("XDG_DATA_HOME", "/d"), ("XDG_DATA_DIRS", "/usr/share")]);
        let fs = FakeFs::new()
            .with_dir(
                "/d/applications",
                ["a.desktop", "notes.txt", "mimeinfo.cache"],
            )
            .with_dir("/usr/share/applications", ["b.desktop", "c.desktop"]);
        let c = application_dirs(&env, &fs);
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("3 .desktop files"));
        assert!(detail(&c).contains("/d/applications — 1 .desktop files"));
    }

    #[test]
    fn inside_a_flatpak_the_host_directories_are_reported() {
        // #95: the report listed six directories and none of them was the one holding the
        // machine's applications, so it looked healthy while the index was empty.
        let env = Env::from_pairs([("HOME", "/var/home/someone")]);
        let fs = FakeFs::new()
            .with_file(FLATPAK_INFO_PATH)
            .with_dir("/run/host/usr/share/applications", ["firefox.desktop"]);

        let dirs = application_dir_paths(&env, &fs);
        assert!(
            dirs.contains(&PathBuf::from("/run/host/usr/share/applications")),
            "the host's /usr, where --filesystem=host-os:ro mounts it: {dirs:?}"
        );
        assert!(
            dirs.contains(&PathBuf::from(
                "/var/home/someone/.local/share/applications"
            )),
            "the user's real data dir, which $XDG_DATA_HOME no longer names: {dirs:?}"
        );

        let c = application_dirs(&env, &fs);
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("/run/host/usr/share/applications — 1 .desktop files"));
    }

    #[test]
    fn outside_a_flatpak_no_host_directories_are_reported() {
        // The control for the test above: without the sandbox marker these must not appear, or
        // every host install grows six permanently-absent lines in its report.
        let env = Env::from_pairs([("HOME", "/home/someone")]);
        let dirs = application_dir_paths(&env, &FakeFs::new());
        assert!(dirs.iter().all(|d| !d.starts_with("/run/host")), "{dirs:?}");
    }

    #[test]
    fn our_own_bundled_desktop_file_does_not_count_as_seeing_applications() {
        // THE EXACT SHAPE OF #95. Inside our Flatpak, /app/share/applications always holds
        // org.tunaos.compass.desktop, so a total that counts it can never reach zero and the
        // "App search will return nothing" failure can never fire -- which is why a machine
        // with 88 applications and an index of 0 reported `ok`.
        let env = Env::from_pairs([
            ("HOME", "/var/home/someone"),
            ("XDG_DATA_DIRS", "/app/share"),
        ]);
        let fs = FakeFs::new()
            .with_file(FLATPAK_INFO_PATH)
            .with_dir("/app/share/applications", ["org.tunaos.compass.desktop"]);

        let c = application_dirs(&env, &fs);
        assert_eq!(
            c.status,
            DoctorStatus::Fail,
            "finding only ourselves is indistinguishable from finding nothing: {}",
            detail(&c)
        );
        assert!(detail(&c).contains("ours; not counted"), "{}", detail(&c));
    }

    #[test]
    fn our_own_bundled_desktop_file_is_still_reported() {
        // Not counted is not the same as not shown: the line has to stay, or the next person
        // diagnosing this cannot tell "we did not look there" from "it was empty".
        let env = Env::from_pairs([
            ("HOME", "/var/home/someone"),
            ("XDG_DATA_DIRS", "/app/share"),
        ]);
        let fs = FakeFs::new()
            .with_file(FLATPAK_INFO_PATH)
            .with_dir("/app/share/applications", ["org.tunaos.compass.desktop"])
            .with_dir("/run/host/usr/share/applications", ["firefox.desktop"]);

        let c = application_dirs(&env, &fs);
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(
            detail(&c).contains("/app/share/applications — 1 .desktop files (ours; not counted)")
        );
        assert!(
            detail(&c).contains("1 .desktop files across"),
            "{}",
            detail(&c)
        );
    }

    #[test]
    fn application_dirs_absent_directories_are_not_a_problem_on_their_own() {
        let env = Env::from_pairs([("XDG_DATA_HOME", "/d"), ("XDG_DATA_DIRS", "/usr/share")]);
        let fs = FakeFs::new().with_dir("/usr/share/applications", ["b.desktop"]);
        let c = application_dirs(&env, &fs);
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("/d/applications — absent"));
    }

    #[test]
    fn application_dirs_unreadable_directory_warns() {
        let env = Env::from_pairs([("XDG_DATA_HOME", "/d"), ("XDG_DATA_DIRS", "/usr/share")]);
        let fs = FakeFs::new()
            .with_unreadable_dir("/d/applications", "Permission denied (os error 13)")
            .with_dir("/usr/share/applications", ["b.desktop"]);
        let c = application_dirs(&env, &fs);
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("Permission denied"));
        assert!(detail(&c).contains("some applications will be missing"));
    }

    #[test]
    fn application_dirs_with_nothing_to_index_fails() {
        let env = Env::from_pairs([("XDG_DATA_DIRS", "/usr/share")]);
        let fs = FakeFs::new().with_dir("/usr/share/applications", ["mimeinfo.cache"]);
        let c = application_dirs(&env, &fs);
        assert_eq!(c.status, DoctorStatus::Fail);
        assert!(detail(&c).contains("app search will return nothing"));
    }

    #[test]
    fn application_dirs_completely_missing_fails() {
        let c = application_dirs(&Env::empty(), &FakeFs::new());
        assert_eq!(c.status, DoctorStatus::Fail);
    }
}
