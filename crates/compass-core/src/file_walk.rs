//! Walking a directory tree, the way the file indexer walks it.
//!
//! Ports `FileSystemWalker`. The tree is supplied by the caller rather than
//! read here, because every rule worth porting is a decision about *which*
//! entries are reached and in what order, and none of them is a decision about
//! how to read a directory.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crate::entry_filter::{Entry, EntryFilter};

/// One entry as the walk sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkEntry {
    /// Where it is.
    pub path: PathBuf,
    /// Whether it is a directory.
    pub is_directory: bool,
    /// Whether it is a symlink. The filter refuses these outright.
    pub is_symlink: bool,
}

impl WalkEntry {
    /// A plain file.
    #[must_use]
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            is_directory: false,
            is_symlink: false,
        }
    }

    /// A directory.
    #[must_use]
    pub fn directory(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            is_directory: true,
            is_symlink: false,
        }
    }

    fn as_filter_entry(&self) -> Entry<'_> {
        Entry {
            path: &self.path,
            is_symlink: self.is_symlink,
            is_directory: self.is_directory,
        }
    }
}

/// What the walk needs from the filesystem.
pub trait Tree {
    /// The direct contents of `dir`, in whatever order the filesystem gives
    /// them. An unreadable directory answers with nothing, as
    /// `directory_iterator` with an error code does.
    fn entries(&self, dir: &Path) -> Vec<WalkEntry>;

    /// Whether `path` is a readable directory.
    fn is_directory(&self, path: &Path) -> bool;

    /// Whether this entry is a `CACHEDIR.TAG` naming its directory as a cache.
    ///
    /// Reading the tag's signature is the filter's job
    /// ([`crate::entry_filter::is_cachedir_tag`]); the walk only asks.
    fn is_cachedir_tag(&self, path: &Path) -> bool;

    /// The contents of an ignore file, if there is one at `path`.
    fn read_ignore_file(&self, _path: &Path) -> Option<String> {
        None
    }
}

/// How deep below the root an entry sits, counted in path components.
///
/// The C++ subtracts the root's component count from the entry's, so a child
/// of the root is at depth 1. The root itself is never measured: it is pushed
/// onto the stack rather than visited.
#[must_use]
pub fn depth_below(root: &Path, path: &Path) -> usize {
    path.components()
        .count()
        .saturating_sub(root.components().count())
}

/// A walk of a tree.
#[derive(Debug)]
pub struct Walker {
    filter: EntryFilter,
    recursive: bool,
    max_depth: Option<usize>,
}

impl Walker {
    /// A recursive walk with no depth limit, filtering as the indexer does.
    #[must_use]
    pub fn new(filter: EntryFilter) -> Self {
        Self {
            filter,
            recursive: true,
            max_depth: None,
        }
    }

    /// Whether directories are descended into at all.
    #[must_use]
    pub fn recursive(mut self, value: bool) -> Self {
        self.recursive = value;
        self
    }

    /// How deep to descend, in components below the root.
    ///
    /// The comparison is `depth <= max_depth`, so a limit of 1 still descends
    /// into the root's children — it bounds what is *entered*, and an entry one
    /// level deeper than the limit is still reported by its parent's listing.
    #[must_use]
    pub fn max_depth(mut self, value: Option<usize>) -> Self {
        self.max_depth = value;
        self
    }

    /// Walks `root`, calling `visit` for every entry that passes the filter.
    ///
    /// The root itself is never visited: the walk reports what is *in* a tree,
    /// and a caller that wanted the root already has it.
    pub fn walk(&self, tree: &impl Tree, root: &Path, mut visit: impl FnMut(&WalkEntry)) {
        // `if (!fs::is_directory(root, ec)) return;` — a file, or a directory
        // that cannot be read, yields nothing rather than one entry.
        if !tree.is_directory(root) {
            return;
        }

        let mut pending = VecDeque::from([root.to_path_buf()]);

        while let Some(dir) = pending.pop_back() {
            let entries = tree.entries(&dir);

            // A directory holding a `CACHEDIR.TAG` is abandoned *whole*,
            // including the entries already listed before the tag turned up.
            // The C++ breaks out of the listing loop and drops the vector it
            // was filling, so what the walk reports does not depend on where
            // in the directory the tag happens to sit.
            if entries
                .iter()
                .any(|entry| tree.is_cachedir_tag(&entry.path))
            {
                continue;
            }

            for entry in &entries {
                if !self
                    .filter
                    .should_visit(entry.as_filter_entry(), |path| tree.read_ignore_file(path))
                {
                    continue;
                }

                if self.recursive && entry.is_directory {
                    let depth = depth_below(root, &entry.path);
                    if self.max_depth.is_none_or(|max| depth <= max) {
                        pending.push_back(entry.path.clone());
                    }
                }

                visit(entry);
            }
        }
    }

    /// Every entry the walk reaches, in the order it reaches them.
    #[must_use]
    pub fn collect(&self, tree: &impl Tree, root: &Path) -> Vec<WalkEntry> {
        let mut found = Vec::new();
        self.walk(tree, root, |entry| found.push(entry.clone()));
        found
    }
}
