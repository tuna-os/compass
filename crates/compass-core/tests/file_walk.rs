//! Walking a directory tree, the way the file indexer walks it.
//!
//! Ported from `FileSystemWalker`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use compass_core::entry_filter::EntryFilter;
use compass_core::file_walk::{Tree, WalkEntry, Walker, depth_below};

/// A tree held in memory: a directory maps to its direct contents.
#[derive(Default)]
struct FakeTree {
    dirs: HashMap<PathBuf, Vec<WalkEntry>>,
    cachedir_tags: Vec<PathBuf>,
    /// Paths that list contents but are not directories, which is what an
    /// unreadable directory looks like to `fs::is_directory` with an error.
    not_directories: Vec<PathBuf>,
}

impl FakeTree {
    fn with(mut self, dir: &str, entries: Vec<WalkEntry>) -> Self {
        self.dirs.insert(PathBuf::from(dir), entries);
        self
    }

    fn not_a_directory(mut self, path: &str) -> Self {
        self.not_directories.push(PathBuf::from(path));
        self
    }

    fn tagged(mut self, path: &str) -> Self {
        self.cachedir_tags.push(PathBuf::from(path));
        self
    }
}

impl Tree for FakeTree {
    fn entries(&self, dir: &Path) -> Vec<WalkEntry> {
        self.dirs.get(dir).cloned().unwrap_or_default()
    }

    fn is_directory(&self, path: &Path) -> bool {
        self.dirs.contains_key(path) && !self.not_directories.iter().any(|item| item == path)
    }

    fn is_cachedir_tag(&self, path: &Path) -> bool {
        self.cachedir_tags.iter().any(|tag| tag == path)
    }
}

fn walker() -> Walker {
    Walker::new(EntryFilter::new(None))
}

