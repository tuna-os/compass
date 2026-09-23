//! The shared half of every scanner.
//!
//! Ports `AbstractScanner`: opening the scan record, throttled progress
//! reports, and the finish/fail/interrupt bookkeeping. Concrete scanners
//! own one of these and call into it; the C++ spells that inheritance, Rust
//! spells it composition.

use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant, SystemTime};

use crate::db_writer::{DbWriter, FileEvent, FileEventType, IndexDatabase, ScanRecord, ScanStatus};
use crate::scan::Scan;

/// Progress reports closer together than this collapse into one.
const PROGRESS_NOTIFY_INTERVAL: Duration = Duration::from_millis(500);

/// Called with the scan's status and how many files it has processed.
pub type StatusCallback = Box<dyn FnMut(ScanStatus, usize) + Send>;

/// The shared half of every scanner: the record, the count, the flag.
///
/// Owns a reference to the writer, the scan, and the status callback.
/// `run` and the interrupt signal live on the concrete scanner; everything
/// here is the part they share.
pub struct Scanner<D: IndexDatabase> {
    writer: Arc<DbWriter<D>>,
    scan: Scan,
    on_status: StatusCallback,
    record_id: Option<i32>,
    processed: usize,
    last_notify: Option<Instant>,
    interrupted: AtomicBool,
}

impl<D: IndexDatabase> Scanner<D> {
    /// Tracks `scan` against `writer`, reporting to `on_status`.
    pub fn new(writer: Arc<DbWriter<D>>, scan: Scan, on_status: StatusCallback) -> Self {
        Self {
            writer,
            scan,
            on_status,
            record_id: None,
            processed: 0,
            last_notify: None,
            interrupted: AtomicBool::new(false),
        }
    }

    /// The scan being tracked.
    #[must_use]
    pub fn scan(&self) -> &Scan {
        &self.scan
    }

    /// How many files the scan has processed.
    #[must_use]
    pub fn processed_count(&self) -> usize {
        self.processed
    }

    /// Opens the scan record and reports `Started`. Without a record the
    /// scan reports `Failed` and there is nothing to finalize later — the
    /// C++ leaves the record id at -1 in exactly this case.
    pub fn start(&mut self) {
        match self
            .writer
            .create_scan(&self.scan.path, self.scan.scan_type())
        {
            Ok(record) => {
                self.record_id = Some(record.id);
                self.writer
                    .update_scan_status(record.id, ScanStatus::Started);
                self.last_notify = Some(Instant::now());
                (self.on_status)(ScanStatus::Started, self.processed);
            }
            Err(error) => {
                tracing::warn!(
                    path = ?self.scan.path,
                    error = ?error,
                    "not scanning: the scan record failed to open"
                );
                (self.on_status)(ScanStatus::Failed, self.processed);
            }
        }
    }

    /// Adds `count` processed files, reporting `Started` when the last
    /// report is over half a second old.
    pub fn report_progress(&mut self, count: usize) {
        self.processed += count;
        let due = self
            .last_notify
            .is_none_or(|last| last.elapsed() >= PROGRESS_NOTIFY_INTERVAL);
        if !due {
            return;
        }
        self.last_notify = Some(Instant::now());
        (self.on_status)(ScanStatus::Started, self.processed);
    }

    /// Closes the record with `Succeeded` — or `Interrupted` when flagged —
    /// and reports it.
    pub fn finish(&mut self) {
        let status = if self.is_interrupted() {
            ScanStatus::Interrupted
        } else {
            ScanStatus::Succeeded
        };
        if let Some(record_id) = self.record_id {
            self.writer.finalize_scan(
                record_id,
                status,
                i64::try_from(self.processed).unwrap_or(i64::MAX),
            );
        }
        (self.on_status)(status, self.processed);
    }

    /// Closes the record as `Failed` and reports it.
    pub fn fail(&mut self) {
        if let Some(record_id) = self.record_id {
            self.writer.finalize_scan(
                record_id,
                ScanStatus::Failed,
                i64::try_from(self.processed).unwrap_or(i64::MAX),
            );
        }
        (self.on_status)(ScanStatus::Failed, self.processed);
    }

