//! What an incremental scan re-reads.
//!
//! Ported from `IncrementalScanner`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use compass_core::incremental_scan::{
    Index, cut_off_for, scan_directory, scan_order, scannable_directories, should_process,
};

const SCAN_AT: u64 = 1_000;

/// An index held in memory.
#[derive(Default)]
struct FakeIndex {
    scans: HashMap<PathBuf, u64>,
    tracked: Vec<PathBuf>,
    children: HashMap<PathBuf, Vec<PathBuf>>,
}

impl FakeIndex {
    fn scanned(mut self, path: &str, at: u64) -> Self {
        self.scans.insert(PathBuf::from(path), at);
        self
    }

    fn tracking(mut self, paths: &[&str]) -> Self {
        self.tracked.extend(paths.iter().map(PathBuf::from));
        self
    }

    fn holding(mut self, dir: &str, children: &[&str]) -> Self {
        self.children.insert(
            PathBuf::from(dir),
            children.iter().map(PathBuf::from).collect(),
        );
        self
    }
}

impl Index for FakeIndex {
    fn last_successful_scan(&self, path: &Path) -> Option<u64> {
        self.scans.get(path).copied()
    }

    fn tracks_file(&self, path: &Path) -> bool {
        self.tracked.iter().any(|item| item == path)
    }

    fn indexed_directory_files(&self, dir: &Path) -> Vec<PathBuf> {
        self.children.get(dir).cloned().unwrap_or_default()
    }
}

