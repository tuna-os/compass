//! The live file-index watch: directory events in, debounced scans out.
//!
//! Ports `FileSystemWatcher`. The routing half already lives in
//! [`compass_core::watch_events`] — which roots earn scans, the background
//! sweep, the dynamic-watch refresh — so this is the live half: a
//! [`DirWatcher`] drained on a pump thread, its events submitted to the
//! [`ScanDispatcher`], and a minute timer refreshing dynamic watches and
//! sweeping the entrypoints.
//!
//! # Deltas from the C++, on purpose
//!
//! * The C++ runs an event-callback thread plus a timer thread; here one pump
//!   thread drains and sweeps, because [`DirWatcher::drain`] polls instead of
//!   blocking on inotify. Shutdown latency is one pump interval, not one
//!   minute.
//! * The C++ `m_allowsBackgroundUpdates` flag is written nowhere — always
//!   true — so there is no equivalent; the sweep always runs.
//! * Like [`FileIndexer`](compass_db::file_indexer::FileIndexer), the pump
//!   thread holds the watcher weakly: dropping the last [`Arc`] stops the
//!   thread instead of leaking it.
//!
//! Only the `compass` binary selects this module. A platform backend a shared
//! crate can reach is not a backend — see `compass-platform-linux`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use compass_core::file_walk::IndexWalk;
use compass_core::incremental_scan::Mode as CoreMode;
use compass_core::watch_events::{
    DYNAMIC_WATCH_COUNT, ScanRequest, WatchConfig, background_sweep_requests, dynamic_watch_dirs,
    requests_for_event,
};
use compass_db::db_writer::IndexDatabase;
use compass_db::file_indexer::{WatcherStarter, WatcherStopper};
use compass_db::query_reader::IndexReader;
use compass_db::scan::{IncrementalScan, Scan, ScanData, ScanMode};
use compass_db::scan_dispatcher::ScanDispatcher;
use compass_platform_linux::dir_watcher::DirWatcher;

/// How often the pump thread drains the watcher: four wakeups a second is
/// invisible next to the dispatcher's five-second debounce quiet period, and
/// bounds shutdown latency the same way.
const PUMP_INTERVAL: Duration = Duration::from_millis(250);

/// Reads the most recently changed directories, newest first.
///
/// Backed by [`IndexReader::recent_directories`]: the backend hands the live
/// reader factory in, so dynamic watches follow the database, not a snapshot.
pub type RecentDirs = Arc<dyn Fn(usize) -> Vec<PathBuf> + Send + Sync>;

/// Builds and tears down the live watch for one [`FileIndexer`](compass_db::file_indexer::FileIndexer).
///
/// Holds everything the watch needs except the configuration snapshot, which
/// arrives with [`controls`](Self::controls): the watcher only ever runs on
/// the snapshot the indexer hands over once full scans drain.
pub struct IndexWatchBackend<D: IndexDatabase, R: IndexReader> {
    dispatcher: Arc<ScanDispatcher<D, R>>,
    home: Option<PathBuf>,
    xdg_dirs: Vec<PathBuf>,
    walk: IndexWalk,
    recent: RecentDirs,
    sweep_interval: Duration,
    live: Mutex<Option<Arc<IndexWatcher<D, R>>>>,
}

impl<D: IndexDatabase, R: IndexReader> std::fmt::Debug for IndexWatchBackend<D, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexWatchBackend")
            .field("home", &self.home)
            .field("sweep_interval", &self.sweep_interval)
            .finish_non_exhaustive()
    }
}

impl<D: IndexDatabase, R: IndexReader> IndexWatchBackend<D, R> {
    /// Stages the backend: nothing watches until the starter runs.
    ///
    /// `sweep_interval` is
    /// [`BACKGROUND_UPDATE_INTERVAL_SECS`](compass_core::watch_events::BACKGROUND_UPDATE_INTERVAL_SECS)
    /// in production; tests pass shorter ones.
    #[must_use]
    pub fn new(
        dispatcher: Arc<ScanDispatcher<D, R>>,
        home: Option<PathBuf>,
        xdg_dirs: Vec<PathBuf>,
        walk: IndexWalk,
        recent: RecentDirs,
        sweep_interval: Duration,
    ) -> Self {
        Self {
            dispatcher,
            home,
            xdg_dirs,
            walk,
            recent,
            sweep_interval,
            live: Mutex::new(None),
        }
    }