    /// Queues scanner-found changes for the writer, the way concrete C++
    /// scanners call `m_writer->indexEvents` on the shared half.
    pub fn index_events(&self, events: Vec<FileEvent>) {
        self.writer.index_events(events);
    }

    /// Queues forgetting `paths`, the way concrete C++ scanners call
    /// `m_writer->deleteIndexedFiles` on the shared half.
    pub fn delete_indexed_files(&self, paths: Vec<PathBuf>) {
        self.writer.delete_indexed_files(paths, None);
    }

    /// Flags the scan interrupted. The next [`Scanner::finish`] closes it as
    /// such; scanners also poll [`Scanner::is_interrupted`] to stop early.
    pub fn interrupt(&self) {
        self.interrupted.store(true, Ordering::SeqCst);
    }

    /// Whether [`Scanner::interrupt`] has been called.
    #[must_use]
    pub fn is_interrupted(&self) -> bool {
        self.interrupted.load(Ordering::SeqCst)
    }
}

/// A directory `Modify` for a scan root, sized nothing: every concrete scan
/// reports the root before its entries, the way both C++ scanners do.
pub(crate) fn root_event(root: &Path) -> FileEvent {
    FileEvent {
        event_type: FileEventType::Modify,
        path: root.to_path_buf(),
        event_time: modified_at(root),
        is_directory: true,
        size_bytes: None,
    }
}

/// A `Modify` for one walked path, with its write time, directory flag and
/// size.
pub(crate) fn file_event(path: PathBuf, is_directory: bool) -> FileEvent {
    FileEvent {
        event_type: FileEventType::Modify,
        path: path.clone(),
        event_time: modified_at(&path),
        is_directory,
        size_bytes: size_bytes(&path, is_directory),
    }
}

/// The write time, or the epoch when the file will not say: the C++ reads
/// with an error code and keeps whatever it got, which is also a fallback.
pub(crate) fn modified_at(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

/// The write time in unix seconds, or nothing when the disk will not say.
pub(crate) fn modified_seconds(path: &Path) -> Option<i64> {
    let seconds = std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();
    i64::try_from(seconds).ok()
}

/// The size, or nothing for directories, unreadable files and overflows:
/// `fileSizeBytesFor` in `util.hpp`.
pub(crate) fn size_bytes(path: &Path, is_directory: bool) -> Option<i64> {
    if is_directory {
        return None;
    }
    let size = std::fs::metadata(path).ok()?.len();
    if size > i64::MAX as u64 {
        return None;
    }
    Some(size as i64)
}

/// `$HOME`, or nothing when it is unset or empty: `homeDir` in `util.hpp`.
pub(crate) fn home_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    (!home.as_os_str().is_empty()).then_some(home)
}

