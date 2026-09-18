//! Which directories the indexer is actually asked to scan.
//!
//! Ports the path arithmetic in `file-indexer/util.hpp`. Given a list of roots
//! from the settings — which a user edits by hand, and which arrives with
//! duplicates, relative paths and overlapping subtrees — this works out the
//! smallest set that covers the same files.
//!
//! Scanning `~` and `~/code` as separate roots reads every file under `~/code`
//! twice and writes it twice, which is not a tidiness problem: the two scans
//! race each other's writes for the same rows.

use std::path::{Path, PathBuf};

/// Whether `path` is `ancestor`, or somewhere beneath it.
///
/// Compared **component by component**, not as text. `/home/user2` starts with
/// the characters of `/home/user` and is not inside it, and a string prefix
/// test would quietly drop one of the two from the scan set — the kind of bug
/// that only shows up for the user whose name is a prefix of someone else's.
#[must_use]
pub fn is_same_or_descendant_of(path: &Path, ancestor: &Path) -> bool {
    let mut components = path.components();

    for expected in ancestor.components() {
        if components.next() != Some(expected) {
            return false;
        }
    }

    true
}

/// Whether any of `roots` covers `path`.
#[must_use]
pub fn is_covered_by_any(path: &Path, roots: &[PathBuf]) -> bool {
    roots
        .iter()
        .any(|root| is_same_or_descendant_of(path, root))
}

/// Makes the paths absolute and comparable, then drops the duplicates.
///
/// `resolve` turns a relative path into an absolute one — the C++ calls
/// `fs::absolute`, which is the process's working directory and not something
/// this should read for itself.
#[must_use]
pub fn normalize_paths(paths: Vec<PathBuf>, resolve: impl Fn(&Path) -> PathBuf) -> Vec<PathBuf> {
    let mut normalized: Vec<PathBuf> = paths
        .iter()
        .map(|path| lexically_normal(&resolve(path)))
        .collect();

    normalized.sort();
    normalized.dedup();
    normalized
}

/// `lexically_normal`: resolves `.` and `..` without touching the disk.
///
/// Purely textual, so it does not follow symlinks — which is what the C++ does
/// and what a scan root should do, since a root that resolves through a link
/// would index the link's target under the wrong name.
#[must_use]
pub fn lexically_normal(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut out = PathBuf::new();
    let mut depth = 0usize;

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if depth > 0 {
                    out.pop();
                    depth -= 1;
                } else if out.as_os_str().is_empty() {
                    out.push("..");
                }
            }
            other => {
                out.push(other);
                if matches!(other, Component::Normal(_)) {
                    depth += 1;
                }
            }
        }
    }

    out
}

/// Drops every root that another root already covers.
///
/// The sort is what makes this correct: **shortest path first**, by component
/// count, then alphabetically. An ancestor always has fewer components than its
/// descendants, so it is always considered — and accepted — before them. Sorting
/// alphabetically alone would not do: `/a/b` sorts before `/a/bb`, but so does
/// `/a/b/c` before `/a/bb`, and the descendant would then be weighed against a
/// set that did not yet hold its ancestor.
///
/// The tie-break on equal length is what keeps the answer stable, so two runs
/// over the same settings scan the same directories in the same order.
#[must_use]
pub fn compact_subtrees(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut sorted = paths;
    sorted.sort_by(|left, right| {
        let by_depth = left.components().count().cmp(&right.components().count());
        by_depth.then_with(|| left.cmp(right))
    });

    let mut compacted: Vec<PathBuf> = Vec::with_capacity(sorted.len());

    for path in sorted {
        if !is_covered_by_any(&path, &compacted) {
            compacted.push(path);
        }
    }

    compacted
}

/// The size to record for an entry, or `None` when there is not one to record.
///
/// A directory has **no** size rather than a size of zero: its entry's size on
/// disk is an implementation detail of the filesystem, and recording it would
/// let a search for large files return directories.
///
/// A size that does not fit in a signed 64-bit integer is also `None`, because
/// the column is signed and a wrapped value would read as negative — a file
/// larger than eight exabytes is not a real case, but a wrong sign is a worse
/// answer than an absent one.
#[must_use]
pub fn file_size_for(size_bytes: Option<u64>, is_directory: bool) -> Option<i64> {
    if is_directory {
        return None;
    }

    size_bytes.and_then(|size| i64::try_from(size).ok())
}
