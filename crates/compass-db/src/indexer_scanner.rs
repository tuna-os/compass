//! The full-scan concrete scanner.
//!
//! Ports `IndexerScanner`: the root goes out as a directory `Modify` first,
//! every walked entry follows as a `Modify` with its write time, directory
//! flag and size, and batches flush to the writer every `INDEX_BATCH_SIZE`
//! events with a final flush for the remainder. Progress counts entries, not
//! the root, exactly like the C++ `reportProgress` calls.
//!
//! The shared half — opening the scan record, throttled progress, finish and
//! fail — lives in [`Scanner`]; this struct owns one alongside its walker,
//! the way the C++ inherits `AbstractScanner`, and interrupts both together.

use std::path::Path;
use std::sync::Arc;

use compass_core::file_walk::IndexWalk;

use crate::db_writer::{DbWriter, IndexDatabase};
use crate::scan::{FullScan, Scan, ScanData};
use crate::scanner::{Scanner, StatusCallback, file_event, home_dir, root_event};

/// Events per writer flush: `IndexerScanner::INDEX_BATCH_SIZE`.
const INDEX_BATCH_SIZE: usize = 5_000;

/// A full scan: walk everything, index what the walk finds.
pub struct IndexerScanner<D: IndexDatabase> {
    core: Scanner<D>,
    walker: IndexWalk,
    /// Flush threshold. Production uses [`INDEX_BATCH_SIZE`]; tests shrink
    /// it to watch batches split without indexing thousands of files.
    batch_size: usize,
}

impl<D: IndexDatabase> IndexerScanner<D> {
    /// Tracks `scan` against `writer`, reporting to `on_status`, walking
    /// with home-relative exclusions resolved the way `EntryFilter` does.
    pub fn new(writer: Arc<DbWriter<D>>, scan: Scan, on_status: StatusCallback) -> Self {
        Self {
            core: Scanner::new(writer, scan, on_status),
            walker: IndexWalk::new(home_dir().as_deref()),
            batch_size: INDEX_BATCH_SIZE,
        }
    }

    /// Runs the scan to `Succeeded` — or `Interrupted` when flagged, `Failed`
    /// when the scan is not a full one. The C++ throws on the wrong variant
    /// and fails from the catch; there is nothing to catch here, so the
    /// shape check fails directly.
    pub fn run(&mut self) {
        self.core.start();
        let scan = self.core.scan().clone();
        match scan.data {
            ScanData::Full(ref full) => {
                self.scan_full(&scan.path, full);
                self.core.finish();
            }
            ScanData::Incremental(_) => self.core.fail(),
        }
    }

    /// Flags the scan and stops its walk, like `IndexerScanner::interrupt`
    /// setting the flag and stopping `m_walker` together.
    pub fn interrupt(&self) {
        self.core.interrupt();
        self.walker.stop();
    }