/// A scan record's timestamp as the cutoff its successors compare against.
/// The record stores it unsigned; the absurd end of the range scans nothing
/// but unindexed files rather than wrapping.
pub(crate) fn cutoff_seconds(record: &ScanRecord) -> i64 {
    i64::try_from(record.created_at).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::{Condvar, Mutex, PoisonError};

    use crate::db_writer::{FileEvent, ScanRecord, ScanType};
    use crate::scan::{FullScan, ScanData};

    /// A database recording what ran, answering scans on demand.
    struct FakeDb {
        log: Vec<String>,
        create: Result<ScanRecord, String>,
    }

    impl FakeDb {
        fn record(&mut self, what: String) {
            self.log.push(what);
        }
    }

    impl IndexDatabase for FakeDb {
        fn is_open(&self) -> bool {
            true
        }

        fn update_scan_status(&mut self, scan_id: i32, status: ScanStatus) -> bool {
            self.record(format!("status {scan_id} {status:?}"));
            true
        }

        fn finalize_scan(&mut self, scan_id: i32, status: ScanStatus, count: i64) -> bool {
            self.record(format!("finalize {scan_id} {status:?} {count}"));
            true
        }

        fn set_scan_error(&mut self, scan_id: i32, error: &str) -> bool {
            self.record(format!("error {scan_id} {error}"));
            true
        }

        fn prune_scan_history(&mut self, max_age_seconds: i64) -> bool {
            self.record(format!("prune {max_age_seconds}"));
            true
        }

        fn create_scan(&mut self, path: &Path, scan_type: ScanType) -> Result<ScanRecord, String> {
            self.record(format!("create {} {scan_type:?}", path.display()));
            self.create.clone()
        }

        fn index_files(&mut self, _paths: &[PathBuf]) {}

        fn delete_indexed_files(&mut self, _paths: &[PathBuf]) {}

        fn delete_all_indexed_files(&mut self) {}

        fn compact(&mut self) {}

        fn needs_compaction(&self) -> bool {
            false
        }

        fn rebuild_spellfix_vocabulary(&mut self) {}

        fn index_events(&mut self, _events: &[FileEvent]) {}
    }

    fn scan() -> Scan {
        Scan {
            path: PathBuf::from("/home/ada"),
            data: ScanData::Full(FullScan {
                excluded_paths: Vec::new(),
            }),
            notify: false,
        }
    }

    fn record() -> ScanRecord {
        ScanRecord {
            id: 7,
            status: ScanStatus::Pending,
            created_at: 0,
            finished_at: 0,
            indexed_file_count: 0,
            path: PathBuf::from("/home/ada"),
            scan_type: ScanType::Full,
        }
    }

    /// The status reports a scan produced, in order, shared with its callback.
    type Reports = Arc<(Mutex<Vec<(ScanStatus, usize)>>, Condvar)>;

    /// A writer plus the status reports it produced, in order.
    struct Harness {
        writer: Arc<DbWriter<FakeDb>>,
        reports: Reports,
    }

    impl Harness {
        fn new(create: Result<ScanRecord, String>) -> Self {
            let reports = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
            let writer = Arc::new(DbWriter::new(move || FakeDb {
                log: Vec::new(),
                create,
            }));
            Self { writer, reports }
        }

        fn scanner(&self) -> Scanner<FakeDb> {
            let reports = Arc::clone(&self.reports);
            Scanner::new(
                Arc::clone(&self.writer),
                scan(),
                Box::new(move |status, count| {
                    let (lock, changed) = &*reports;
                    lock.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push((status, count));
                    changed.notify_all();
                }),
            )
        }

        fn reported(&self) -> Vec<(ScanStatus, usize)> {
            self.reports
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    }

    #[test]
    fn a_failed_record_reports_failed_with_nothing_to_finalize() {
        let harness = Harness::new(Err("no database".to_owned()));
        let mut scanner = harness.scanner();
        scanner.start();
        scanner.finish();
        assert_eq!(
            harness.reported(),
            [(ScanStatus::Failed, 0), (ScanStatus::Succeeded, 0)]
        );
    }

    #[test]
    fn start_reports_started_and_finish_closes_succeeded() {
        let harness = Harness::new(Ok(record()));
        let mut scanner = harness.scanner();
        scanner.start();
        scanner.report_progress(3);
        scanner.finish();
        let reported = harness.reported();
        assert_eq!(reported.first(), Some(&(ScanStatus::Started, 0)));
        assert_eq!(reported.last(), Some(&(ScanStatus::Succeeded, 3)));
        assert_eq!(scanner.processed_count(), 3);
    }

    #[test]
    fn quick_reports_collapse_into_one() {
        let harness = Harness::new(Ok(record()));
        let mut scanner = harness.scanner();
        scanner.start();
        scanner.report_progress(1);
        scanner.report_progress(1);
        assert_eq!(harness.reported().len(), 1);
    }

    #[test]
    fn interrupt_turns_finish_into_interrupted() {
        let harness = Harness::new(Ok(record()));
        let mut scanner = harness.scanner();
        assert!(!scanner.is_interrupted());
        scanner.start();
        scanner.interrupt();
        assert!(scanner.is_interrupted());
        scanner.finish();
        assert_eq!(
            harness.reported().last(),
            Some(&(ScanStatus::Interrupted, 0))
        );
    }

    #[test]
    fn fail_reports_failed() {
        let harness = Harness::new(Ok(record()));
        let mut scanner = harness.scanner();
        scanner.start();
        scanner.report_progress(2);
        scanner.fail();
        assert_eq!(harness.reported().last(), Some(&(ScanStatus::Failed, 2)));
    }

    #[test]
    fn scan_types_follow_the_data() {
        assert_eq!(scan().scan_type(), ScanType::Full);
    }
}