    /// The starter/stopper pair for
    /// [`FileIndexer::set_watcher_controls`](compass_db::file_indexer::FileIndexer::set_watcher_controls).
    ///
    /// The starter builds the watch from the snapshot and replaces whatever
    /// ran before; a failed build logs and leaves the previous watch — or no
    /// watch — in place. The stopper drops the running watch, if any.
    #[must_use]
    pub fn controls(self: &Arc<Self>) -> (WatcherStarter, WatcherStopper) {
        let starter_backend = Arc::clone(self);
        let starter: WatcherStarter =
            Arc::new(move |entrypoints, excluded_paths, excluded_filenames| {
                starter_backend.restart(entrypoints, excluded_paths, excluded_filenames);
            });
        let stopper_backend = Arc::clone(self);
        let stopper: WatcherStopper = Arc::new(move || stopper_backend.stop());
        (starter, stopper)
    }

    /// Whether a watch is currently running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    fn restart(
        &self,
        entrypoints: Vec<PathBuf>,
        excluded_paths: Vec<PathBuf>,
        excluded_filenames: Vec<String>,
    ) {
        let home_entries = self.home.as_deref().map_or_else(Vec::new, list_entries);
        match IndexWatcher::start(
            Arc::clone(&self.dispatcher),
            WatchConfig {
                entrypoints,
                excluded_paths,
                excluded_filenames,
            },
            self.home.as_deref(),
            &home_entries,
            &self.xdg_dirs,
            self.walk.clone(),
            Arc::clone(&self.recent),
            self.sweep_interval,
        ) {
            Ok(watcher) => {
                *self.live.lock().unwrap_or_else(PoisonError::into_inner) = Some(watcher);
            }
            Err(error) => {
                tracing::error!(error = %error, "starting the file-index watch");
            }
        }
    }

    fn stop(&self) {
        *self.live.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// The running watch: a [`DirWatcher`] drained on a pump thread.
///
/// Built by [`IndexWatchBackend`], never directly. Dropping the last [`Arc`]
/// stops the pump thread.
pub struct IndexWatcher<D: IndexDatabase, R: IndexReader> {
    dispatcher: Arc<ScanDispatcher<D, R>>,
    config: WatchConfig,
    watcher: Mutex<DirWatcher>,
    recent: RecentDirs,
    sweep_interval: Duration,
    alive: AtomicBool,
    pump: Mutex<Option<JoinHandle<()>>>,
}

impl<D: IndexDatabase, R: IndexReader> std::fmt::Debug for IndexWatcher<D, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexWatcher")
            .field("config", &self.config)
            .field("sweep_interval", &self.sweep_interval)
            .finish_non_exhaustive()
    }
}

impl<D: IndexDatabase, R: IndexReader> IndexWatcher<D, R> {
    /// Starts watching: refreshes the dynamic watches, then pumps.
    #[allow(clippy::too_many_arguments)]
    fn start(
        dispatcher: Arc<ScanDispatcher<D, R>>,
        config: WatchConfig,
        home: Option<&Path>,
        home_entries: &[PathBuf],
        xdg_dirs: &[PathBuf],
        walk: IndexWalk,
        recent: RecentDirs,
        sweep_interval: Duration,
    ) -> notify::Result<Arc<Self>> {
        let watcher = DirWatcher::new(
            home,
            home_entries,
            |path| path.is_dir(),
            |path| path.is_symlink(),
            xdg_dirs,
            walk,
        )?;
        let watcher = Arc::new(Self {
            dispatcher,
            config,
            watcher: Mutex::new(watcher),
            recent,
            sweep_interval,
            alive: AtomicBool::new(true),
            pump: Mutex::new(None),
        });
        watcher.refresh_dynamic();
        let pump_weak: Weak<Self> = Arc::downgrade(&watcher);
        let mut last_sweep = Instant::now();
        let pump = std::thread::spawn(move || {
            while let Some(watcher) = pump_weak.upgrade() {
                std::thread::sleep(PUMP_INTERVAL);
                if !watcher.alive.load(Ordering::SeqCst) {
                    return;
                }
                watcher.drain_once();
                if last_sweep.elapsed() >= watcher.sweep_interval {
                    last_sweep = Instant::now();
                    watcher.sweep_once();
                }
            }
        });
        *watcher.pump.lock().unwrap_or_else(PoisonError::into_inner) = Some(pump);
        Ok(watcher)
    }

