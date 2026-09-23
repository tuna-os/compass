//! Serialized writes into the file index, behind a bounded queue.
//!
//! Ports `DbWriter`: one worker thread owns the database, every mutation
//! goes through it in submission order, bulk writes apply backpressure past
//! [`MAX_PENDING_BULK_WRITES`], and each item runs under the IO pacer's
//! checkpoint. The database itself is an [`IndexDatabase`] trait — the SQLite
//! port implements it later — so this discipline pins without a database file.
//!
//! # Deltas from the C++, on purpose
//!
//! * The C++ raises the worker thread's IO priority (`setBackgroundThreadPriority`,
//!   Linux-only). Thread priority has no portable spelling and the seam for it
//!   does not exist yet, so this slice does not set one.
//! * Shutdown drains the queue, so the C++'s submit-after-destroy — a
//!   use-after-free there — is unrepresentable here: dropping is the only way
//!   to stop, and it joins after everything queued runs. [`DbWriter::create_scan`]
//!   only fails when the worker thread itself died.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Condvar, Mutex, PoisonError,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread::{self, JoinHandle};
use std::time::SystemTime;

use compass_core::io_pacer::IoPacer;

/// How many bulk writes may wait before their submitters block.
pub const MAX_PENDING_BULK_WRITES: usize = 8;

/// Which shape of scan produced the indexed files.
///
/// Stored in SQLite by number: the discriminants are the C++ enum order, so
/// do not reorder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanType {
    /// Read everything.
    Full = 0,
    /// Read what changed.
    Incremental = 1,
}

/// Where a scan stands.
///
/// Stored in SQLite by number: the discriminants are the C++ enum order, so
/// do not reorder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanStatus {
    /// Recorded, not started.
    Pending = 0,
    /// Running.
    Started = 1,
    /// Stopped early.
    Interrupted = 2,
    /// Stopped with an error.
    Failed = 3,
    /// Finished.
    Succeeded = 4,
}

/// One row of the scan history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanRecord {
    /// The row id.
    pub id: i32,
    /// Where the scan stands.
    pub status: ScanStatus,
    /// When it started, in unix seconds.
    pub created_at: u64,
    /// When it finished, in unix seconds, 0 while running.
    pub finished_at: u64,
    /// How many files it indexed.
    pub indexed_file_count: i64,
    /// What was scanned.
    pub path: PathBuf,
    /// Which shape of scan.
    pub scan_type: ScanType,
}

/// What happened to a file between scans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileEventType {
    /// Contents or metadata changed.
    Modify,
    /// Gone.
    Delete,
}

/// One file the scanner found changed or missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEvent {
    /// What happened.
    pub event_type: FileEventType,
    /// Which file.
    pub path: PathBuf,
    /// When it was last written.
    pub event_time: SystemTime,
    /// Whether the path is a directory.
    pub is_directory: bool,
    /// Its size in bytes, when known.
    pub size_bytes: Option<i64>,
}

/// What the writer needs from the file-index database.
///
/// Mirrors the `FileIndexerDatabase` methods the C++ writer calls, so the
/// SQLite port implements this trait without the writer changing.
pub trait IndexDatabase: Send + 'static {
    /// Whether the database opened. A closed database logs once and carries
    /// on; queued writes fail inside the database, as they do in C++.
    fn is_open(&self) -> bool;
    /// Records a scan's status. Returns what the database reports.
    fn update_scan_status(&mut self, scan_id: i32, status: ScanStatus) -> bool;
    /// Closes a scan's record with its outcome and file count.
    fn finalize_scan(&mut self, scan_id: i32, status: ScanStatus, indexed_file_count: i64) -> bool;
    /// Records why a scan failed.
    fn set_scan_error(&mut self, scan_id: i32, error: &str) -> bool;
    /// Drops scan history older than a budget.
    fn prune_scan_history(&mut self, max_age_seconds: i64) -> bool;
    /// Opens a scan record.
    fn create_scan(&mut self, path: &Path, scan_type: ScanType) -> Result<ScanRecord, String>;
    /// Adds or refreshes indexed files.
    fn index_files(&mut self, paths: &[PathBuf]);
    /// Forgets indexed files.
    fn delete_indexed_files(&mut self, paths: &[PathBuf]);
    /// Forgets everything indexed.
    fn delete_all_indexed_files(&mut self);
    /// Reclaims space.
    fn compact(&mut self);
    /// Whether reclaiming would help.
    fn needs_compaction(&self) -> bool;
    /// Rebuilds the typo-correction vocabulary.
    fn rebuild_spellfix_vocabulary(&mut self);
    /// Applies scanner-found changes and removals.
    fn index_events(&mut self, events: &[FileEvent]);
}

