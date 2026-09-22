//! Which directories are worth an inotify watch.
//!
//! Ports the policy in `important-dir-watcher-linux.cpp`, leaving the inotify
//! plumbing behind. A kernel watch is a finite resource — `fs.inotify.max_user_watches`
//! is 8,192 on many systems and shared with every other program on the desktop
//! — so a launcher that watched a whole home directory would take all of them
//! and break whatever asked next.
//!
//! The answer is not to watch less accurately but to watch *shallowly*, and to
//! treat running out as an expected outcome rather than an error: the periodic
//! scan already covers everything, and a watch only makes it prompt.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crate::file_walk::IndexWalk;

/// How deep below an important root a watch is placed.
///
/// Two levels. Deeper directories are reached by the periodic scan instead:
/// `~/code/project/src` is not watched, but a change in it still shows up
/// within a scan cycle, and the watch budget it would have cost covers a
/// hundred other projects' top levels.
pub const MAX_WATCH_DEPTH: usize = 2;

/// How many watches the launcher will take before giving up.
///
/// Matches the common `fs.inotify.max_user_watches` default. Reaching it is
/// **not** an error: the remaining directories fall back to the scan cadence,
/// which is what would have covered them anyway.
pub const WATCH_BUDGET: usize = 8192;

/// The directories worth watching closely.
///
/// The home directory itself, each of its visible subdirectories, and then the
/// XDG config and data homes — which are hidden, so the enumeration above skips
/// them, and which are indexed regardless. Adding them explicitly is the whole
/// reason the function is not just "list the home directory".
///
/// Symlinked and hidden entries are left out: a symlink would watch a tree that
/// is already watched under its real name, and hidden directories under home
/// are caches and state far more often than they are documents.
#[must_use]
pub fn important_roots(
    home: Option<&Path>,
    home_entries: &[PathBuf],
    is_directory: impl Fn(&Path) -> bool,
    is_symlink: impl Fn(&Path) -> bool,
    xdg_dirs: &[PathBuf],
) -> Vec<PathBuf> {
    // `if (home.empty()) return dirs;` — with no home there is nothing
    // important, and the XDG directories are not reached either.
    let Some(home) = home else {
        return Vec::new();
    };

    let mut roots = vec![home.to_path_buf()];

    for entry in home_entries {
        if !is_directory(entry) || is_symlink(entry) {
            continue;
        }
        if crate::entry_filter::is_hidden_path(entry) {
            continue;
        }
        roots.push(entry.clone());
    }

    for dir in xdg_dirs {
        if is_directory(dir) {
            roots.push(dir.clone());
        }
    }

    roots
}

/// The watches to place, and whether the budget ran out placing them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchSet {
    /// Each directory to watch, with how deep below its root it sits.
    pub watches: Vec<(PathBuf, usize)>,
    /// Whether the budget was reached, leaving directories to the scan.
    pub budget_exhausted: bool,
}

/// Works out the watch set, breadth-first from the important roots.
///
/// Breadth-first is the point: with a budget that can run out, the order
/// decides what is covered when it does. Depth-first would spend the whole
/// budget inside the first root and leave the others entirely unwatched, while
/// breadth-first covers every root's top level before any root's second.
///
/// The queue walks the live filesystem one level at a time, asking the walker's
/// policy about each directory — the same policy the deep scan enforces, so a
/// directory the scan would skip never costs a watch.
#[must_use]
pub fn build_watch_set(roots: &[PathBuf], walk: &IndexWalk, budget: usize) -> WatchSet {
    let mut set = WatchSet::default();
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
    let mut visited: Vec<PathBuf> = Vec::new();

    for root in roots {
        if visited.contains(root) {
            continue;
        }
        visited.push(root.clone());
        queue.push_back((root.clone(), 0));
    }

    while let Some((dir, depth)) = queue.pop_front() {
        if set.watches.len() >= budget {
            set.budget_exhausted = true;
            break;
        }

        set.watches.push((dir.clone(), depth));

        if depth >= MAX_WATCH_DEPTH {
            continue;
        }

        // An unreadable directory answers with nothing, as `directory_iterator`
        // with an error code does.
        let Ok(listing) = std::fs::read_dir(&dir) else {
            continue;
        };
        for child in listing {
            let Ok(child) = child else {
                continue;
            };
            let Ok(kind) = child.file_type() else {
                continue;
            };
            if !kind.is_dir() {
                continue;
            }
            let path = child.path();
            if !walk.should_visit(&path, kind.is_symlink(), true) {
                continue;
            }
            if visited.contains(&path) {
                continue;
            }
            visited.push(path.clone());
            queue.push_back((path, depth + 1));
        }
    }

    set
}

/// Whether a directory created inside a watched one gets a watch of its own.
///
/// It inherits its parent's depth plus one, and the same ceiling applies — so a
/// directory created three levels under a root is covered by the scan rather
/// than watched, exactly as it would have been had it existed at startup.
#[must_use]
pub fn should_watch_new_directory(parent_depth: usize) -> bool {
    parent_depth < MAX_WATCH_DEPTH
}

/// The depth a directory created inside a watched one is recorded at.
#[must_use]
pub const fn new_directory_depth(parent_depth: usize) -> usize {
    parent_depth + 1
}