fn paths(items: &[PathBuf]) -> Vec<String> {
    items
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn a_directory_changed_since_the_cut_off_is_re_read() {
    assert!(should_process(Some(SCAN_AT + 1), SCAN_AT, true));
}

#[test]
fn a_directory_untouched_since_the_cut_off_is_left_alone() {
    assert!(!should_process(Some(SCAN_AT - 1), SCAN_AT, true));
}

#[test]
fn a_directory_written_in_the_same_second_as_the_scan_is_re_read() {
    // `>=`, not `>`. Both timestamps land on whole seconds, so a strict
    // comparison drops a directory written while the scan was finishing —
    // exactly the one most likely to have changed.
    assert!(should_process(Some(SCAN_AT), SCAN_AT, true));
}

#[test]
fn a_directory_the_index_has_never_seen_is_re_read_however_old_it_is() {
    // A mounted disk, a restored backup, a moved folder that kept its
    // timestamps: old and unknown. An mtime test alone walks straight past it.
    assert!(should_process(Some(1), SCAN_AT, false));
}

#[test]
fn a_timestamp_that_cannot_be_read_is_re_read() {
    assert!(should_process(None, SCAN_AT, true));
}

#[test]
fn the_cut_off_comes_from_the_scan_path_itself_when_it_has_one() {
    let index = FakeIndex::default()
        .scanned("/home/u", 500)
        .scanned("/home/u/code", 900);

    assert_eq!(cut_off_for(&index, Path::new("/home/u/code")), 900);
}

#[test]
fn the_cut_off_is_inherited_from_an_ancestor() {
    // A scan of `~` is what makes `~/code/project` up to date. Asking only
    // about the exact path finds nothing and re-reads the whole tree.
    let index = FakeIndex::default().scanned("/home/u", 500);

    assert_eq!(cut_off_for(&index, Path::new("/home/u/code/project")), 500);
}

#[test]
fn the_nearest_ancestor_wins() {
    let index = FakeIndex::default()
        .scanned("/home/u", 500)
        .scanned("/home/u/code", 900);

    assert_eq!(cut_off_for(&index, Path::new("/home/u/code/project")), 900);
}

#[test]
fn no_scan_anywhere_above_means_everything_is_newer() {
    let index = FakeIndex::default();

    assert_eq!(cut_off_for(&index, Path::new("/home/u/code")), 0);
    // Which is to say: a pruned scan against an index that has never been
    // built reads the whole tree, rather than nothing.
    assert!(should_process(Some(1), 0, true));
}

#[test]
fn the_scan_path_is_always_read_however_recently_it_was_scanned() {
    let index = FakeIndex::default().scanned("/root", SCAN_AT);

    let directories = scannable_directories(&index, Path::new("/root"), &[]);

    assert_eq!(paths(&directories), ["/root"]);
}

#[test]
fn a_path_never_scanned_to_completion_yields_only_itself() {
    // There is no cut-off to be incremental against, and walking with one of 0
    // would re-read the tree while calling itself an incremental pass.
    let index = FakeIndex::default();

    let directories = scannable_directories(
        &index,
        Path::new("/root"),
        &[(PathBuf::from("/root/changed"), Some(SCAN_AT + 10))],
    );

    assert_eq!(paths(&directories), ["/root"]);
}

#[test]
fn only_the_changed_candidates_are_read() {
    let index = FakeIndex::default()
        .scanned("/root", SCAN_AT)
        .tracking(&["/root/old", "/root/changed"]);

    let directories = scannable_directories(
        &index,
        Path::new("/root"),
        &[
            (PathBuf::from("/root/old"), Some(SCAN_AT - 100)),
            (PathBuf::from("/root/changed"), Some(SCAN_AT + 100)),
        ],
    );

    assert_eq!(paths(&directories), ["/root", "/root/changed"]);
}

#[test]
fn an_untracked_candidate_is_read_even_when_it_is_old() {
    let index = FakeIndex::default().scanned("/root", SCAN_AT);

    let directories = scannable_directories(
        &index,
        Path::new("/root"),
        &[(PathBuf::from("/root/restored"), Some(SCAN_AT - 100))],
    );

    assert_eq!(paths(&directories), ["/root", "/root/restored"]);
}

#[test]
fn a_file_the_index_holds_and_the_disk_lost_is_deleted() {
    // Nothing tells the scanner a file is gone. Missing from a listing it
    // re-read is the only evidence there is.
    let index = FakeIndex::default().holding("/root", &["/root/a.txt", "/root/gone.txt"]);

    let scan = scan_directory(&index, Path::new("/root"), &[PathBuf::from("/root/a.txt")]);

    assert_eq!(paths(&scan.deleted), ["/root/gone.txt"]);
}

#[test]
fn a_file_still_there_is_not_deleted() {
    let index = FakeIndex::default().holding("/root", &["/root/a.txt"]);

    let scan = scan_directory(&index, Path::new("/root"), &[PathBuf::from("/root/a.txt")]);

    assert!(scan.deleted.is_empty());
}

#[test]
fn an_entry_the_index_has_never_seen_is_reported_as_new() {
    let index = FakeIndex::default()
        .holding("/root", &["/root/a.txt"])
        .tracking(&["/root/a.txt"]);

    let scan = scan_directory(
        &index,
        Path::new("/root"),
        &[PathBuf::from("/root/a.txt"), PathBuf::from("/root/b.txt")],
    );

    assert_eq!(paths(&scan.new_entries), ["/root/b.txt"]);
}

#[test]
fn the_directory_itself_is_part_of_what_was_processed() {
    // The C++ counts events, and the directory's own event is one of them, so
    // an empty directory still counts as one thing processed.
    let index = FakeIndex::default();

    let scan = scan_directory(&index, Path::new("/root"), &[]);

    assert_eq!(scan.directory, PathBuf::from("/root"));
    assert_eq!(scan.processed_count(), 1);
}

#[test]
fn every_entry_counts_towards_what_was_processed() {
    let index = FakeIndex::default();

    let scan = scan_directory(
        &index,
        Path::new("/root"),
        &[PathBuf::from("/root/a"), PathBuf::from("/root/b")],
    );

    assert_eq!(scan.processed_count(), 3);
}

#[test]
fn a_scan_follows_what_reading_a_directory_turns_up() {
    let discovered: HashMap<PathBuf, Vec<PathBuf>> =
        HashMap::from([(PathBuf::from("/root"), vec![PathBuf::from("/root/new")])]);

    let order = scan_order(vec![PathBuf::from("/root")], |dir| {
        discovered.get(dir).cloned().unwrap_or_default()
    });

    assert_eq!(paths(&order), ["/root", "/root/new"]);
}

#[test]
fn a_directory_reached_twice_is_read_once() {
    let discovered: HashMap<PathBuf, Vec<PathBuf>> = HashMap::from([
        (
            PathBuf::from("/root/a"),
            vec![PathBuf::from("/root/shared")],
        ),
        (
            PathBuf::from("/root/b"),
            vec![PathBuf::from("/root/shared")],
        ),
    ]);

    let order = scan_order(
        vec![PathBuf::from("/root/a"), PathBuf::from("/root/b")],
        |dir| discovered.get(dir).cloned().unwrap_or_default(),
    );

    assert_eq!(paths(&order), ["/root/a", "/root/b", "/root/shared"]);
}

#[test]
fn a_directory_that_discovers_itself_does_not_loop() {
    let order = scan_order(vec![PathBuf::from("/root")], |dir| vec![dir.to_path_buf()]);

    assert_eq!(paths(&order), ["/root"]);
}

#[test]
fn a_starting_set_with_a_repeat_reads_it_once() {
    let order = scan_order(vec![PathBuf::from("/root"), PathBuf::from("/root")], |_| {
        Vec::new()
    });

    assert_eq!(paths(&order), ["/root"]);
}

#[test]
fn what_a_directory_turns_up_is_read_after_everything_already_queued() {
    // First in, first out: the starting set is read before anything it
    // discovers, so a scan interrupted early has covered the breadth it was
    // asked about rather than one deep branch of it.
    let discovered: HashMap<PathBuf, Vec<PathBuf>> = HashMap::from([(
        PathBuf::from("/root/a"),
        vec![PathBuf::from("/root/a/deep")],
    )]);

    let order = scan_order(
        vec![PathBuf::from("/root/a"), PathBuf::from("/root/b")],
        |dir| discovered.get(dir).cloned().unwrap_or_default(),
    );

    assert_eq!(paths(&order), ["/root/a", "/root/b", "/root/a/deep"]);
}