/// One queued mutation and whether it counts against the bulk budget.
struct QueuedWork<D: IndexDatabase> {
    work: Box<dyn FnOnce(&mut D) + Send>,
    bounded: bool,
}

struct State<D: IndexDatabase> {
    queue: VecDeque<QueuedWork<D>>,
    pending_bulk: usize,
}

struct Shared<D: IndexDatabase> {
    state: Mutex<State<D>>,
    /// A queued item arrived.
    update: Condvar,
    /// A bulk slot freed up.
    not_full: Condvar,
    /// Cleared by shutdown; the worker drains what is queued first.
    active: AtomicBool,
    /// A vocabulary rebuild is queued or running.
    vocab_queued: AtomicBool,
}

/// One writer, one worker thread, one database.
///
/// `new` spawns the thread; dropping shuts it down, draining what is queued
/// first. Shared across scanner threads behind [`Arc`], so the join handle
/// sits behind a mutex: the handle is only ever taken at drop, but a plain
/// [`JoinHandle`] is not [`Sync`] and would pin the whole writer to one
/// thread.
pub struct DbWriter<D: IndexDatabase> {
    worker: Mutex<Option<JoinHandle<()>>>,
    shared: Arc<Shared<D>>,
}

impl<D: IndexDatabase> DbWriter<D> {
    /// Starts the writer, opening the database on the worker thread.
    pub fn new(make_db: impl FnOnce() -> D + Send + 'static) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                queue: VecDeque::new(),
                pending_bulk: 0,
            }),
            update: Condvar::new(),
            not_full: Condvar::new(),
            active: AtomicBool::new(true),
            vocab_queued: AtomicBool::new(false),
        });
        let worker_shared = Arc::clone(&shared);
        let worker = thread::spawn(move || {
            let mut db = make_db();
            if !db.is_open() {
                tracing::error!(
                    "file indexer writer database is not open, queued writes will fail"
                );
            }
            let mut pacer = IoPacer::new("/proc/pressure/io", 1);
            loop {
                let work = {
                    let mut state = worker_shared
                        .state
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    while state.queue.is_empty() && worker_shared.active.load(Ordering::SeqCst) {
                        state = worker_shared
                            .update
                            .wait(state)
                            .unwrap_or_else(PoisonError::into_inner);
                    }
                    if state.queue.is_empty() {
                        break;
                    }
                    state.queue.pop_front().expect("queue checked non-empty")
                };
                pacer.checkpoint();
                (work.work)(&mut db);
                if work.bounded {
                    let mut state = worker_shared
                        .state
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    state.pending_bulk -= 1;
                    worker_shared.not_full.notify_one();
                }
            }
        });
        Self {
            worker: Mutex::new(Some(worker)),
            shared,
        }
    }

    /// Queues a mutation. Bulk writes block past [`MAX_PENDING_BULK_WRITES`]
    /// until a slot frees up; the rest never block.
    pub fn submit(&self, work: impl FnOnce(&mut D) + Send + 'static, bounded: bool) {
        {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if bounded {
                while state.pending_bulk >= MAX_PENDING_BULK_WRITES
                    && self.shared.active.load(Ordering::SeqCst)
                {
                    state = self
                        .shared
                        .not_full
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                }
                state.pending_bulk += 1;
            }
            state.queue.push_back(QueuedWork {
                work: Box::new(work),
                bounded,
            });
        }
        self.shared.update.notify_one();
    }

    /// Records a scan's status.
    pub fn update_scan_status(&self, scan_id: i32, status: ScanStatus) {
        self.submit(
            move |db: &mut D| {
                db.update_scan_status(scan_id, status);
            },
            false,
        );
    }

    /// Closes a scan's record with its outcome and file count.
    pub fn finalize_scan(&self, scan_id: i32, status: ScanStatus, indexed_file_count: i64) {
        self.submit(
            move |db: &mut D| {
                db.finalize_scan(scan_id, status, indexed_file_count);
            },
            false,
        );
    }

    /// Records why a scan failed.
    pub fn set_scan_error(&self, scan_id: i32, error: String) {
        self.submit(
            move |db: &mut D| {
                db.set_scan_error(scan_id, &error);
            },
            false,
        );
    }

    /// Drops scan history older than a budget.
    pub fn prune_scan_history(&self, max_age_seconds: i64) {
        self.submit(
            move |db: &mut D| {
                db.prune_scan_history(max_age_seconds);
            },
            false,
        );
    }

    /// Opens a scan record, waiting for the worker to answer.
    ///
    /// Fails when the worker thread died instead of answering.
    pub fn create_scan(&self, path: &Path, scan_type: ScanType) -> Result<ScanRecord, String> {
        let (sender, receiver) = mpsc::channel();
        let path = path.to_path_buf();
        self.submit(
            move |db: &mut D| {
                let _ = sender.send(db.create_scan(&path, scan_type));
            },
            false,
        );
        receiver.recv().map_err(|_| "writer shut down".to_owned())?
    }

    /// Adds or refreshes indexed files, applying backpressure.
    pub fn index_files(&self, paths: Vec<PathBuf>) {
        self.submit(
            move |db: &mut D| {
                db.index_files(&paths);
            },
            true,
        );
    }

    /// Forgets indexed files, then runs `on_complete` on the worker thread.
    pub fn delete_indexed_files(
        &self,
        paths: Vec<PathBuf>,
        on_complete: Option<Box<dyn FnOnce() + Send>>,
    ) {
        self.submit(
            move |db: &mut D| {
                db.delete_indexed_files(&paths);
                if let Some(done) = on_complete {
                    done();
                }
            },
            false,
        );
    }

    /// Forgets everything indexed, then runs `on_complete` on the worker thread.
    pub fn delete_all_indexed_files(&self, on_complete: Option<Box<dyn FnOnce() + Send>>) {
        self.submit(
            move |db: &mut D| {
                db.delete_all_indexed_files();
                if let Some(done) = on_complete {
                    done();
                }
            },
            false,
        );
    }

    /// Reclaims space, then runs `on_complete` on the worker thread.
    pub fn compact(&self, on_complete: Option<Box<dyn FnOnce() + Send>>) {
        self.submit(
            move |db: &mut D| {
                db.compact();
                if let Some(done) = on_complete {
                    done();
                }
            },
            false,
        );
    }

    /// Reclaims space, but only when the database says it would help.
    pub fn compact_if_needed(&self) {
        self.submit(
            move |db: &mut D| {
                if db.needs_compaction() {
                    db.compact();
                }
            },
            false,
        );
    }

    /// Applies scanner-found changes and removals, applying backpressure.
    pub fn index_events(&self, events: Vec<FileEvent>) {
        self.submit(
            move |db: &mut D| {
                db.index_events(&events);
            },
            true,
        );
    }

    /// Rebuilds the typo-correction vocabulary, coalescing bursts: a second
    /// call while one is queued or running is dropped, not queued. The flag
    /// clears on the worker thread, after the rebuild runs.
    pub fn rebuild_spellfix_vocabulary(&self) {
        if self.shared.vocab_queued.swap(true, Ordering::SeqCst) {
            return;
        }
        let shared = Arc::clone(&self.shared);
        self.submit(
            move |db: &mut D| {
                db.rebuild_spellfix_vocabulary();
                shared.vocab_queued.store(false, Ordering::SeqCst);
            },
            false,
        );
    }
}