    /// Every directory currently watched.
    #[must_use]
    pub fn watched_directories(&self) -> Vec<PathBuf> {
        self.watcher
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .watched_directories()
    }

    fn drain_once(&self) {
        let events = self
            .watcher
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain();
        for event in &events {
            for request in requests_for_event(&self.config, event) {
                self.submit(request);
            }
        }
    }

    fn sweep_once(&self) {
        for request in background_sweep_requests(&self.config, |path| path.is_dir()) {
            self.submit(request);
        }
        self.refresh_dynamic();
    }

    fn refresh_dynamic(&self) {
        let recent = (self.recent)(DYNAMIC_WATCH_COUNT);
        let dirs = dynamic_watch_dirs(&self.config, recent);
        self.watcher
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set_dynamic_directories(&dirs);
    }

    fn submit(&self, request: ScanRequest) {
        let scan = Scan {
            path: request.path,
            data: ScanData::Incremental(IncrementalScan {
                mode: match request.mode {
                    CoreMode::Exhaustive => ScanMode::Exhaustive,
                    CoreMode::Pruned => ScanMode::Pruned,
                },
                max_depth: request.max_depth,
                excluded_filenames: request.excluded_filenames,
                excluded_paths: request.excluded_paths,
            }),
            notify: true,
        };
        if request.debounced {
            self.dispatcher.enqueue_debounced(scan);
        } else {
            self.dispatcher.enqueue(scan);
        }
    }
}

impl<D: IndexDatabase, R: IndexReader> Drop for IndexWatcher<D, R> {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        if let Some(pump) = self
            .pump
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            let _ = pump.join();
        }
    }
}

