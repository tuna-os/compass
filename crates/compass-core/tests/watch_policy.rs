//! Which directories are worth an inotify watch.
//!
//! Ported from `important-dir-watcher-linux.cpp`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use compass_core::entry_filter::EntryFilter;
use compass_core::file_walk::{Tree, WalkEntry};
use compass_core::watch_policy::{
    MAX_WATCH_DEPTH, WATCH_BUDGET, build_watch_set, important_roots, new_directory_depth,
    should_watch_new_directory,
};

#[derive(Default)]
struct FakeTree {
    dirs: HashMap<PathBuf, Vec<WalkEntry>>,
}

impl FakeTree {
    fn with(mut self, dir: &str, entries: Vec<WalkEntry>) -> Self {
        self.dirs.insert(PathBuf::from(dir), entries);
        self
    }
}

impl Tree for FakeTree {
    fn entries(&self, dir: &Path) -> Vec<WalkEntry> {
        self.dirs.get(dir).cloned().unwrap_or_default()
    }

    fn is_directory(&self, path: &Path) -> bool {
        self.dirs.contains_key(path)
    }

    fn is_cachedir_tag(&self, _path: &Path) -> bool {
        false
    }
}

fn owned(items: &[&str]) -> Vec<PathBuf> {
    items.iter().map(PathBuf::from).collect()
}

fn watched(set: &[(PathBuf, usize)]) -> Vec<String> {
    set.iter()
        .map(|(path, _)| path.to_string_lossy().into_owned())
        .collect()
}

fn all_directories(path: &Path) -> bool {
    let _ = path;
    true
}

fn nothing_is_a_symlink(path: &Path) -> bool {
    let _ = path;
    false
}

#[test]
fn the_home_directory_is_itself_a_root() {
    let roots = important_roots(
        Some(Path::new("/home/u")),
        &[],
        all_directories,
        nothing_is_a_symlink,
        &[],
    );

    assert_eq!(watched_paths(&roots), ["/home/u"]);
}

