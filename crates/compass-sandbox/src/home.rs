//! The few things in `$HOME` an extension may read.
//!
//! Everything else in the home directory stays denied. The list is a
//! decision (2026-09-25), not a discovery: each entry is there because a
//! store extension reads it to do its one job, and none of them is a secret
//! on its own.
//!
//! - `~/.ssh/config`, the host list `ssh` extensions offer. Only that file:
//!   the keys, `known_hosts` and everything else in `~/.ssh` stay denied.
//! - `~/.password-store`, which `pass` extensions list. Its entries are
//!   encrypted, and `~/.gnupg`, which would open them, stays denied.
//! - `~/.config/hypr`, `~/.config/sway` and `~/.config/niri`, which the
//!   keybinding-list extensions read.
//!
//! All read-only, and each only when it exists (Landlock cannot name a path
//! that is not there).
//!
//! # Symbolic links
//!
//! Landlock rules name inodes, so granting a path grants whatever it resolves
//! to. An entry that is (or passes through) a symbolic link is therefore
//! granted only when it resolves to somewhere the list itself names: a
//! `~/.config/hypr` linked into a dotfiles repository, or a
//! `~/.ssh/config` linked to a key, grants nothing. Inside a granted
//! directory Landlock itself refuses a link that leads out of it. The home
//! directory is resolved first, so a home that is itself a link (`/home` to
//! `/var/home` on an image-based system) is followed.

use std::path::{Path, PathBuf};

/// What an allowlisted entry must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// A regular file, granted alone.
    File,
    /// A directory, granted with everything beneath it.
    Directory,
}

/// One allowlisted path, relative to `$HOME`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HomeEntry {
    /// The path under `$HOME`.
    pub path: &'static str,
    /// What it must be.
    pub kind: EntryKind,
}

/// Everything in `$HOME` an extension may read, read-only.
pub const HOME_READ_ALLOWLIST: &[HomeEntry] = &[
    HomeEntry {
        path: ".ssh/config",
        kind: EntryKind::File,
    },
    HomeEntry {
        path: ".password-store",
        kind: EntryKind::Directory,
    },
    HomeEntry {
        path: ".config/hypr",
        kind: EntryKind::Directory,
    },
    HomeEntry {
        path: ".config/sway",
        kind: EntryKind::Directory,
    },
    HomeEntry {
        path: ".config/niri",
        kind: EntryKind::Directory,
    },
];

/// Why an allowlisted entry that exists was not granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// It is not the kind of thing the list names (a directory where a file
    /// is expected, or a device).
    WrongKind,
    /// It resolves, through a symbolic link, to somewhere the list does not
    /// name.
    LinksOutside,
}

/// What [`home_reads`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HomeReads {
    /// The paths to grant read access to, resolved.
    pub granted: Vec<PathBuf>,
    /// The entries that exist and were not granted, with why.
    pub refused: Vec<(PathBuf, Refusal)>,
}