/// The home directory's direct contents, or nothing when it cannot be read.
///
/// Read fresh at every watch build: entries created since the last build join
/// the watch set, the way a restarted C++ watcher picks them up.
fn list_entries(home: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(home).map_or_else(
        |error| {
            tracing::warn!(home = ?home, error = %error, "listing the home directory");
            Vec::new()
        },
        |entries| {
            entries
                .filter_map(|entry| entry.map(|entry| entry.path()).ok())
                .collect()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    use compass_db::db_writer::{DbWriter, ScanRecord, ScanStatus, ScanType};
    use compass_db::query_engine::{SearchCandidate, SearchOptions};
    use compass_db::query_policy::VocabularySuggestion;
    use compass_db::scan::ScanEvent;

    struct FakeDb;

    impl IndexDatabase for FakeDb {
        fn is_open(&self) -> bool {
            true
        }

        fn update_scan_status(&mut self, _scan_id: i32, _status: ScanStatus) -> bool {
            true
        }

        fn finalize_scan(
            &mut self,
            _scan_id: i32,
            _status: ScanStatus,
            _indexed_file_count: i64,
        ) -> bool {
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
                id: 1,
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

        fn rebuild_vocabulary(&mut self) {}

        fn index_events(&mut self, _events: &[compass_db::db_writer::FileEvent]) {}
    }

    #[derive(Clone)]
    struct FakeReader {
        recent: Vec<PathBuf>,
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

        fn list_indexed_directory_files(&self, _path: &Path) -> HashSet<PathBuf> {
            HashSet::new()
        }

        fn tracks_file(&self, _path: &Path) -> bool {
            false
        }

        fn last_successful_scan(&self, _path: &Path) -> Option<ScanRecord> {
            None
        }

        fn last_scan(&self, _path: &Path, _scan_type: ScanType) -> Option<ScanRecord> {
            None
        }

        fn has_vocabulary(&self) -> bool {
            false
        }

        fn recent_directories(&self, limit: usize) -> Vec<PathBuf> {
            self.recent.iter().take(limit).cloned().collect()
        }
    }

    struct Fixture {
        events: Arc<Mutex<Vec<ScanEvent>>>,
        backend: Arc<IndexWatchBackend<FakeDb, FakeReader>>,
    }

    impl Fixture {
        fn new(home: &Path, recent: Vec<PathBuf>, sweep_interval: Duration) -> Self {
            let writer = Arc::new(DbWriter::new(|| FakeDb));
            let dispatcher = Arc::new(ScanDispatcher::new(writer, || FakeReader {
                recent: Vec::new(),
            }));
            let events = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::clone(&events);
            dispatcher.set_event_callback(move |event| {
                sink.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(event);
            });
            let backend = Arc::new(IndexWatchBackend::new(
                Arc::clone(&dispatcher),
                Some(home.to_path_buf()),
                Vec::new(),
                IndexWalk::new(Some(home)),
                Arc::new(move |limit| recent.iter().take(limit).cloned().collect()),
                sweep_interval,
            ));
            Self { events, backend }
        }

        fn start(&self, entrypoints: Vec<PathBuf>) -> WatcherStopper {
            let (starter, stopper) = self.backend.controls();
            starter(entrypoints, Vec::new(), Vec::new());
            stopper
        }

        fn wait_for(
            &self,
            mut matches: impl FnMut(&ScanEvent) -> bool,
            what: &str,
            timeout: Duration,
        ) {
            let start = Instant::now();
            loop {
                if self
                    .events
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .iter()
                    .any(&mut matches)
                {
                    return;
                }
                if start.elapsed() > timeout {
                    panic!("timed out waiting for {what}");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    #[test]
    fn background_sweep_scans_live_entrypoints() {
        let home = tempfile::tempdir().expect("tempdir");
        let fixture = Fixture::new(home.path(), Vec::new(), Duration::from_millis(50));
        let _stopper = fixture.start(vec![home.path().to_path_buf()]);

        // The sweep enqueues immediately, not through the debounce, so this
        // arrives on the first interval — no quiet period to wait out.
        fixture.wait_for(
            |event| event.scan_type == ScanType::Incremental && event.entrypoint == home.path(),
            "sweep scan",
            Duration::from_secs(10),
        );
    }

    #[test]
    fn directory_change_enqueues_a_debounced_scan() {
        let home = tempfile::tempdir().expect("tempdir");
        // An hour between sweeps: only the change below may scan.
        let fixture = Fixture::new(home.path(), Vec::new(), Duration::from_secs(3600));
        let _stopper = fixture.start(vec![home.path().to_path_buf()]);

        std::fs::write(home.path().join("note.txt"), "notes").expect("fixture file");

        // Through the debounce quiet period, so slower than the sweep.
        fixture.wait_for(
            |event| event.scan_type == ScanType::Incremental && event.entrypoint == home.path(),
            "change scan",
            Duration::from_secs(20),
        );
    }

    #[test]
    fn starter_builds_and_stopper_tears_down_the_watch() {
        let home = tempfile::tempdir().expect("tempdir");
        let watched = home.path().join("watched");
        std::fs::create_dir(&watched).expect("fixture dir");
        let fixture = Fixture::new(
            home.path(),
            vec![watched.clone()],
            Duration::from_secs(3600),
        );

        assert!(!fixture.backend.is_running());
        let stopper = fixture.start(vec![home.path().to_path_buf()]);
        assert!(fixture.backend.is_running());

        // The initial refresh is synchronous in the starter: recent
        // directories are watched before it returns.
        let live = fixture
            .backend
            .live
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let watched_dirs = live.as_ref().expect("running watch").watched_directories();
        assert!(
            watched_dirs.contains(&watched),
            "recent directory watched, saw {watched_dirs:?}"
        );
        drop(live);

        stopper();
        assert!(!fixture.backend.is_running());
    }

    #[test]
    fn entries_outside_the_entrypoints_earn_no_scan() {
        let home = tempfile::tempdir().expect("tempdir");
        let elsewhere = tempfile::tempdir().expect("tempdir");
        let fixture = Fixture::new(home.path(), Vec::new(), Duration::from_millis(50));
        let _stopper = fixture.start(vec![home.path().to_path_buf()]);

        // The sweep only covers the entrypoints: a live elsewhere directory
        // must never produce a scan for itself.
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            !fixture
                .events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .any(|event| event.entrypoint == elsewhere.path()),
            "no scan outside the entrypoints"
        );
    }
}