fn watched_paths(roots: &[PathBuf]) -> Vec<String> {
    roots
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn no_home_means_no_roots_at_all() {
    // Not even the XDG directories: the C++ returns before reaching them.
    let roots = important_roots(
        None,
        &owned(&["/home/u/Documents"]),
        all_directories,
        nothing_is_a_symlink,
        &owned(&["/home/u/.config"]),
    );

    assert!(roots.is_empty());
}

#[test]
fn every_visible_subdirectory_of_home_is_a_root() {
    let roots = important_roots(
        Some(Path::new("/home/u")),
        &owned(&["/home/u/Documents", "/home/u/code"]),
        all_directories,
        nothing_is_a_symlink,
        &[],
    );

    assert_eq!(
        watched_paths(&roots),
        ["/home/u", "/home/u/Documents", "/home/u/code"]
    );
}

#[test]
fn a_file_in_home_is_not_a_root() {
    let roots = important_roots(
        Some(Path::new("/home/u")),
        &owned(&["/home/u/notes.txt"]),
        |path| path != Path::new("/home/u/notes.txt"),
        nothing_is_a_symlink,
        &[],
    );

    assert_eq!(watched_paths(&roots), ["/home/u"]);
}

#[test]
fn a_symlink_in_home_is_not_a_root() {
    // It would watch a tree already watched under its real name.
    let roots = important_roots(
        Some(Path::new("/home/u")),
        &owned(&["/home/u/link"]),
        all_directories,
        |path| path == Path::new("/home/u/link"),
        &[],
    );

    assert_eq!(watched_paths(&roots), ["/home/u"]);
}

#[test]
fn a_hidden_directory_in_home_is_not_a_root() {
    // Under home these are caches and state far more often than documents.
    let roots = important_roots(
        Some(Path::new("/home/u")),
        &owned(&["/home/u/.cache", "/home/u/code"]),
        all_directories,
        nothing_is_a_symlink,
        &[],
    );

    assert_eq!(watched_paths(&roots), ["/home/u", "/home/u/code"]);
}

#[test]
fn the_xdg_directories_are_added_although_they_are_hidden() {
    // They are hidden, so the enumeration above skips them, and they are
    // indexed regardless. This is the whole reason the function is not just
    // "list the home directory".
    let roots = important_roots(
        Some(Path::new("/home/u")),
        &[],
        all_directories,
        nothing_is_a_symlink,
        &owned(&["/home/u/.config", "/home/u/.local/share"]),
    );

    assert_eq!(
        watched_paths(&roots),
        ["/home/u", "/home/u/.config", "/home/u/.local/share"]
    );
}

#[test]
fn an_xdg_directory_that_does_not_exist_is_left_out() {
    let roots = important_roots(
        Some(Path::new("/home/u")),
        &[],
        |path| path != Path::new("/home/u/.config"),
        nothing_is_a_symlink,
        &owned(&["/home/u/.config"]),
    );

    assert_eq!(watched_paths(&roots), ["/home/u"]);
}

#[test]
fn the_shipped_limits_are_what_they_are() {
    // Read through the constants everywhere else, so nothing else would notice
    // them changing. Two levels, and the common fs.inotify.max_user_watches.
    assert_eq!(MAX_WATCH_DEPTH, 2);
    assert_eq!(WATCH_BUDGET, 8192);
}

#[test]
fn a_root_is_watched_at_depth_zero() {
    let tree = FakeTree::default().with("/root", vec![]);

    let set = build_watch_set(
        &tree,
        &owned(&["/root"]),
        &EntryFilter::new(None),
        WATCH_BUDGET,
    );

    assert_eq!(set.watches, [(PathBuf::from("/root"), 0)]);
    assert!(!set.budget_exhausted);
}

#[test]
fn subdirectories_are_watched_down_to_the_ceiling() {
    let tree = FakeTree::default()
        .with("/root", vec![WalkEntry::directory("/root/a")])
        .with("/root/a", vec![WalkEntry::directory("/root/a/b")])
        .with("/root/a/b", vec![WalkEntry::directory("/root/a/b/c")]);

    let set = build_watch_set(
        &tree,
        &owned(&["/root"]),
        &EntryFilter::new(None),
        WATCH_BUDGET,
    );

    // Depth 2 is watched; its children are left to the periodic scan.
    assert_eq!(watched(&set.watches), ["/root", "/root/a", "/root/a/b"]);
}

#[test]
fn a_file_is_not_watched() {
    let tree = FakeTree::default().with(
        "/root",
        vec![
            WalkEntry::file("/root/notes.txt"),
            WalkEntry::directory("/root/a"),
        ],
    );

    let set = build_watch_set(
        &tree,
        &owned(&["/root"]),
        &EntryFilter::new(None),
        WATCH_BUDGET,
    );

    assert_eq!(watched(&set.watches), ["/root", "/root/a"]);
}

#[test]
fn a_filtered_directory_is_not_watched() {
    let mut filter = EntryFilter::new(None);
    filter.set_excluded_filenames(vec!["node_modules".to_owned()]);

    let tree = FakeTree::default().with(
        "/root",
        vec![
            WalkEntry::directory("/root/node_modules"),
            WalkEntry::directory("/root/src"),
        ],
    );

    let set = build_watch_set(&tree, &owned(&["/root"]), &filter, WATCH_BUDGET);

    assert_eq!(watched(&set.watches), ["/root", "/root/src"]);
}

#[test]
fn a_directory_reached_twice_is_watched_once() {
    let tree = FakeTree::default()
        .with("/a", vec![WalkEntry::directory("/shared")])
        .with("/b", vec![WalkEntry::directory("/shared")])
        .with("/shared", vec![]);

    let set = build_watch_set(
        &tree,
        &owned(&["/a", "/b"]),
        &EntryFilter::new(None),
        WATCH_BUDGET,
    );

    assert_eq!(watched(&set.watches), ["/a", "/b", "/shared"]);
}

#[test]
fn a_root_listed_twice_is_watched_once() {
    let tree = FakeTree::default().with("/root", vec![]);

    let set = build_watch_set(
        &tree,
        &owned(&["/root", "/root"]),
        &EntryFilter::new(None),
        WATCH_BUDGET,
    );

    assert_eq!(watched(&set.watches), ["/root"]);
}

#[test]
fn every_root_is_covered_before_any_root_goes_deeper() {
    // Breadth-first is the point. With a budget that can run out, the order
    // decides what is covered when it does -- depth-first would spend it all
    // inside the first root and leave the others unwatched.
    let tree = FakeTree::default()
        .with("/a", vec![WalkEntry::directory("/a/deep")])
        .with("/a/deep", vec![])
        .with("/b", vec![]);

    let set = build_watch_set(
        &tree,
        &owned(&["/a", "/b"]),
        &EntryFilter::new(None),
        WATCH_BUDGET,
    );

    assert_eq!(watched(&set.watches), ["/a", "/b", "/a/deep"]);
}

#[test]
fn running_out_of_watches_stops_the_walk_and_says_so() {
    let tree = FakeTree::default()
        .with(
            "/root",
            vec![
                WalkEntry::directory("/root/a"),
                WalkEntry::directory("/root/b"),
            ],
        )
        .with("/root/a", vec![])
        .with("/root/b", vec![]);

    let set = build_watch_set(&tree, &owned(&["/root"]), &EntryFilter::new(None), 2);

    assert_eq!(watched(&set.watches), ["/root", "/root/a"]);
    assert!(set.budget_exhausted);
}

#[test]
fn a_budget_that_was_enough_is_not_reported_as_exhausted() {
    // Reaching the limit is not an error -- the remaining directories fall back
    // to the scan cadence, which would have covered them anyway. But a set that
    // fit must not claim it did.
    let tree = FakeTree::default().with("/root", vec![]);

    let set = build_watch_set(&tree, &owned(&["/root"]), &EntryFilter::new(None), 1);

    assert!(!set.budget_exhausted);
}

#[test]
fn a_budget_of_nothing_watches_nothing() {
    let tree = FakeTree::default().with("/root", vec![]);

    let set = build_watch_set(&tree, &owned(&["/root"]), &EntryFilter::new(None), 0);

    assert!(set.watches.is_empty());
    assert!(set.budget_exhausted);
}

#[test]
fn a_directory_created_under_a_shallow_watch_is_watched_too() {
    assert!(should_watch_new_directory(0));
    assert!(should_watch_new_directory(1));
}

#[test]
fn a_directory_created_under_the_deepest_watch_is_left_to_the_scan() {
    // Exactly as it would have been had it existed at startup.
    assert!(!should_watch_new_directory(MAX_WATCH_DEPTH));
}

#[test]
fn a_new_directory_inherits_its_parents_depth_plus_one() {
    assert_eq!(new_directory_depth(0), 1);
    assert_eq!(new_directory_depth(1), 2);
}