    /// Walks `root`, batching a directory `Modify` for the root and one file
    /// event per entry into the writer.
    fn scan_full(&mut self, root: &Path, full: &FullScan) {
        self.walker.set_excluded_paths(full.excluded_paths.clone());
        let mut batched = vec![root_event(root)];
        let batch_size = self.batch_size;
        let core = &mut self.core;
        self.walker.walk(root, |entry| {
            core.report_progress(1);
            batched.push(file_event(entry.path.clone(), entry.is_directory));
            if batched.len() >= batch_size {
                core.index_events(std::mem::take(&mut batched));
                batched.reserve(batch_size);
            }
        });
        core.index_events(batched);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::{Mutex, PoisonError};
    use std::time::{Duration, SystemTime};

    use crate::db_writer::{FileEvent, IndexDatabase, ScanRecord, ScanStatus, ScanType};
    use crate::scan::{FullScan, IncrementalScan, ScanMode};
    use crate::scanner::{modified_at, size_bytes};

    /// A database recording batches and events, in order.
    struct RecordingDb {
        batches: Arc<Mutex<Vec<usize>>>,
        events: Arc<Mutex<Vec<FileEvent>>>,
    }

    impl IndexDatabase for RecordingDb {
        fn is_open(&self) -> bool {
            true
        }

        fn update_scan_status(&mut self, _scan_id: i32, _status: ScanStatus) -> bool {
            true
        }

        fn finalize_scan(&mut self, _scan_id: i32, _status: ScanStatus, _count: i64) -> bool {
            true
        }

        fn set_scan_error(&mut self, _scan_id: i32, _error: &str) -> bool {
            true
        }

        fn prune_scan_history(&mut self, _max_age_seconds: i64) -> bool {
            true
        }

        fn create_scan(&mut self, path: &Path, scan_type: ScanType) -> Result<ScanRecord, String> {
            Ok(ScanRecord {
                id: 7,
                status: ScanStatus::Pending,
                created_at: 0,
                finished_at: 0,
                indexed_file_count: 0,
                path: path.to_path_buf(),
                scan_type,
            })
        }

        fn index_files(&mut self, _paths: &[PathBuf]) {}

        fn delete_indexed_files(&mut self, _paths: &[PathBuf]) {}

        fn delete_all_indexed_files(&mut self) {}

        fn compact(&mut self) {}

        fn needs_compaction(&self) -> bool {
            false
        }

        fn rebuild_spellfix_vocabulary(&mut self) {}

        fn index_events(&mut self, events: &[FileEvent]) {
            self.events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(events.iter().cloned());
            self.batches
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(events.len());
        }
    }

    struct Harness {
        writer: Arc<DbWriter<RecordingDb>>,
        batches: Arc<Mutex<Vec<usize>>>,
        events: Arc<Mutex<Vec<FileEvent>>>,
        statuses: Arc<Mutex<Vec<(ScanStatus, usize)>>>,
    }

    impl Harness {
        fn new() -> Self {
            let batches = Arc::new(Mutex::new(Vec::new()));
            let events = Arc::new(Mutex::new(Vec::new()));
            let maker_batches = Arc::clone(&batches);
            let maker_events = Arc::clone(&events);
            let writer = Arc::new(DbWriter::new(move || RecordingDb {
                batches: maker_batches,
                events: maker_events,
            }));
            Self {
                writer,
                batches,
                events,
                statuses: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn scanner(&self, data: ScanData, root: &Path) -> IndexerScanner<RecordingDb> {
            let statuses = Arc::clone(&self.statuses);
            IndexerScanner::new(
                Arc::clone(&self.writer),
                Scan {
                    path: root.to_path_buf(),
                    data,
                    notify: false,
                },
                Box::new(move |status, count| {
                    statuses
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push((status, count));
                }),
            )
        }

        fn full_scan(&self, root: &Path) -> IndexerScanner<RecordingDb> {
            self.scanner(
                ScanData::Full(FullScan {
                    excluded_paths: Vec::new(),
                }),
                root,
            )
        }

        fn reported(&self) -> Vec<(ScanStatus, usize)> {
            self.statuses
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    }

    /// Waits until `locked` holds `count` entries, so assertions see finished
    /// writer work rather than racing it.
    fn wait_for<T: Clone>(locked: &Mutex<Vec<T>>, count: usize) -> Vec<T> {
        let start = std::time::Instant::now();
        loop {
            let entries = locked
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            if entries.len() >= count {
                return entries;
            }
            if start.elapsed() > Duration::from_secs(5) {
                panic!("timed out waiting for {count} entries");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn fixture() -> tempfile::TempDir {
        tempfile::tempdir().expect("temporary tree")
    }

    fn write(root: &Path, name: &str, contents: &str) {
        std::fs::write(root.join(name), contents).expect("seed file");
    }

    #[test]
    fn full_scan_indexes_the_root_and_every_entry_then_succeeds() {
        let dir = fixture();
        write(dir.path(), "a.txt", "notes");
        std::fs::create_dir(dir.path().join("sub")).expect("seed dir");
        write(&dir.path().join("sub"), "b.txt", "hi");

        let harness = Harness::new();
        let mut scanner = harness.full_scan(dir.path());
        scanner.run();

        assert_eq!(wait_for(&harness.batches, 1), [4]);
        let events = wait_for(&harness.events, 4);
        let paths: HashSet<&Path> = events.iter().map(|event| event.path.as_path()).collect();
        assert_eq!(
            paths,
            HashSet::from([
                dir.path(),
                &dir.path().join("a.txt"),
                &dir.path().join("sub"),
                &dir.path().join("sub/b.txt"),
            ])
        );
        for event in &events {
            if event.is_directory {
                assert_eq!(event.size_bytes, None);
            }
        }
        let file = events
            .iter()
            .find(|event| event.path == dir.path().join("a.txt"))
            .expect("the file is indexed");
        assert_eq!(file.size_bytes, Some(5));

        let reported = harness.reported();
        assert_eq!(reported.first(), Some(&(ScanStatus::Started, 0)));
        assert_eq!(reported.last(), Some(&(ScanStatus::Succeeded, 3)));
    }

    #[test]
    fn batches_split_at_the_configured_size() {
        let dir = fixture();
        for name in ["f1.txt", "f2.txt", "f3.txt", "f4.txt"] {
            write(dir.path(), name, "x");
        }

        let harness = Harness::new();
        let mut scanner = harness.full_scan(dir.path());
        scanner.batch_size = 2;
        scanner.run();

        // Root plus four files in twos: two full batches, one remainder.
        assert_eq!(wait_for(&harness.batches, 3), [2, 2, 1]);
        assert_eq!(wait_for(&harness.events, 5).len(), 5);
        assert_eq!(harness.reported().last(), Some(&(ScanStatus::Succeeded, 4)));
    }

    #[test]
    fn a_non_full_scan_fails_without_indexing() {
        let dir = fixture();
        write(dir.path(), "a.txt", "notes");

        let harness = Harness::new();
        let mut scanner = harness.scanner(
            ScanData::Incremental(IncrementalScan {
                mode: ScanMode::Exhaustive,
                max_depth: None,
                excluded_filenames: Vec::new(),
                excluded_paths: Vec::new(),
            }),
            dir.path(),
        );
        scanner.run();

        // The record opens before the shape check, like the C++ `start`
        // running before the throwing `std::get`: Started, then Failed.
        assert_eq!(
            harness.reported(),
            [(ScanStatus::Started, 0), (ScanStatus::Failed, 0)]
        );
        assert!(
            harness
                .batches
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_empty()
        );
    }

    #[test]
    fn a_stopped_walk_reports_interrupted_with_only_the_root() {
        let dir = fixture();
        write(dir.path(), "a.txt", "notes");

        let harness = Harness::new();
        let mut scanner = harness.full_scan(dir.path());
        scanner.interrupt();
        scanner.run();

        assert_eq!(wait_for(&harness.batches, 1), [1]);
        let events = wait_for(&harness.events, 1);
        assert_eq!(events[0].path, dir.path());
        assert!(events[0].is_directory);
        assert_eq!(
            harness.reported().last(),
            Some(&(ScanStatus::Interrupted, 0))
        );
    }

    #[test]
    fn unreadable_times_fall_back_to_the_epoch() {
        assert_eq!(
            modified_at(Path::new("/no/such/file/anywhere")),
            SystemTime::UNIX_EPOCH
        );
        assert_eq!(size_bytes(Path::new("/no/such/file/anywhere"), false), None);
    }
}