fn paths(entries: &[WalkEntry]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| entry.path.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn the_root_is_not_among_the_entries() {
    let tree = FakeTree::default().with("/root", vec![WalkEntry::file("/root/a.txt")]);

    let found = walker().collect(&tree, Path::new("/root"));

    assert_eq!(paths(&found), ["/root/a.txt"]);
}

#[test]
fn a_root_that_is_not_a_directory_yields_nothing() {
    // `if (!fs::is_directory(root, ec)) return;` — including the unreadable
    // case, where the error code is set and the walk gives up rather than
    // reporting the root as its own single entry.
    // The fixture would happily list contents for this path: only the
    // directory check stands between the walk and reporting them. A tree that
    // simply had nothing there would pass with the check removed, and a
    // control said so.
    let tree = FakeTree::default()
        .with("/root/a.txt", vec![WalkEntry::file("/root/a.txt/inside")])
        .not_a_directory("/root/a.txt");

    let found = walker().collect(&tree, Path::new("/root/a.txt"));

    assert!(found.is_empty());
}

#[test]
fn a_subdirectory_is_both_reported_and_descended_into() {
    let tree = FakeTree::default()
        .with("/root", vec![WalkEntry::directory("/root/sub")])
        .with("/root/sub", vec![WalkEntry::file("/root/sub/a.txt")]);

    let found = walker().collect(&tree, Path::new("/root"));

    assert_eq!(paths(&found), ["/root/sub", "/root/sub/a.txt"]);
}

#[test]
fn a_walk_that_is_not_recursive_still_reports_the_directories() {
    let tree = FakeTree::default()
        .with("/root", vec![WalkEntry::directory("/root/sub")])
        .with("/root/sub", vec![WalkEntry::file("/root/sub/a.txt")]);

    let found = walker().recursive(false).collect(&tree, Path::new("/root"));

    // The directory is an entry like any other; what changes is that nothing
    // below it is reached.
    assert_eq!(paths(&found), ["/root/sub"]);
}

#[test]
fn an_unreadable_directory_does_not_stop_the_walk() {
    // `/root/locked` is a directory nothing can list. The C++ gets an error
    // code from `directory_iterator` and carries on with the stack.
    let tree = FakeTree::default()
        .with(
            "/root",
            vec![
                WalkEntry::directory("/root/locked"),
                WalkEntry::file("/root/a.txt"),
            ],
        )
        .with("/root/locked", vec![]);

    let found = walker().collect(&tree, Path::new("/root"));

    assert_eq!(paths(&found), ["/root/locked", "/root/a.txt"]);
}

#[test]
fn directories_are_descended_in_reverse_order_of_listing() {
    // A stack, not a queue: the last directory listed is the first entered.
    // This is not a preference — it decides which half of a large tree is
    // indexed first when a scan is interrupted.
    let tree = FakeTree::default()
        .with(
            "/root",
            vec![
                WalkEntry::directory("/root/first"),
                WalkEntry::directory("/root/second"),
            ],
        )
        .with("/root/first", vec![WalkEntry::file("/root/first/a.txt")])
        .with("/root/second", vec![WalkEntry::file("/root/second/b.txt")]);

    let found = walker().collect(&tree, Path::new("/root"));

    assert_eq!(
        paths(&found),
        [
            "/root/first",
            "/root/second",
            "/root/second/b.txt",
            "/root/first/a.txt"
        ]
    );
}

#[test]
fn a_depth_is_counted_in_components_below_the_root() {
    assert_eq!(depth_below(Path::new("/root"), Path::new("/root/a")), 1);
    assert_eq!(depth_below(Path::new("/root"), Path::new("/root/a/b")), 2);
    assert_eq!(depth_below(Path::new("/root"), Path::new("/root")), 0);
}

#[test]
fn a_max_depth_of_zero_reaches_the_roots_own_entries_only() {
    let tree = FakeTree::default()
        .with("/root", vec![WalkEntry::directory("/root/sub")])
        .with("/root/sub", vec![WalkEntry::file("/root/sub/a.txt")]);

    let found = walker()
        .max_depth(Some(0))
        .collect(&tree, Path::new("/root"));

    assert_eq!(paths(&found), ["/root/sub"]);
}

#[test]
fn a_max_depth_of_one_reaches_one_level_deeper_than_it_reads() {
    // `depth <= maxDepth` bounds what is *entered*, and entering a directory
    // reports its contents — so a limit of 1 yields entries at depth 2. Off by
    // one against the obvious reading, and it is the shipped behaviour.
    let tree = FakeTree::default()
        .with("/root", vec![WalkEntry::directory("/root/sub")])
        .with("/root/sub", vec![WalkEntry::directory("/root/sub/deep")])
        .with(
            "/root/sub/deep",
            vec![WalkEntry::file("/root/sub/deep/a.txt")],
        );

    let found = walker()
        .max_depth(Some(1))
        .collect(&tree, Path::new("/root"));

    assert_eq!(paths(&found), ["/root/sub", "/root/sub/deep"]);
}

#[test]
fn a_symlink_is_never_visited() {
    let tree = FakeTree::default().with(
        "/root",
        vec![
            WalkEntry {
                path: PathBuf::from("/root/link"),
                is_directory: false,
                is_symlink: true,
            },
            WalkEntry::file("/root/a.txt"),
        ],
    );

    let found = walker().collect(&tree, Path::new("/root"));

    assert_eq!(paths(&found), ["/root/a.txt"]);
}

#[test]
fn a_directory_holding_a_cachedir_tag_is_skipped_whole() {
    let tree = FakeTree::default()
        .with("/root", vec![WalkEntry::directory("/root/cache")])
        .with(
            "/root/cache",
            vec![
                WalkEntry::file("/root/cache/CACHEDIR.TAG"),
                WalkEntry::file("/root/cache/blob"),
            ],
        )
        .tagged("/root/cache/CACHEDIR.TAG");

    let found = walker().collect(&tree, Path::new("/root"));

    // The directory itself is still reported by its parent — what is skipped
    // is everything inside it.
    assert_eq!(paths(&found), ["/root/cache"]);
}

#[test]
fn entries_listed_before_the_cachedir_tag_are_dropped_too() {
    // The C++ breaks out of the listing loop and abandons the vector it was
    // filling, so what a cache directory yields does not depend on where in
    // the listing order the tag happens to sit. Reported on either side of it,
    // the answer is the same: nothing.
    let tree = FakeTree::default()
        .with("/root", vec![WalkEntry::directory("/root/cache")])
        .with(
            "/root/cache",
            vec![
                WalkEntry::file("/root/cache/blob"),
                WalkEntry::file("/root/cache/CACHEDIR.TAG"),
                WalkEntry::file("/root/cache/other"),
            ],
        )
        .tagged("/root/cache/CACHEDIR.TAG");

    let found = walker().collect(&tree, Path::new("/root"));

    assert_eq!(paths(&found), ["/root/cache"]);
}

#[test]
fn a_cache_directory_does_not_silence_its_siblings() {
    let tree = FakeTree::default()
        .with(
            "/root",
            vec![
                WalkEntry::directory("/root/keep"),
                WalkEntry::directory("/root/cache"),
            ],
        )
        .with("/root/keep", vec![WalkEntry::file("/root/keep/a.txt")])
        .with(
            "/root/cache",
            vec![WalkEntry::file("/root/cache/CACHEDIR.TAG")],
        )
        .tagged("/root/cache/CACHEDIR.TAG");

    let found = walker().collect(&tree, Path::new("/root"));

    assert_eq!(
        paths(&found),
        ["/root/keep", "/root/cache", "/root/keep/a.txt"]
    );
}

#[test]
fn an_excluded_path_is_not_visited_and_not_descended_into() {
    let mut filter = EntryFilter::new(None);
    filter.set_excluded_paths(vec![PathBuf::from("/root/skip")]);

    let tree = FakeTree::default()
        .with(
            "/root",
            vec![
                WalkEntry::directory("/root/skip"),
                WalkEntry::file("/root/a.txt"),
            ],
        )
        .with("/root/skip", vec![WalkEntry::file("/root/skip/secret")]);

    let found = Walker::new(filter).collect(&tree, Path::new("/root"));

    assert_eq!(paths(&found), ["/root/a.txt"]);
}

#[test]
fn an_excluded_filename_is_not_visited() {
    let mut filter = EntryFilter::new(None);
    filter.set_excluded_filenames(vec!["node_modules".to_owned()]);

    let tree = FakeTree::default().with(
        "/root",
        vec![
            WalkEntry::directory("/root/node_modules"),
            WalkEntry::file("/root/a.txt"),
        ],
    );

    let found = Walker::new(filter).collect(&tree, Path::new("/root"));

    assert_eq!(paths(&found), ["/root/a.txt"]);
}

#[test]
fn an_empty_tree_yields_nothing_rather_than_looping() {
    let tree = FakeTree::default().with("/root", vec![]);

    assert!(walker().collect(&tree, Path::new("/root")).is_empty());
}