impl<D: IndexDatabase> Drop for DbWriter<D> {
    fn drop(&mut self) {
        self.shared.active.store(false, Ordering::SeqCst);
        self.shared.update.notify_one();
        self.shared.not_full.notify_all();
        if let Some(worker) = self
            .worker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    /// A database that records what ran, in order.
    struct FakeDb {
        log: Arc<Mutex<Vec<String>>>,
        compact_needed: bool,
        rebuilds: Arc<AtomicUsize>,
    }

    impl FakeDb {
        fn record(&self, what: &str) {
            self.log
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(what.to_owned());
        }
    }

    impl IndexDatabase for FakeDb {
        fn is_open(&self) -> bool {
            true
        }

        fn update_scan_status(&mut self, scan_id: i32, status: ScanStatus) -> bool {
            self.record(&format!("status {scan_id} {status:?}"));
            true
        }

        fn finalize_scan(
            &mut self,
            scan_id: i32,
            status: ScanStatus,
            indexed_file_count: i64,
        ) -> bool {
            self.record(&format!(
                "finalize {scan_id} {status:?} {indexed_file_count}"
            ));
            true
        }

        fn set_scan_error(&mut self, scan_id: i32, error: &str) -> bool {
            self.record(&format!("error {scan_id} {error}"));
            true
        }

        fn prune_scan_history(&mut self, max_age_seconds: i64) -> bool {
            self.record(&format!("prune {max_age_seconds}"));
            true
        }

        fn create_scan(&mut self, path: &Path, scan_type: ScanType) -> Result<ScanRecord, String> {
            self.record(&format!("create {} {scan_type:?}", path.display()));
            Ok(ScanRecord {
                id: 7,
                status: ScanStatus::Started,
                created_at: 1,
                finished_at: 0,
                indexed_file_count: 0,
                path: path.to_path_buf(),
                scan_type,
            })
        }

        fn index_files(&mut self, paths: &[PathBuf]) {
            self.record(&format!("index {}", paths.len()));
        }

        fn delete_indexed_files(&mut self, paths: &[PathBuf]) {
            self.record(&format!("delete {}", paths.len()));
        }

        fn delete_all_indexed_files(&mut self) {
            self.record("delete-all");
        }

        fn compact(&mut self) {
            self.record("compact");
        }

        fn needs_compaction(&self) -> bool {
            self.compact_needed
        }

        fn rebuild_spellfix_vocabulary(&mut self) {
            self.rebuilds.fetch_add(1, Ordering::SeqCst);
            self.record("rebuild");
        }

        fn index_events(&mut self, events: &[FileEvent]) {
            self.record(&format!("events {}", events.len()));
        }
    }

    fn writer(compact_needed: bool) -> (DbWriter<FakeDb>, Arc<Mutex<Vec<String>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let rebuilds = Arc::new(AtomicUsize::new(0));
        let maker_log = Arc::clone(&log);
        let maker_rebuilds = Arc::clone(&rebuilds);
        let writer = DbWriter::new(move || FakeDb {
            log: maker_log,
            compact_needed,
            rebuilds: maker_rebuilds,
        });
        (writer, log)
    }

    /// Waits until the log holds `count` entries, so assertions see finished
    /// work rather than racing it.
    fn logged(log: &Arc<Mutex<Vec<String>>>, count: usize) -> Vec<String> {
        let start = std::time::Instant::now();
        loop {
            let entries = log.lock().unwrap_or_else(PoisonError::into_inner).clone();
            if entries.len() >= count {
                return entries;
            }
            if start.elapsed() > Duration::from_secs(5) {
                panic!("timed out waiting for {count} log entries; saw {entries:?}");
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn submissions_run_in_order() {
        let (writer, log) = writer(false);
        writer.update_scan_status(1, ScanStatus::Started);
        writer.finalize_scan(1, ScanStatus::Succeeded, 42);
        writer.set_scan_error(2, "boom".to_owned());
        writer.prune_scan_history(3600);

        assert_eq!(
            logged(&log, 4),
            [
                "status 1 Started",
                "finalize 1 Succeeded 42",
                "error 2 boom",
                "prune 3600"
            ]
        );
    }

    #[test]
    fn bulk_submitters_block_past_the_budget() {
        let (writer, log) = writer(false);
        let (release, gate) = mpsc::channel::<()>();
        // Park the worker: everything behind this waits, including bulk slots.
        writer.submit(
            move |_: &mut FakeDb| {
                let _ = gate.recv();
            },
            false,
        );
        for _ in 0..MAX_PENDING_BULK_WRITES {
            writer.index_files(vec![PathBuf::from("/a")]);
        }
        assert_eq!(MAX_PENDING_BULK_WRITES, 8);

        let parked = thread::spawn({
            let paths = vec![PathBuf::from("/b")];
            move || {
                // Ninth bulk write: blocks until a slot frees up.
                writer.index_files(paths);
            }
        });
        thread::sleep(Duration::from_millis(200));
        assert!(!parked.is_finished(), "the ninth bulk write must wait");

        let _ = release.send(());
        parked.join().expect("the ninth bulk write runs");
        let entries = logged(&log, MAX_PENDING_BULK_WRITES + 1);
        assert!(entries.iter().all(|entry| entry == "index 1"));
    }

    #[test]
    fn create_scan_round_trips_through_the_worker() {
        let (writer, _) = writer(false);
        let record = writer
            .create_scan(Path::new("/home/ada"), ScanType::Incremental)
            .expect("the worker answers");
        assert_eq!(record.id, 7);
        assert_eq!(record.path, PathBuf::from("/home/ada"));
        assert_eq!(record.scan_type, ScanType::Incremental);
    }

    #[test]
    fn scans_carry_their_type() {
        let (writer, _) = writer(false);
        let record = writer
            .create_scan(Path::new("/x"), ScanType::Full)
            .expect("the worker answers");
        assert_eq!(record.scan_type, ScanType::Full);
    }

    #[test]
    fn completions_run_after_their_work() {
        let (writer, log) = writer(false);
        let done = Arc::new(AtomicBool::new(false));
        let setter = Arc::clone(&done);
        writer.delete_all_indexed_files(Some(Box::new(move || {
            setter.store(true, Ordering::SeqCst);
        })));

        logged(&log, 1);
        assert!(done.load(Ordering::SeqCst));
    }

    #[test]
    fn vocabulary_rebuilds_coalesce() {
        let (writer, log) = writer(false);
        let (release, gate) = mpsc::channel::<()>();
        // Park the worker so both rebuilds queue before either runs.
        writer.submit(
            move |_: &mut FakeDb| {
                let _ = gate.recv();
            },
            false,
        );
        writer.rebuild_spellfix_vocabulary();
        writer.rebuild_spellfix_vocabulary();
        let _ = release.send(());

        let entries = logged(&log, 1);
        assert_eq!(entries, ["rebuild"]);
    }

    #[test]
    fn compaction_runs_only_when_needed() {
        let (needed, needed_log) = writer(true);
        needed.compact_if_needed();
        assert_eq!(logged(&needed_log, 1), ["compact"]);

        let (unneeded, unneeded_log) = writer(false);
        unneeded.compact_if_needed();
        thread::sleep(Duration::from_millis(200));
        assert!(
            unneeded_log
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_empty()
        );
    }

    #[test]
    fn shutdown_drains_what_is_queued() {
        let (writer, log) = writer(false);
        writer.update_scan_status(1, ScanStatus::Started);
        writer.update_scan_status(2, ScanStatus::Started);
        drop(writer);

        // Joining drained the queue: both ran before drop returned.
        assert_eq!(log.lock().unwrap_or_else(PoisonError::into_inner).len(), 2);
    }

    #[test]
    fn events_and_deletions_reach_the_database() {
        let (writer, log) = writer(false);
        writer.delete_indexed_files(vec![PathBuf::from("/gone")], None);
        writer.index_events(vec![FileEvent {
            event_type: FileEventType::Modify,
            path: PathBuf::from("/changed"),
            event_time: SystemTime::UNIX_EPOCH,
            is_directory: false,
            size_bytes: Some(12),
        }]);

        assert_eq!(logged(&log, 2), ["delete 1", "events 1"]);
    }
}
