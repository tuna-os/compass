//! The scan queue: debounced scheduling plus a worker pool.
//!
//! Ports `ScanDispatcher`. Scans arrive through [`ScanDispatcher::enqueue`]
//! for immediate work or [`ScanDispatcher::enqueue_debounced`] for
//! coalescing: rapid repeats of one path collapse into a single scan that
//! fires after a quiet period, or a longer cap, whichever comes first.
//! Two workers run whatever is ready; each scan opens its own
//! read database through the reader factory rather than sharing one per
//! worker, because the reader owns a connection that is `Send` but not
//! `Sync`.
//!
//! Dropping the dispatcher stops everything in the C++ destructor's order:
//! pending scans are forgotten, running ones are interrupted, the queue
//! drains to idle, and the threads join.

use std::collections::{HashMap, VecDeque};
use std::sync::{
    Arc, Condvar, Mutex, PoisonError,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::db_writer::{DbWriter, IndexDatabase, ScanType};
use crate::incremental_scanner::IncrementalScanner;
use crate::indexer_scanner::IndexerScanner;
use crate::query_reader::IndexReader;
use crate::scan::{Scan, ScanData, ScanEvent, ScanMode};
use crate::scanner::StatusCallback;

/// Scanner threads.
const WORKER_COUNT: usize = 2;

/// A coalesced scan fires this long after the last repeat.
const DEBOUNCE_QUIET: Duration = Duration::from_secs(5);

/// ...or this long after the first, however chatty the repeats.
const DEBOUNCE_MAX_DELAY: Duration = Duration::from_secs(30);

/// What a finished status report becomes when the scan asked for events.
pub type EventCallback = Arc<dyn Fn(ScanEvent) + Send + Sync>;

/// One concrete scanner behind the dispatcher's uniform handle.
enum Runner<D: IndexDatabase, R: IndexReader> {
    Full(IndexerScanner<D>),
    Incremental(IncrementalScanner<D, R>),
}

impl<D: IndexDatabase, R: IndexReader> Runner<D, R> {
    fn run(&mut self) {
        match self {
            Self::Full(scanner) => scanner.run(),
            Self::Incremental(scanner) => scanner.run(),
        }
    }

    fn stop_handle(&self) -> Arc<dyn Fn() + Send + Sync> {
        match self {
            Self::Full(scanner) => scanner.stop_handle(),
            Self::Incremental(scanner) => scanner.stop_handle(),
        }
    }
}

/// A scan the workers are running, stoppable through a flag rather than the
/// scanner itself: the worker runs the scanner while the queue only holds
/// how to stop it, so interrupting never waits on a running scan.
struct Running {
    scan: Scan,
    stop: Arc<dyn Fn() + Send + Sync>,
    started: Instant,
}

/// A debounced scan waiting out its quiet period.
struct Pending {
    scan: Scan,
    deadline: Instant,
    first_seen: Instant,
}

struct WorkState {
    ready: VecDeque<(i32, Scan)>,
    running: HashMap<i32, Running>,
    next_id: i32,
}

struct PendingState {
    pending: Vec<Pending>,
}

struct Shared<D: IndexDatabase, R: IndexReader> {
    writer: Arc<DbWriter<D>>,
    make_reader: Arc<dyn Fn() -> R + Send + Sync>,
    on_event: Mutex<Option<EventCallback>>,
    work: Mutex<WorkState>,
    work_cv: Condvar,
    idle_cv: Condvar,
    pending: Mutex<PendingState>,
    pending_cv: Condvar,
    alive: AtomicBool,
    /// How long repeats coalesce. Read with pending work queued; tests
    /// shrink it to watch debounce fire.
    debounce: Mutex<Debounce>,
}

/// How long repeats of one path coalesce before firing.
#[derive(Debug, Clone, Copy)]
struct Debounce {
    /// Past the last repeat: `ScanDispatcher::DEBOUNCE_QUIET`.
    quiet: Duration,
    /// Past the first, however chatty: `ScanDispatcher::DEBOUNCE_MAX_DELAY`.
    max_delay: Duration,
}

/// The file-indexer scan queue.
///
/// Shared across threads behind [`Arc`], so the join handles sit behind
/// mutexes: a plain [`JoinHandle`] is not [`Sync`], and the indexer hands
/// the queue to rebuild threads that outlive any borrow.
pub struct ScanDispatcher<D: IndexDatabase, R: IndexReader> {
    shared: Arc<Shared<D, R>>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    scheduler: Mutex<Option<JoinHandle<()>>>,
}

impl<D: IndexDatabase, R: IndexReader> ScanDispatcher<D, R> {
    /// Starts the scheduler and the worker pool. `make_reader` opens the
    /// read database each scan gets; the writer is shared by every scan.
    pub fn new(
        writer: Arc<DbWriter<D>>,
        make_reader: impl Fn() -> R + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(Shared {
            writer,
            make_reader: Arc::new(make_reader),
            on_event: Mutex::new(None),
            work: Mutex::new(WorkState {
                ready: VecDeque::new(),
                running: HashMap::new(),
                next_id: 0,
            }),
            work_cv: Condvar::new(),
            idle_cv: Condvar::new(),
            pending: Mutex::new(PendingState {
                pending: Vec::new(),
            }),
            pending_cv: Condvar::new(),
            alive: AtomicBool::new(true),
            debounce: Mutex::new(Debounce {
                quiet: DEBOUNCE_QUIET,
                max_delay: DEBOUNCE_MAX_DELAY,
            }),
        });
        let mut workers = Vec::with_capacity(WORKER_COUNT);
        for _ in 0..WORKER_COUNT {
            let worker_shared = Arc::clone(&shared);
            workers.push(std::thread::spawn(move || worker_loop(worker_shared)));
        }
        let scheduler_shared = Arc::clone(&shared);
        let scheduler = std::thread::spawn(move || scheduler_loop(scheduler_shared));
        Self {
            shared,
            workers: Mutex::new(workers),
            scheduler: Mutex::new(Some(scheduler)),
        }
    }

    /// Delivers scan events to `callback`. Stored, never called under a
    /// lock, the way the C++ calls its member without holding either mutex.
    pub fn set_event_callback(&self, callback: impl Fn(ScanEvent) + Send + Sync + 'static) {
        *self
            .shared
            .on_event
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(callback));
    }

    /// Queues `scan` for the next free worker, or `-1` when its path is
    /// already running or queued.
    pub fn enqueue(&self, scan: Scan) -> i32 {
        enqueue(&self.shared, scan)
    }

    /// Coalesces `scan` with repeats of its path instead of queueing it.
    pub fn enqueue_debounced(&self, scan: Scan) {
        let debounce = *self
            .shared
            .debounce
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        {
            let mut pending = self
                .shared
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            merge_pending(
                &mut pending.pending,
                scan,
                now,
                debounce.quiet,
                debounce.max_delay,
            );
        }
        self.shared.pending_cv.notify_one();
    }

    /// Stops the scan with `id`: flags it running, or drops it queued.
    /// Reports whether either happened.
    pub fn interrupt(&self, id: i32) -> bool {
        let mut work = self
            .shared
            .work
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(running) = work.running.get(&id) {
            (running.stop)();
            return true;
        }
        if let Some(queued) = work.ready.iter().position(|(ready_id, _)| *ready_id == id) {
            work.ready.remove(queued);
            self.shared.idle_cv.notify_all();
            return true;
        }
        false
    }

    /// Forgets everything queued and flags everything running.
    pub fn interrupt_all(&self) {
        let mut work = self
            .shared
            .work
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        work.ready.clear();
        for running in work.running.values() {
            (running.stop)();
        }
        self.shared.idle_cv.notify_all();
    }

    /// Forgets everything waiting out its quiet period.
    pub fn clear_pending(&self) {
        self.shared
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pending
            .clear();
    }

    /// Blocks until nothing is running or queued.
    pub fn wait_until_idle(&self) {
        let mut work = self
            .shared
            .work
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while !(work.running.is_empty() && work.ready.is_empty()) {
            work = self
                .shared
                .idle_cv
                .wait(work)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// The running scans, then the queued ones.
    pub fn scans(&self) -> Vec<(i32, Scan)> {
        let work = self
            .shared
            .work
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut scans = Vec::with_capacity(work.running.len() + work.ready.len());
        for (id, running) in &work.running {
            scans.push((*id, running.scan.clone()));
        }
        for (id, scan) in &work.ready {
            scans.push((*id, scan.clone()));
        }
        scans
    }
}

impl<D: IndexDatabase, R: IndexReader> Drop for ScanDispatcher<D, R> {
    fn drop(&mut self) {
        self.clear_pending();
        self.interrupt_all();
        self.wait_until_idle();
        self.shared.alive.store(false, Ordering::SeqCst);
        self.shared.pending_cv.notify_all();
        self.shared.work_cv.notify_all();
        if let Some(scheduler) = self
            .scheduler
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            let _ = scheduler.join();
        }
        for worker in self
            .workers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
        {
            let _ = worker.join();
        }
    }
}

/// The shared enqueue: duplicate paths across running and queued scans read
/// `-1`, everything else takes the next id and wakes a worker.
fn enqueue<D: IndexDatabase, R: IndexReader>(shared: &Arc<Shared<D, R>>, scan: Scan) -> i32 {
    let mut work = shared.work.lock().unwrap_or_else(PoisonError::into_inner);
    let duplicate = work
        .running
        .values()
        .any(|running| running.scan.path == scan.path)
        || work.ready.iter().any(|(_, ready)| ready.path == scan.path);
    if duplicate {
        scan_log(&scan, "skipping, already running");
        return -1;
    }
    let scan_id = work.next_id;
    work.next_id += 1;
    scan_log(&scan, &format!("enqueuing scan {scan_id}"));
    work.ready.push_back((scan_id, scan));
    shared.work_cv.notify_one();
    scan_id
}

/// Folds `scan` into the pending set: new paths wait a quiet period,
/// repeats rearm up to the cap past their first sighting.
fn merge_pending(
    pending: &mut Vec<Pending>,
    scan: Scan,
    now: Instant,
    quiet: Duration,
    max_delay: Duration,
) {
    if let Some(repeat) = pending
        .iter_mut()
        .find(|repeat| repeat.scan.path == scan.path && repeat.scan.scan_type() == scan.scan_type())
    {
        repeat.deadline = std::cmp::min(now + quiet, repeat.first_seen + max_delay);
        return;
    }
    pending.push(Pending {
        scan,
        deadline: now + quiet,
        first_seen: now,
    });
}

/// Full scans log loud, incremental ones quiet: the C++ picks info or debug
/// by scan shape.
fn scan_log(scan: &Scan, message: &str) {
    let what = match scan.scan_type() {
        ScanType::Full => "full",
        ScanType::Incremental => "incremental",
    };
    let how = match &scan.data {
        ScanData::Incremental(incremental) => match incremental.mode {
            ScanMode::Exhaustive => "exhaustive",
            ScanMode::Pruned => "pruned",
        },
        ScanData::Full(_) => "exhaustive",
    };
    if scan.scan_type() == ScanType::Full {
        tracing::info!(path = ?scan.path, what, how, "{message}");
    } else {
        tracing::debug!(path = ?scan.path, what, how, "{message}");
    }
}

/// One concrete scanner for `scan`, reporting through the event callback
/// when the scan asked for events. The callback slot is read per event, so
/// a callback set after enqueue still delivers.
fn runner_for<D: IndexDatabase, R: IndexReader>(
    shared: &Arc<Shared<D, R>>,
    scan_id: i32,
    scan: Scan,
) -> Runner<D, R> {
    let slot = Arc::clone(shared);
    let notify = scan.notify;
    let path = scan.path.clone();
    let scan_type = scan.scan_type();
    let callback: StatusCallback = Box::new(move |status, processed| {
        if !notify {
            return;
        }
        let on_event = slot
            .on_event
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let Some(on_event) = on_event else {
            return;
        };
        on_event(ScanEvent {
            scan_id,
            scan_type,
            status,
            entrypoint: path.clone(),
            processed_file_count: processed,
        });
    });
    match scan.data {
        ScanData::Full(_) => Runner::Full(IndexerScanner::new(
            Arc::clone(&shared.writer),
            scan,
            callback,
        )),
        ScanData::Incremental(_) => {
            let reader = (shared.make_reader)();
            Runner::Incremental(IncrementalScanner::new(
                Arc::clone(&shared.writer),
                scan,
                reader,
                callback,
            ))
        }
    }
}

/// One worker: take the next ready scan, run it unlocked, record the run.
fn worker_loop<D: IndexDatabase, R: IndexReader>(shared: Arc<Shared<D, R>>) {
    loop {
        let (scan_id, mut runner) = {
            let mut work = shared.work.lock().unwrap_or_else(PoisonError::into_inner);
            while work.ready.is_empty() && shared.alive.load(Ordering::SeqCst) {
                work = shared
                    .work_cv
                    .wait(work)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            if !shared.alive.load(Ordering::SeqCst) {
                break;
            }
            let (scan_id, scan) = work.ready.pop_front().expect("signalled non-empty");
            let record = scan.clone();
            let runner = runner_for(&shared, scan_id, scan);
            let stop = runner.stop_handle();
            let started = Instant::now();
            work.running.insert(
                scan_id,
                Running {
                    scan: record,
                    stop,
                    started,
                },
            );
            (scan_id, runner)
        };
        runner.run();
        let mut work = shared.work.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(running) = work.running.remove(&scan_id) {
            let elapsed = running.started.elapsed();
            scan_log(&running.scan, &format!("done in {}ms", elapsed.as_millis()));
        }
        shared.idle_cv.notify_all();
    }
}

/// The scheduler: pending scans whose quiet period passed go to the queue;
/// ones the queue refuses — already running — wait out another period.
fn scheduler_loop<D: IndexDatabase, R: IndexReader>(shared: Arc<Shared<D, R>>) {
    let mut pending = shared
        .pending
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    loop {
        if !shared.alive.load(Ordering::SeqCst) {
            break;
        }
        if pending.pending.is_empty() {
            pending = shared
                .pending_cv
                .wait_while(pending, |pending| {
                    pending.pending.is_empty() && shared.alive.load(Ordering::SeqCst)
                })
                .unwrap_or_else(PoisonError::into_inner);
            continue;
        }
        let next = pending
            .pending
            .iter()
            .map(|repeat| repeat.deadline)
            .min()
            .expect("pending non-empty");
        let debounce = *shared
            .debounce
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let timeout = next.saturating_duration_since(Instant::now());
        let (guard, waited) = shared
            .pending_cv
            .wait_timeout_while(pending, timeout, |_| shared.alive.load(Ordering::SeqCst))
            .unwrap_or_else(PoisonError::into_inner);
        pending = guard;
        if !waited.timed_out() {
            break;
        }
        if !shared.alive.load(Ordering::SeqCst) {
            break;
        }
        let now = Instant::now();
        let mut due = Vec::new();
        pending.pending.retain(|repeat| {
            if repeat.deadline > now {
                return true;
            }
            due.push(repeat.scan.clone());
            false
        });
        drop(pending);
        let mut deferred = Vec::new();
        for scan in due {
            if !shared.alive.load(Ordering::SeqCst) {
                break;
            }
            if enqueue(&shared, scan.clone()) < 0 {
                deferred.push(scan);
            }
        }
        pending = shared
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        for scan in deferred {
            merge_pending(
                &mut pending.pending,
                scan,
                now,
                debounce.quiet,
                debounce.max_delay,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    use crate::db_writer::{FileEvent, IndexDatabase, ScanRecord, ScanStatus, ScanType};
    use crate::query_engine::{SearchCandidate, SearchOptions};
    use crate::query_policy::VocabularySuggestion;
    use crate::scan::{FullScan, IncrementalScan, ScanMode};

    /// A database recording indexed paths and deletes, gating scan records
    /// behind a latch so tests can hold scans running or queued on purpose.
    struct GatedDb {
        events: Arc<Mutex<Vec<PathBuf>>>,
        deleted: Arc<Mutex<Vec<PathBuf>>>,
        gate: Arc<(Mutex<bool>, Condvar)>,
    }

    impl IndexDatabase for GatedDb {
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
            let (open, changed) = &*self.gate;
            let mut open = open.lock().unwrap_or_else(PoisonError::into_inner);
            while !*open {
                open = changed.wait(open).unwrap_or_else(PoisonError::into_inner);
            }
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

        fn delete_indexed_files(&mut self, paths: &[PathBuf]) {
            self.deleted
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(paths.iter().cloned());
        }

        fn delete_all_indexed_files(&mut self) {}

        fn compact(&mut self) {}

        fn needs_compaction(&self) -> bool {
            false
        }

        fn rebuild_vocabulary(&mut self) {}

        fn index_events(&mut self, events: &[FileEvent]) {
            self.events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(events.iter().map(|event| event.path.clone()));
        }
    }

    /// The index as dispatched scans read it.
    #[derive(Clone, Default)]
    struct FakeReader {
        children: HashMap<PathBuf, HashSet<PathBuf>>,
        tracked: HashSet<PathBuf>,
    }

    impl IndexReader for FakeReader {
        fn is_open(&self) -> bool {
            true
        }

        fn search_candidates(
            &self,
            _query: &str,
            _limit: usize,
            _options: &SearchOptions,
        ) -> Vec<SearchCandidate> {
            Vec::new()
        }

        fn search_skeleton_candidates(
            &self,
            _query: &str,
            _limit: usize,
            _options: &SearchOptions,
        ) -> Vec<SearchCandidate> {
            Vec::new()
        }

        fn vocabulary_suggestions(
            &self,
            _word: &str,
            _top: i32,
            _prefix: bool,
        ) -> Vec<VocabularySuggestion> {
            Vec::new()
        }

        fn list_indexed_directory_files(&self, path: &Path) -> HashSet<PathBuf> {
            self.children.get(path).cloned().unwrap_or_default()
        }

        fn tracks_file(&self, path: &Path) -> bool {
            self.tracked.contains(path)
        }

        fn last_successful_scan(&self, _path: &Path) -> Option<ScanRecord> {
            None
        }

        fn last_scan(&self, _path: &Path, _scan_type: ScanType) -> Option<ScanRecord> {
            None
        }

        fn has_vocabulary(&self) -> bool {
            true
        }

        fn recent_directories(&self, _limit: usize) -> Vec<PathBuf> {
            Vec::new()
        }
    }

    struct Harness {
        dispatcher: ScanDispatcher<GatedDb, FakeReader>,
        events: Arc<Mutex<Vec<PathBuf>>>,
        reported: Arc<Mutex<Vec<ScanEvent>>>,
        gate: Arc<(Mutex<bool>, Condvar)>,
    }

    impl Harness {
        fn new(reader: FakeReader, gate_open: bool) -> Self {
            let events = Arc::new(Mutex::new(Vec::new()));
            let deleted = Arc::new(Mutex::new(Vec::new()));
            let gate = Arc::new((Mutex::new(gate_open), Condvar::new()));
            let maker_events = Arc::clone(&events);
            let maker_deleted = Arc::clone(&deleted);
            let maker_gate = Arc::clone(&gate);
            let writer = Arc::new(DbWriter::new(move || GatedDb {
                events: maker_events,
                deleted: maker_deleted,
                gate: maker_gate,
            }));
            let reported = Arc::new(Mutex::new(Vec::new()));
            let maker_reported = Arc::clone(&reported);
            let dispatcher = ScanDispatcher::new(Arc::clone(&writer), move || reader.clone());
            dispatcher.set_event_callback(move |event| {
                maker_reported
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(event);
            });
            // The dispatcher holds the writer's other half; this one is
            // only built to hand over.
            drop(writer);
            Self {
                dispatcher,
                events,
                reported,
                gate,
            }
        }

        fn release(&self) {
            let (open, changed) = &*self.gate;
            *open.lock().unwrap_or_else(PoisonError::into_inner) = true;
            changed.notify_all();
        }

        fn full_scan(root: &Path) -> Scan {
            Scan {
                path: root.to_path_buf(),
                data: ScanData::Full(FullScan {
                    excluded_paths: Vec::new(),
                }),
                notify: true,
            }
        }

        fn reported(&self) -> Vec<ScanEvent> {
            self.reported
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    }

    /// Waits until `locked` holds `count` entries, so assertions see finished
    /// work rather than racing it.
    fn wait_for<T: Clone>(locked: &Mutex<Vec<T>>, count: usize) -> Vec<T> {
        let start = Instant::now();
        loop {
            let entries = locked
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            if entries.len() >= count {
                return entries;
            }
            if start.elapsed() > Duration::from_secs(10) {
                panic!("timed out waiting for {count} entries");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn fixture() -> tempfile::TempDir {
        tempfile::tempdir().expect("temporary tree")
    }

    #[test]
    fn enqueue_runs_a_full_scan_and_reports_its_events() {
        let dir = fixture();
        std::fs::write(dir.path().join("a.txt"), "notes").expect("seed file");

        let harness = Harness::new(FakeReader::default(), true);
        assert_eq!(
            harness.dispatcher.enqueue(Harness::full_scan(dir.path())),
            0
        );
        harness.dispatcher.wait_until_idle();

        let mut indexed = wait_for(&harness.events, 2);
        indexed.sort();
        let mut expected = vec![dir.path().to_path_buf(), dir.path().join("a.txt")];
        expected.sort();
        assert_eq!(indexed, expected);

        let reported = wait_for(&harness.reported, 2);
        assert_eq!(reported[0].scan_id, 0);
        assert_eq!(reported[0].status, ScanStatus::Started);
        assert_eq!(reported[0].entrypoint, dir.path());
        assert_eq!(reported[1].status, ScanStatus::Succeeded);
        assert_eq!(reported[1].processed_file_count, 1);
    }

    #[test]
    fn enqueue_runs_an_incremental_scan_through_the_reader() {
        let dir = fixture();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).expect("seed dir");
        std::fs::write(sub.join("nested.txt"), "deep").expect("seed file");

        let harness = Harness::new(FakeReader::default(), true);
        let scan = Scan {
            path: dir.path().to_path_buf(),
            data: ScanData::Incremental(IncrementalScan {
                mode: ScanMode::Exhaustive,
                max_depth: None,
                excluded_filenames: Vec::new(),
                excluded_paths: Vec::new(),
            }),
            notify: true,
        };
        assert_eq!(harness.dispatcher.enqueue(scan), 0);
        harness.dispatcher.wait_until_idle();

        // Nothing indexed, so the new subdirectory is descended into: root
        // and sub from the root diff, sub and nested from the sub diff.
        let mut indexed = wait_for(&harness.events, 4);
        indexed.sort();
        let mut expected = vec![
            dir.path().to_path_buf(),
            sub.clone(),
            sub.clone(),
            sub.join("nested.txt"),
        ];
        expected.sort();
        assert_eq!(indexed, expected);
        assert_eq!(
            harness.reported().last().map(|event| event.status),
            Some(ScanStatus::Succeeded)
        );
    }

    #[test]
    fn a_path_already_running_or_queued_reads_minus_one() {
        let dir = fixture();

        // The gate holds the first scan running, so the repeat is a
        // duplicate whether the worker has picked it up or not.
        let harness = Harness::new(FakeReader::default(), false);
        assert_eq!(
            harness.dispatcher.enqueue(Harness::full_scan(dir.path())),
            0
        );
        assert_eq!(
            harness.dispatcher.enqueue(Harness::full_scan(dir.path())),
            -1
        );

        harness.release();
        harness.dispatcher.wait_until_idle();

        let succeeded = harness
            .reported()
            .into_iter()
            .filter(|event| event.status == ScanStatus::Succeeded)
            .count();
        assert_eq!(succeeded, 1);
    }

    #[test]
    fn interrupt_drops_a_queued_scan() {
        let first = fixture();
        let second = fixture();
        let queued = fixture();

        // Both workers hold gated scans, so the third scan stays queued.
        let harness = Harness::new(FakeReader::default(), false);
        assert_eq!(
            harness.dispatcher.enqueue(Harness::full_scan(first.path())),
            0
        );
        assert_eq!(
            harness
                .dispatcher
                .enqueue(Harness::full_scan(second.path())),
            1
        );
        assert_eq!(
            harness
                .dispatcher
                .enqueue(Harness::full_scan(queued.path())),
            2
        );
        assert!(harness.dispatcher.interrupt(2));

        let running: HashSet<i32> = harness
            .dispatcher
            .scans()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(running, HashSet::from([0, 1]));

        harness.release();
        harness.dispatcher.wait_until_idle();

        assert!(
            harness
                .reported()
                .iter()
                .all(|event| event.entrypoint != queued.path())
        );
    }

    #[test]
    fn interrupt_reports_false_for_an_unknown_scan() {
        let harness = Harness::new(FakeReader::default(), true);
        assert!(!harness.dispatcher.interrupt(7));
    }

    #[test]
    fn debounced_repeats_coalesce_into_one_scan() {
        let dir = fixture();
        std::fs::write(dir.path().join("a.txt"), "notes").expect("seed file");

        let harness = Harness::new(FakeReader::default(), true);
        *harness
            .dispatcher
            .shared
            .debounce
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Debounce {
            quiet: Duration::from_millis(50),
            max_delay: Duration::from_millis(200),
        };
        for _ in 0..3 {
            harness
                .dispatcher
                .enqueue_debounced(Harness::full_scan(dir.path()));
        }

        // The scheduler fires one scan for all three repeats.
        let reported = wait_for(&harness.reported, 2);
        harness.dispatcher.wait_until_idle();
        assert_eq!(
            reported
                .iter()
                .filter(|event| event.status == ScanStatus::Succeeded)
                .count(),
            1
        );
    }
}
