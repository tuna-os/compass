//! Deciding what an incremental scan re-reads.
//!
//! Ports `IncrementalScanner`. A full scan reads everything and needs no
//! decisions; an incremental one exists to read as little as possible, and
//! every rule here is about where the line falls. Reading too little loses
//! files from the index silently, which is the worse failure of the two: a
//! search simply does not find them, and nothing says why.
//!
//! What the disk and the index hold is supplied by the caller, so the rules can
//! be tested against trees that would be tedious to build and impossible to
//! time.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// Which shape of incremental scan to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Walk the whole tree, re-reading the directories that changed, then
    /// follow anything new that turns up.
    Exhaustive,
    /// Start at the scan path and follow only what changed, never walking a
    /// branch that did not.
    Pruned,
}

/// What the index already knows, as the scanner asks it.
pub trait Index {
    /// When the last successful scan of this exact path finished, in unix
    /// seconds. `None` means this path has never been scanned to completion.
    fn last_successful_scan(&self, path: &Path) -> Option<u64>;

    /// Whether the index holds a row for this path.
    fn tracks_file(&self, path: &Path) -> bool;

    /// The paths the index holds as direct children of this directory.
    fn indexed_directory_files(&self, dir: &Path) -> Vec<PathBuf>;
}

/// Whether a directory has to be re-read.
///
/// Two rules, and the `||` between them is the point: **either** it changed
/// since the cut-off **or** the index has never heard of it. A directory
/// created long ago and only now brought into the scan path — a mounted disk, a
/// restored backup, a moved folder that kept its timestamps — is old and
/// unknown, and an mtime test alone would walk straight past it.
///
/// The comparison is `>=`, not `>`. Filesystem timestamps and scan records both
/// land on whole seconds, so a directory written in the same second the scan
/// recorded its success would otherwise be dropped. Re-reading a directory
/// needlessly costs a listing; not reading it loses its files until something
/// else touches it.
///
/// A timestamp that cannot be read answers **yes**, on the same asymmetry: the
/// cost of being wrong in one direction is a wasted listing, and in the other a
/// file nobody can find.
#[must_use]
pub fn should_process(last_modified: Option<u64>, cut_off: u64, tracked: bool) -> bool {
    match last_modified {
        Some(modified) => modified >= cut_off || !tracked,
        None => true,
    }
}

/// The cut-off a pruned scan measures against.
///
/// Walks **up** from the scan path until a directory with a successful scan
/// turns up, because a scan of `~` is what makes `~/code/project` up to date;
/// asking only about the exact path would find nothing and re-read everything.
///
/// With no record anywhere the cut-off is **0**, which makes every timestamp
/// newer and the pruned scan read the whole tree. That is the right answer for
/// an index that has never been built.
#[must_use]
pub fn cut_off_for(index: &impl Index, scan_path: &Path) -> u64 {
    let mut path = scan_path;

    loop {
        if let Some(finished) = index.last_successful_scan(path) {
            return finished;
        }

        match path.parent() {
            Some(parent) if parent != path => path = parent,
            _ => return 0,
        }
    }
}

/// What one directory's re-read changes in the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryScan {
    /// The directory itself, always re-indexed so its own timestamp moves.
    pub directory: PathBuf,
    /// Its entries, as the walk reached them.
    pub entries: Vec<PathBuf>,
    /// Paths the index holds that are no longer there.
    pub deleted: Vec<PathBuf>,
    /// The entries the index had never seen.
    pub new_entries: Vec<PathBuf>,
}

impl DirectoryScan {
    /// How many rows the scan reports as processed.
    ///
    /// The C++ counts *events*, and the directory's own event is one of them,
    /// so an empty directory still counts as 1. The progress figure is the
    /// number of things written, not the number of files found.
    #[must_use]
    pub fn processed_count(&self) -> usize {
        self.entries.len() + 1
    }
}

/// Reads one directory and works out what changed in it.
///
/// `present` is what the walk found there now. Anything the index holds for
/// this directory and the walk did not find is deleted — which is the only way
/// a removed file ever leaves the index, since nothing tells the scanner that a
/// file it never sees again is gone.
#[must_use]
pub fn scan_directory(index: &impl Index, directory: &Path, present: &[PathBuf]) -> DirectoryScan {
    let indexed = index.indexed_directory_files(directory);
    let here: HashSet<&PathBuf> = present.iter().collect();

    DirectoryScan {
        directory: directory.to_path_buf(),
        entries: present.to_vec(),
        deleted: indexed
            .into_iter()
            .filter(|path| !here.contains(path))
            .collect(),
        new_entries: present
            .iter()
            .filter(|path| !index.tracks_file(path))
            .cloned()
            .collect(),
    }
}

/// The directories an exhaustive incremental scan starts from.
///
/// The scan path is **always** first, whatever its timestamp says: it is the
/// one directory the scan was asked about, and skipping it would make a scan of
/// an unchanged directory do nothing at all.
///
/// With no successful scan on record for it, that is the whole answer — the
/// tree is not walked. An incremental scan with nothing to be incremental
/// against has no cut-off to compare to, and walking with a cut-off of 0 would
/// re-read everything under the guise of an incremental pass.
#[must_use]
pub fn scannable_directories(
    index: &impl Index,
    scan_path: &Path,
    candidates: &[(PathBuf, Option<u64>)],
) -> Vec<PathBuf> {
    let mut directories = vec![scan_path.to_path_buf()];

    let Some(cut_off) = index.last_successful_scan(scan_path) else {
        return directories;
    };

    directories.extend(
        candidates
            .iter()
            .filter(|(path, modified)| should_process(*modified, cut_off, index.tracks_file(path)))
            .map(|(path, _)| path.clone()),
    );

    directories
}

/// The order in which directories are read, once the starting set is known.
///
/// Each directory that is read can turn up more to read: an exhaustive scan
/// follows every **new** subdirectory, a pruned one follows every subdirectory
/// that is new *or* changed. The queue is drained first-in-first-out, and a
/// directory reached twice is read once — two paths into the same directory is
/// ordinary in a tree with links or overlapping scan roots.
///
/// **One deliberate difference.** The C++ dedupes only the directories it
/// *discovers*, not the ones it starts with: its first loop reads every
/// starting directory unconditionally. Here the check covers both. The
/// difference is unreachable through [`scannable_directories`], which cannot
/// return a path twice, and the uniform rule is the one worth having if a
/// caller ever does hand it a repeat.
#[must_use]
pub fn scan_order(
    start: Vec<PathBuf>,
    mut discover: impl FnMut(&Path) -> Vec<PathBuf>,
) -> Vec<PathBuf> {
    let mut pending: VecDeque<PathBuf> = start.into();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut order = Vec::new();

    while let Some(directory) = pending.pop_front() {
        if !seen.insert(directory.clone()) {
            continue;
        }

        pending.extend(discover(&directory));
        order.push(directory);
    }

    order
}