/// The [`HOME_READ_ALLOWLIST`] entries that exist under `home`, resolved,
/// and the ones refused.
///
/// A missing entry is neither: it is simply not there to grant.
#[must_use]
pub fn home_reads(home: &Path) -> HomeReads {
    let mut reads = HomeReads::default();
    let Ok(home) = std::fs::canonicalize(home) else {
        return reads;
    };
    let named = |target: &Path| {
        HOME_READ_ALLOWLIST.iter().any(|entry| {
            let place = home.join(entry.path);
            match entry.kind {
                EntryKind::File => target == place,
                EntryKind::Directory => target.starts_with(&place),
            }
        })
    };
    for entry in HOME_READ_ALLOWLIST {
        let literal = home.join(entry.path);
        let Ok(target) = std::fs::canonicalize(&literal) else {
            continue;
        };
        let Ok(metadata) = std::fs::metadata(&target) else {
            continue;
        };
        let kind_matches = match entry.kind {
            EntryKind::File => metadata.is_file(),
            EntryKind::Directory => metadata.is_dir(),
        };
        if !kind_matches {
            reads.refused.push((literal, Refusal::WrongKind));
        } else if target != literal && !named(&target) {
            reads.refused.push((literal, Refusal::LinksOutside));
        } else if !reads.granted.contains(&target) {
            reads.granted.push(target);
        }
    }
    reads
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn home() -> tempfile::TempDir {
        let home = tempfile::tempdir().expect("a home");
        let ssh = home.path().join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        std::fs::write(ssh.join("config"), "Host box\n").unwrap();
        std::fs::write(ssh.join("id_ed25519"), "PRIVATE\n").unwrap();
        std::fs::create_dir_all(home.path().join(".password-store/web")).unwrap();
        std::fs::create_dir_all(home.path().join(".config/hypr")).unwrap();
        std::fs::create_dir_all(home.path().join(".gnupg")).unwrap();
        home
    }

    #[test]
    fn the_list_is_the_decided_five_and_only_ssh_config_is_a_file() {
        let paths: Vec<&str> = HOME_READ_ALLOWLIST.iter().map(|e| e.path).collect();
        assert_eq!(
            paths,
            [
                ".ssh/config",
                ".password-store",
                ".config/hypr",
                ".config/sway",
                ".config/niri"
            ]
        );
        for entry in HOME_READ_ALLOWLIST {
            assert_eq!(
                entry.kind == EntryKind::File,
                entry.path == ".ssh/config",
                "{entry:?}"
            );
        }
    }

    #[test]
    fn what_exists_is_granted_and_what_does_not_is_skipped() {
        let home = home();
        let root = std::fs::canonicalize(home.path()).unwrap();
        let reads = home_reads(home.path());
        assert_eq!(
            reads.granted,
            [
                root.join(".ssh/config"),
                root.join(".password-store"),
                root.join(".config/hypr"),
            ],
            "sway and niri are not there"
        );
        assert!(reads.refused.is_empty());
        assert!(
            !reads
                .granted
                .iter()
                .any(|p| p == &root || p == &root.join(".ssh")),
            "neither the home nor ~/.ssh is granted"
        );
    }

    #[test]
    fn a_link_to_a_key_or_out_of_the_list_grants_nothing() {
        let home = home();
        let root = std::fs::canonicalize(home.path()).unwrap();
        std::fs::remove_file(home.path().join(".ssh/config")).unwrap();
        symlink(
            home.path().join(".ssh/id_ed25519"),
            home.path().join(".ssh/config"),
        )
        .unwrap();
        std::fs::create_dir_all(home.path().join("dotfiles/sway")).unwrap();
        symlink(
            home.path().join("dotfiles/sway"),
            home.path().join(".config/sway"),
        )
        .unwrap();
        symlink(home.path(), home.path().join(".config/niri")).unwrap();

        let reads = home_reads(home.path());
        assert_eq!(
            reads.granted,
            [root.join(".password-store"), root.join(".config/hypr")]
        );
        assert_eq!(
            reads.refused,
            [
                (root.join(".ssh/config"), Refusal::LinksOutside),
                (root.join(".config/sway"), Refusal::LinksOutside),
                (root.join(".config/niri"), Refusal::LinksOutside),
            ]
        );
    }

    #[test]
    fn a_link_to_another_listed_place_is_followed() {
        let home = home();
        let root = std::fs::canonicalize(home.path()).unwrap();
        std::fs::create_dir_all(home.path().join(".config/hypr/sway")).unwrap();
        symlink(
            home.path().join(".config/hypr/sway"),
            home.path().join(".config/sway"),
        )
        .unwrap();
        let reads = home_reads(home.path());
        assert!(
            reads.granted.contains(&root.join(".config/hypr/sway")),
            "{reads:?}"
        );
    }

    #[test]
    fn a_home_that_is_itself_a_link_is_followed() {
        let real = home();
        let outer = tempfile::tempdir().unwrap();
        let linked = outer.path().join("home");
        symlink(real.path(), &linked).unwrap();
        let root = std::fs::canonicalize(real.path()).unwrap();
        let reads = home_reads(&linked);
        assert!(
            reads.granted.contains(&root.join(".ssh/config")),
            "{reads:?}"
        );
        assert!(reads.refused.is_empty());
    }

    #[test]
    fn the_wrong_kind_is_refused() {
        let home = home();
        let root = std::fs::canonicalize(home.path()).unwrap();
        std::fs::remove_file(home.path().join(".ssh/config")).unwrap();
        std::fs::create_dir(home.path().join(".ssh/config")).unwrap();
        std::fs::write(home.path().join(".config/niri"), "not a directory").unwrap();
        let reads = home_reads(home.path());
        assert_eq!(
            reads.refused,
            [
                (root.join(".ssh/config"), Refusal::WrongKind),
                (root.join(".config/niri"), Refusal::WrongKind),
            ]
        );
    }

    #[test]
    fn no_home_grants_nothing() {
        assert_eq!(
            home_reads(Path::new("/definitely/not/a/home")),
            HomeReads::default()
        );
    }
}
