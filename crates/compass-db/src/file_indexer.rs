//! The file indexer: configuration, startup scans, and the watch.
//!
//! Ports `FileIndexer`. It owns the writer, the dispatcher, and the query
//! engine; on top of those it layers what turns configuration into scans —
//! entrypoints with exclusions, the pending full-scan roots a restart
//! resumes from, the reconcile plan a settings change executes, and the
//! throttled typo-vocabulary rebuild. The live directory watch itself
//! arrives behind [`FileIndexer::set_watcher_controls`]: the platform
//! backend builds it, the indexer only decides when it runs — stopped
//! across full scans, back once they drain.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use compass_core::scan_roots::{
    compact_subtrees, is_covered_by_any, is_same_or_descendant_of, normalize_path, normalize_paths,
};

use crate::db_writer::{DbWriter, IndexDatabase, ScanRecord, ScanStatus, ScanType};
use crate::query_engine::{IndexerFileResult, SearchOptions};
use crate::query_reader::{FileIndexerQueryEngine, IndexReader};
use crate::scan::{FullScan, IncrementalScan, Scan, ScanData, ScanEvent, ScanMode};
use crate::scan_dispatcher::{EventCallback, ScanDispatcher};

/// Scan history older than this is pruned at startup:
/// `FileIndexer::SCAN_HISTORY_MAX_AGE`.
const SCAN_HISTORY_MAX_AGE_SECS: i64 = 7 * 24 * 60 * 60;

/// Typo-vocabulary rebuilds at most this often:
/// `FileIndexer::VOCAB_REBUILD_MIN_INTERVAL`.
const VOCAB_REBUILD_MIN_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Builds the platform watch from a configuration snapshot. The backend
/// wires its change reports back into the dispatcher's debounced scans; the
/// indexer only calls this once full scans have drained.
pub type WatcherStarter = Arc<dyn Fn(Vec<PathBuf>, Vec<PathBuf>, Vec<String>) + Send + Sync>;

/// Tears the platform watch down across full scans and reconfigures.
pub type WatcherStopper = Arc<dyn Fn() + Send + Sync>;

/// What a settings change executes: subtrees to forget, roots to scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcilePlan {
    /// Indexed subtrees no configuration covers anymore.
    pub delete_subtrees: Vec<PathBuf>,
    /// Roots no surviving scan covers anymore.
    pub scan_roots: Vec<PathBuf>,
}

/// The configured roots and what they refuse.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct IndexerConfig {
    entrypoints: Vec<PathBuf>,
    excluded_paths: Vec<PathBuf>,
    excluded_filenames: Vec<String>,
}

struct Inner<D: IndexDatabase, R: IndexReader> {
    writer: Arc<DbWriter<D>>,
    dispatcher: Arc<ScanDispatcher<D, R>>,
    /// Behind a mutex because the reader is `Send` but not `Sync`: queries
    /// from any thread serialize on the one connection, the way the C++
    /// pool gives each worker its own engine and never shares one.
    query_engine: Mutex<FileIndexerQueryEngine<R>>,
    make_reader: Arc<dyn Fn() -> R + Send + Sync>,
    database_file_name: String,
    config: Mutex<IndexerConfig>,
    pending_full_scan_roots: Mutex<Vec<PathBuf>>,
    last_vocab_rebuild: Mutex<Option<Instant>>,
    watcher_starter: Mutex<Option<WatcherStarter>>,
    watcher_stopper: Mutex<Option<WatcherStopper>>,
    on_scan_event: Mutex<Option<EventCallback>>,
}

/// The file indexer: configuration in, scans out.
pub struct FileIndexer<D: IndexDatabase, R: IndexReader> {
    inner: Arc<Inner<D, R>>,
}

impl<D: IndexDatabase, R: IndexReader> FileIndexer<D, R> {
    /// Builds the indexer around `writer`, opening read databases through
    /// `make_reader` — one for the query engine, one per dispatched scan.
    /// `database_file_name` is the index file's name (`file-indexer.db` in
    /// production): its rows must never index themselves, so it and its
    /// write-ahead log join every scan's excluded basenames.
    pub fn new(
        writer: Arc<DbWriter<D>>,
        make_reader: impl Fn() -> R + Send + Sync + 'static,
        database_file_name: impl Into<String>,
    ) -> Self {
        let make_reader = Arc::new(make_reader);
        let query_engine = FileIndexerQueryEngine::new(make_reader());
        let scan_reader = Arc::clone(&make_reader);
        let dispatcher = Arc::new(ScanDispatcher::new(Arc::clone(&writer), move || {
            scan_reader()
        }));
        let inner = Arc::new(Inner {
            writer: Arc::clone(&writer),
            dispatcher,
            query_engine: Mutex::new(query_engine),
            make_reader,
            database_file_name: database_file_name.into(),
            config: Mutex::new(IndexerConfig::default()),
            pending_full_scan_roots: Mutex::new(Vec::new()),
            last_vocab_rebuild: Mutex::new(None),
            watcher_starter: Mutex::new(None),
            watcher_stopper: Mutex::new(None),
            on_scan_event: Mutex::new(None),
        });
        inner.writer.prune_scan_history(SCAN_HISTORY_MAX_AGE_SECS);
        inner.writer.compact_if_needed();
        // A strong callback here would cycle inner through the dispatcher
        // and leak both; the weak one drops the events once the indexer is
        // gone instead.
        let event_inner: Weak<Inner<D, R>> = Arc::downgrade(&inner);
        inner
            .dispatcher
            .set_event_callback(move |event: ScanEvent| {
                // Bound before matching: a temporary in the `let-else`
                // scrutinee would drop with the statement.
                let upgraded = event_inner.upgrade();
                let Some(event_inner) = upgraded else {
                    return;
                };
                if event.scan_type == ScanType::Full && event.status == ScanStatus::Succeeded {
                    event_inner.mark_full_scan_succeeded(&event.entrypoint);
                }
                if event.status == ScanStatus::Succeeded && event_inner.should_rebuild_vocabulary()
                {
                    event_inner.writer.rebuild_spellfix_vocabulary();
                }
                if event.status == ScanStatus::Succeeded
                    && event.scan_type == ScanType::Full
                    && !event_inner.has_pending_full_scan_roots()
                {
                    event_inner.start_file_system_watcher();
                }
                let callback = event_inner
                    .on_scan_event
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone();
                if let Some(callback) = callback {
                    callback(event);
                }
            });
        Self { inner }
    }

    /// Forwards scan events to `callback`, after the indexer's own
    /// bookkeeping ran.
    pub fn set_scan_event_callback(&self, callback: impl Fn(ScanEvent) + Send + Sync + 'static) {
        *self
            .inner
            .on_scan_event
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(callback));
    }

    /// Hands the platform watch lifecycle to the indexer: built with a
    /// configuration snapshot once full scans drain, torn down across them.
    pub fn set_watcher_controls(&self, starter: WatcherStarter, stopper: WatcherStopper) {
        *self
            .inner
            .watcher_starter
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(starter);
        *self
            .inner
            .watcher_stopper
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(stopper);
    }

    /// The scan queue, for the watch backend to report through.
    pub fn dispatcher(&self) -> Arc<ScanDispatcher<D, R>> {
        Arc::clone(&self.inner.dispatcher)
    }

    /// Replaces the configuration wholesale, deriving the index's own
    /// excluded basenames like the C++ `setConfig`.
    pub fn set_config(&self, paths: Vec<PathBuf>, excluded_paths: Vec<PathBuf>) {
        let mut config = self
            .inner
            .config
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        config.entrypoints = normalize_paths(paths, absolute);
        config.excluded_paths = normalize_paths(excluded_paths, absolute);
        config.excluded_filenames = self.inner.excluded_database_basenames();
    }

    /// Applies a configuration change without rescanning the world:
    /// uncovered subtrees are forgotten, uncovered roots are fully scanned,
    /// and the watch restarts around the work.
    pub fn apply_config(&self, paths: Vec<PathBuf>, excluded_paths: Vec<PathBuf>) {
        let new_entrypoints = normalize_paths(paths, absolute);
        let new_excluded = normalize_paths(excluded_paths, absolute);
        let (old_entrypoints, old_excluded) = {
            let config = self
                .inner
                .config
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if new_entrypoints == config.entrypoints && new_excluded == config.excluded_paths {
                return;
            }
            (config.entrypoints.clone(), config.excluded_paths.clone())
        };
        let mut plan = build_reconcile_plan(
            &old_entrypoints,
            &old_excluded,
            &new_entrypoints,
            &new_excluded,
        );
        self.inner
            .prune_pending_full_scans(&new_entrypoints, &new_excluded);
        let mut pending = self
            .inner
            .pending_full_scan_roots_for(&new_entrypoints, &new_excluded);
        plan.scan_roots.append(&mut pending);
        plan.scan_roots = compact_subtrees(std::mem::take(&mut plan.scan_roots));
        {
            let mut config = self
                .inner
                .config
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            config.entrypoints = new_entrypoints;
            config.excluded_paths = new_excluded;
            config.excluded_filenames = self.inner.excluded_database_basenames();
        }

        tracing::info!(
            deletes = plan.delete_subtrees.len(),
            scans = plan.scan_roots.len(),
            "applying file indexer configuration change"
        );
        self.inner.stop_file_system_watcher();
        self.dispatcher().clear_pending();
        self.dispatcher().interrupt_all();
        self.dispatcher().wait_until_idle();

        let has_deletes = !plan.delete_subtrees.is_empty();
        let has_scans = !plan.scan_roots.is_empty();
        if has_deletes {
            let inner = Arc::clone(&self.inner);
            let restart_watcher = !has_scans;
            self.inner.writer.delete_indexed_files(
                std::mem::take(&mut plan.delete_subtrees),
                restart_watcher.then(|| {
                    Box::new(move || inner.start_file_system_watcher()) as Box<dyn FnOnce() + Send>
                }),
            );
        }
        for root in std::mem::take(&mut plan.scan_roots) {
            self.inner
                .mark_full_scan_roots_pending(std::slice::from_ref(&root));
            self.dispatcher().enqueue(Scan {
                path: root,
                data: ScanData::Full(FullScan {
                    excluded_paths: self.excluded_paths(),
                }),
                notify: true,
            });
        }
        if !has_deletes && !has_scans {
            self.inner.start_file_system_watcher();
        }
    }

    /// Clears the index and fully scans every entrypoint on a worker thread,
    /// the way `startFullScan` detaches its rebuild.
    pub fn start_full_scan(&self) {
        self.inner.stop_file_system_watcher();
        self.dispatcher().clear_pending();
        self.dispatcher().interrupt_all();
        self.dispatcher().wait_until_idle();
        self.inner.mark_full_scan_roots_pending(&self.entrypoints());

        // The completion outlives any borrow: it carries the dispatcher
        // handle and a configuration snapshot, never the indexer itself.
        let dispatcher = Arc::clone(&self.inner.dispatcher);
        let entrypoints = self.entrypoints();
        let excluded_paths = self.excluded_paths();
        let inner = Arc::clone(&self.inner);
        std::thread::spawn(move || {
            tracing::info!("starting full scan, clearing existing index");
            inner
                .writer
                .delete_all_indexed_files(Some(Box::new(move || {
                    tracing::info!("existing index cleared, enqueuing full scan tasks");
                    for entrypoint in entrypoints {
                        tracing::info!(path = ?entrypoint, "enqueuing full scan");
                        dispatcher.enqueue(Scan {
                            path: entrypoint,
                            data: ScanData::Full(FullScan {
                                excluded_paths: excluded_paths.clone(),
                            }),
                            notify: true,
                        });
                    }
                }) as Box<dyn FnOnce() + Send>));
        });
    }

    /// Scans one entrypoint, interrupting its same-shape scan first.
    /// Incremental scans default to exhaustive, like the C++ struct default.
    pub fn start_single_scan(
        &self,
        entrypoint: &Path,
        scan_type: ScanType,
        excluded_filenames: Vec<String>,
    ) {
        for (id, scan) in self.dispatcher().scans() {
            if scan.scan_type() == scan_type && scan.path == entrypoint {
                self.dispatcher().interrupt(id);
            }
        }
        if scan_type == ScanType::Full {
            self.inner
                .mark_full_scan_roots_pending(&[entrypoint.to_path_buf()]);
            self.dispatcher().enqueue(Scan {
                path: entrypoint.to_path_buf(),
                data: ScanData::Full(FullScan {
                    excluded_paths: self.excluded_paths(),
                }),
                notify: true,
            });
            return;
        }
        let mut excluded = self.excluded_filenames();
        excluded.extend(excluded_filenames);
        self.dispatcher().enqueue(Scan {
            path: entrypoint.to_path_buf(),
            data: ScanData::Incremental(IncrementalScan {
                mode: ScanMode::Exhaustive,
                max_depth: None,
                excluded_filenames: excluded,
                excluded_paths: self.excluded_paths(),
            }),
            notify: true,
        });
    }

    /// Clears the index and fully scans everything.
    pub fn rebuild_index(&self) {
        self.start_full_scan();
    }

    /// Records a scan that never finished as interrupted, so the next start
    /// does not mistake it for a crash without a witness.
    pub fn mark_scan_as_interrupted(&self, scan: Option<ScanRecord>) {
        let Some(scan) = scan else {
            return;
        };
        tracing::warn!(path = ?scan.path, "previous scan was unsuccessful, marking interrupted");
        self.inner
            .writer
            .set_scan_error(scan.id, "Interrupted".to_owned());
    }

    /// Starts from the entrypoints: first startups scan fully, interrupted
    /// full scans are marked and restarted, healthy ones go incremental —
    /// and the watch starts when nothing needs a full scan.
    pub fn start(&self) {
        if !(self.inner.make_reader)().is_open() {
            tracing::error!("file indexer database is not open, refusing to start scans");
            return;
        }
        // Typo corrections before the first scan of the session completes.
        if !(self.inner.make_reader)().has_spellfix_vocabulary() {
            self.inner.writer.rebuild_spellfix_vocabulary();
        }

        let mut needs_full_scan = false;
        for entrypoint in self.entrypoints() {
            let reader = (self.inner.make_reader)();
            match reader.last_scan(&entrypoint, ScanType::Full) {
                None => {
                    tracing::info!(path = ?entrypoint, "first startup, starting full scan");
                    self.start_single_scan(&entrypoint, ScanType::Full, Vec::new());
                    needs_full_scan = true;
                }
                Some(last) if last.status != ScanStatus::Succeeded => {
                    tracing::info!(path = ?entrypoint, "last full scan did not succeed, restarting");
                    self.mark_scan_as_interrupted(Some(last));
                    self.start_single_scan(&entrypoint, ScanType::Full, Vec::new());
                    needs_full_scan = true;
                }
                _ => {
                    let last = reader.last_scan(&entrypoint, ScanType::Incremental);
                    if let Some(last) = last.filter(|last| last.status != ScanStatus::Succeeded) {
                        self.mark_scan_as_interrupted(Some(last));
                    }
                    tracing::debug!(path = ?entrypoint, "starting incremental scan");
                    self.start_single_scan(&entrypoint, ScanType::Incremental, Vec::new());
                }
            }
        }
        if !needs_full_scan {
            self.inner.start_file_system_watcher();
        }
    }

    /// Searches the index, or nothing when the engine has no vocabulary.
    pub fn query(
        &self,
        view: &str,
        limit: usize,
        options: &SearchOptions,
    ) -> Vec<IndexerFileResult> {
        let engine = self
            .inner
            .query_engine
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !engine.is_available() {
            return Vec::new();
        }
        engine.query(view, limit, options)
    }

    fn entrypoints(&self) -> Vec<PathBuf> {
        self.inner.entrypoints()
    }

    fn excluded_paths(&self) -> Vec<PathBuf> {
        self.inner.excluded_paths()
    }

    fn excluded_filenames(&self) -> Vec<String> {
        self.inner
            .config
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .excluded_filenames
            .clone()
    }
}

impl<D: IndexDatabase, R: IndexReader> Inner<D, R> {
    fn entrypoints(&self) -> Vec<PathBuf> {
        self.config
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entrypoints
            .clone()
    }

    fn excluded_paths(&self) -> Vec<PathBuf> {
        self.config
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .excluded_paths
            .clone()
    }

    fn excluded_database_basenames(&self) -> Vec<String> {
        vec![
            self.database_file_name.clone(),
            format!("{}-wal", self.database_file_name),
        ]
    }

    fn start_file_system_watcher(&self) {
        let config = self.config.lock().unwrap_or_else(PoisonError::into_inner);
        let starter = self
            .watcher_starter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(starter) = starter {
            starter(
                config.entrypoints.clone(),
                config.excluded_paths.clone(),
                config.excluded_filenames.clone(),
            );
        }
    }

    fn stop_file_system_watcher(&self) {
        if let Some(stopper) = self
            .watcher_stopper
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            stopper();
        }
    }

    fn mark_full_scan_roots_pending(&self, roots: &[PathBuf]) {
        let mut pending = self
            .pending_full_scan_roots
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let room = pending.len() + roots.len();
        pending.reserve(room);
        pending.extend(roots.iter().map(|root| normalize_path(root)));
        let compacted = compact_subtrees(std::mem::take(&mut *pending));
        *pending = compacted;
    }

    fn mark_full_scan_succeeded(&self, root: &Path) {
        let normalized = normalize_path(root);
        self.pending_full_scan_roots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|pending| !is_same_or_descendant_of(pending, &normalized));
    }

    fn has_pending_full_scan_roots(&self) -> bool {
        !self
            .pending_full_scan_roots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    }

    fn prune_pending_full_scans(&self, roots: &[PathBuf], exclusions: &[PathBuf]) {
        self.pending_full_scan_roots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|pending| {
                is_covered_by_any(pending, roots) && !is_covered_by_any(pending, exclusions)
            });
    }

    fn pending_full_scan_roots_for(
        &self,
        roots: &[PathBuf],
        exclusions: &[PathBuf],
    ) -> Vec<PathBuf> {
        let pending = self
            .pending_full_scan_roots
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let matching: Vec<PathBuf> = pending
            .iter()
            .filter(|pending| {
                is_covered_by_any(pending, roots) && !is_covered_by_any(pending, exclusions)
            })
            .cloned()
            .collect();
        compact_subtrees(matching)
    }

    fn should_rebuild_vocabulary(&self) -> bool {
        let mut last = self
            .last_vocab_rebuild
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        if last.is_some_and(|rebuild| now - rebuild < VOCAB_REBUILD_MIN_INTERVAL) {
            return false;
        }
        *last = Some(now);
        true
    }
}

/// What a settings change executes, from the old and new configurations.
fn build_reconcile_plan(
    old_roots: &[PathBuf],
    old_exclusions: &[PathBuf],
    new_roots: &[PathBuf],
    new_exclusions: &[PathBuf],
) -> ReconcilePlan {
    let mut plan = ReconcilePlan {
        delete_subtrees: Vec::new(),
        scan_roots: Vec::new(),
    };
    for old in old_roots {
        if !is_covered_by_any(old, new_roots) {
            plan.delete_subtrees.push(old.clone());
        }
    }
    for new in new_roots {
        if !covered_by_remaining_old_root(new, old_roots, new_roots) {
            plan.scan_roots.push(new.clone());
        }
    }
    for exclusion in new_exclusions {
        if !is_covered_by_any(exclusion, old_exclusions) {
            plan.delete_subtrees.push(exclusion.clone());
        }
    }
    for exclusion in old_exclusions {
        if is_covered_by_any(exclusion, new_exclusions) {
            continue;
        }
        if is_covered_by_any(exclusion, new_roots) {
            plan.scan_roots.push(exclusion.clone());
        }
    }
    plan.scan_roots
        .retain(|root| !is_covered_by_any(root, new_exclusions));
    plan.delete_subtrees = compact_subtrees(std::mem::take(&mut plan.delete_subtrees));
    plan.scan_roots = compact_subtrees(std::mem::take(&mut plan.scan_roots));
    plan
}

/// Whether an old root still standing covers `new_root`.
fn covered_by_remaining_old_root(
    new_root: &Path,
    old_roots: &[PathBuf],
    new_roots: &[PathBuf],
) -> bool {
    old_roots
        .iter()
        .any(|old| is_same_or_descendant_of(new_root, old) && is_covered_by_any(old, new_roots))
}

/// An absolute path, falling back to the path itself when the working
/// directory is gone: the `resolve` half of `normalize_paths`.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::sync::Condvar;
    use std::time::Duration;

    use crate::db_writer::{FileEvent, IndexDatabase};
    use crate::query_engine::{SearchCandidate, SearchOptions};
    use crate::query_policy::SpellfixSuggestion;

    /// A database recording events, deletes, errors and rebuilds, gating
    /// scan records behind a latch so tests can hold scans back on purpose.
    struct RecordingDb {
        events: Arc<Mutex<Vec<PathBuf>>>,
        deleted: Arc<Mutex<Vec<PathBuf>>>,
        errors: Arc<Mutex<Vec<String>>>,
        rebuilds: Arc<Mutex<usize>>,
        gate: Arc<(Mutex<bool>, Condvar)>,
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

        fn set_scan_error(&mut self, scan_id: i32, error: &str) -> bool {
            self.errors
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(format!("error {scan_id} {error}"));
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

        fn rebuild_spellfix_vocabulary(&mut self) {
            *self.rebuilds.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        }

        fn index_events(&mut self, events: &[FileEvent]) {
            self.events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(events.iter().map(|event| event.path.clone()));
        }
    }

    /// The index as the orchestrator reads it.
    #[derive(Clone)]
    struct FakeReader {
        children: HashMap<PathBuf, HashSet<PathBuf>>,
        tracked: HashSet<PathBuf>,
        scans: Vec<ScanRecord>,
        open: bool,
        has_spellfix: bool,
    }

    impl FakeReader {
        fn healthy() -> Self {
            Self {
                children: HashMap::new(),
                tracked: HashSet::new(),
                scans: Vec::new(),
                open: true,
                has_spellfix: true,
            }
        }

        fn with_scan(mut self, record: ScanRecord) -> Self {
            self.scans.push(record);
            self
        }
    }

    impl IndexReader for FakeReader {
        fn is_open(&self) -> bool {
            self.open
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

        fn spellfix_suggestions(
            &self,
            _word: &str,
            _top: i32,
            _prefix: bool,
        ) -> Vec<SpellfixSuggestion> {
            Vec::new()
        }

        fn list_indexed_directory_files(&self, path: &Path) -> HashSet<PathBuf> {
            self.children.get(path).cloned().unwrap_or_default()
        }

        fn tracks_file(&self, path: &Path) -> bool {
            self.tracked.contains(path)
        }

        fn last_successful_scan(&self, path: &Path) -> Option<ScanRecord> {
            self.last_scan(path, ScanType::Full)
                .filter(|scan| scan.status == ScanStatus::Succeeded)
        }

        fn last_scan(&self, path: &Path, scan_type: ScanType) -> Option<ScanRecord> {
            self.scans
                .iter()
                .filter(|scan| scan.path == path && scan.scan_type == scan_type)
                .max_by_key(|scan| scan.created_at)
                .cloned()
        }

        fn has_spellfix_vocabulary(&self) -> bool {
            self.has_spellfix
        }
    }

    /// One recorded watch start: the configuration snapshot it was built
    /// from.
    type WatchStart = (Vec<PathBuf>, Vec<PathBuf>, Vec<String>);

    /// A watcher backend recording its starts and stops.
    #[derive(Default)]
    struct FakeWatcher {
        starts: Arc<Mutex<Vec<WatchStart>>>,
        stops: Arc<Mutex<usize>>,
    }

    struct Harness {
        indexer: FileIndexer<RecordingDb, FakeReader>,
        events: Arc<Mutex<Vec<PathBuf>>>,
        deleted: Arc<Mutex<Vec<PathBuf>>>,
        errors: Arc<Mutex<Vec<String>>>,
        rebuilds: Arc<Mutex<usize>>,
        gate: Arc<(Mutex<bool>, Condvar)>,
        watcher: FakeWatcher,
    }

    impl Harness {
        fn new(reader: FakeReader) -> Self {
            Self::gated(reader, true)
        }

        fn gated(reader: FakeReader, gate_open: bool) -> Self {
            let events = Arc::new(Mutex::new(Vec::new()));
            let deleted = Arc::new(Mutex::new(Vec::new()));
            let errors = Arc::new(Mutex::new(Vec::new()));
            let rebuilds = Arc::new(Mutex::new(0));
            let gate = Arc::new((Mutex::new(gate_open), Condvar::new()));
            let maker_events = Arc::clone(&events);
            let maker_deleted = Arc::clone(&deleted);
            let maker_errors = Arc::clone(&errors);
            let maker_rebuilds = Arc::clone(&rebuilds);
            let maker_gate = Arc::clone(&gate);
            let writer = Arc::new(DbWriter::new(move || RecordingDb {
                events: maker_events,
                deleted: maker_deleted,
                errors: maker_errors,
                rebuilds: maker_rebuilds,
                gate: maker_gate,
            }));
            let indexer = FileIndexer::new(writer, move || reader.clone(), "test.db");
            let watcher = FakeWatcher::default();
            let starter_starts = Arc::clone(&watcher.starts);
            let starter_stops = Arc::clone(&watcher.stops);
            indexer.set_watcher_controls(
                Arc::new(move |entrypoints, excluded_paths, excluded_filenames| {
                    starter_starts
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push((entrypoints, excluded_paths, excluded_filenames));
                }),
                Arc::new(move || {
                    *starter_stops.lock().unwrap_or_else(PoisonError::into_inner) += 1;
                }),
            );
            Self {
                indexer,
                events,
                deleted,
                errors,
                rebuilds,
                gate,
                watcher,
            }
        }

        fn release(&self) {
            let (open, changed) = &*self.gate;
            *open.lock().unwrap_or_else(PoisonError::into_inner) = true;
            changed.notify_all();
        }

        fn scan_record(path: &Path, scan_type: ScanType, status: ScanStatus) -> ScanRecord {
            ScanRecord {
                id: 7,
                status,
                created_at: 0,
                finished_at: 0,
                indexed_file_count: 0,
                path: path.to_path_buf(),
                scan_type,
            }
        }

        fn started_watches(&self) -> Vec<WatchStart> {
            self.watcher
                .starts
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

    fn wait_for_rebuilds(rebuilds: &Mutex<usize>, count: usize) {
        let start = Instant::now();
        loop {
            if *rebuilds.lock().unwrap_or_else(PoisonError::into_inner) >= count {
                return;
            }
            if start.elapsed() > Duration::from_secs(10) {
                panic!("timed out waiting for {count} rebuilds");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn fixture() -> tempfile::TempDir {
        tempfile::tempdir().expect("temporary tree")
    }

    #[test]
    fn reconcile_drops_roots_no_configuration_covers() {
        let old = PathBuf::from("/home/ada");
        let plan = build_reconcile_plan(
            std::slice::from_ref(&old),
            &[],
            &[PathBuf::from("/srv")],
            &[],
        );
        assert_eq!(plan.delete_subtrees, [old]);
        assert_eq!(plan.scan_roots, [PathBuf::from("/srv")]);
    }

    #[test]
    fn reconcile_keeps_roots_a_surviving_scan_covers() {
        let kept = PathBuf::from("/home/ada");
        let plan = build_reconcile_plan(
            std::slice::from_ref(&kept),
            &[],
            std::slice::from_ref(&kept),
            &[],
        );
        assert!(plan.delete_subtrees.is_empty());
        assert!(plan.scan_roots.is_empty());
    }

    #[test]
    fn reconcile_forgets_new_exclusions_and_rescans_lifted_ones() {
        let root = PathBuf::from("/home/ada");
        let dropped = build_reconcile_plan(
            std::slice::from_ref(&root),
            &[],
            std::slice::from_ref(&root),
            &[root.join("tmp")],
        );
        assert_eq!(dropped.delete_subtrees, [root.join("tmp")]);
        assert!(dropped.scan_roots.is_empty());

        let lifted = build_reconcile_plan(
            std::slice::from_ref(&root),
            &[root.join("tmp")],
            std::slice::from_ref(&root),
            &[],
        );
        assert!(lifted.delete_subtrees.is_empty());
        assert_eq!(lifted.scan_roots, [root.join("tmp")]);
    }

    #[test]
    fn reconcile_compacts_overlapping_roots() {
        let plan = build_reconcile_plan(
            &[],
            &[],
            &[PathBuf::from("/home/ada"), PathBuf::from("/home/ada/code")],
            &[],
        );
        assert_eq!(plan.scan_roots, [PathBuf::from("/home/ada")]);
    }

    #[test]
    fn first_startup_scans_fully_without_starting_the_watch() {
        let dir = fixture();
        std::fs::write(dir.path().join("a.txt"), "notes").expect("seed file");

        // The gate holds the scan short of a record, so the full scan is
        // still pending and the watch cannot have started yet.
        let harness = Harness::gated(FakeReader::healthy(), false);
        harness
            .indexer
            .set_config(vec![dir.path().to_path_buf()], vec![]);
        harness.indexer.start();
        // The record never opens behind the gate, so no success event can
        // have restarted the watch — running or queued, the scan is stuck.
        assert!(harness.started_watches().is_empty());

        harness.release();
        let mut indexed = wait_for(&harness.events, 2);
        indexed.sort();
        assert_eq!(
            indexed,
            vec![dir.path().to_path_buf(), dir.path().join("a.txt")].sort_clone()
        );
    }

    #[test]
    fn a_failed_full_scan_is_marked_and_restarted() {
        let dir = fixture();

        let reader = FakeReader::healthy().with_scan(Harness::scan_record(
            dir.path(),
            ScanType::Full,
            ScanStatus::Failed,
        ));
        let harness = Harness::new(reader);
        harness
            .indexer
            .set_config(vec![dir.path().to_path_buf()], vec![]);
        harness.indexer.start();

        assert_eq!(wait_for(&harness.errors, 1), ["error 7 Interrupted"]);
        wait_for(&harness.events, 1);
    }

    #[test]
    fn a_healthy_entrypoint_scans_incrementally_and_starts_the_watch() {
        let dir = fixture();
        std::fs::write(dir.path().join("a.txt"), "notes").expect("seed file");

        let reader = [ScanType::Full, ScanType::Incremental].into_iter().fold(
            FakeReader::healthy(),
            |reader, scan_type| {
                reader.with_scan(Harness::scan_record(
                    dir.path(),
                    scan_type,
                    ScanStatus::Succeeded,
                ))
            },
        );
        let harness = Harness::new(reader);
        harness
            .indexer
            .set_config(vec![dir.path().to_path_buf()], vec![]);
        harness.indexer.start();

        // Nothing indexed, so the incremental diff reports root and file.
        let mut indexed = wait_for(&harness.events, 2);
        indexed.sort();
        assert_eq!(
            indexed,
            vec![dir.path().to_path_buf(), dir.path().join("a.txt")].sort_clone()
        );
        // No full scan pending anywhere, so the watch starts with the
        // normalized roots and the index's own basenames excluded.
        let watches = wait_for(&harness.watcher.starts, 1);
        assert_eq!(watches[0].0, [dir.path().to_path_buf()]);
        assert!(watches[0].1.is_empty());
        assert_eq!(watches[0].2, ["test.db", "test.db-wal"]);
    }

    #[test]
    fn a_closed_database_starts_nothing() {
        let dir = fixture();
        let reader = FakeReader {
            open: false,
            ..FakeReader::healthy()
        };
        let harness = Harness::new(reader);
        harness
            .indexer
            .set_config(vec![dir.path().to_path_buf()], vec![]);
        harness.indexer.start();

        assert!(harness.indexer.dispatcher().scans().is_empty());
        assert!(harness.started_watches().is_empty());
    }

    #[test]
    fn a_missing_vocabulary_rebuilds_before_scanning() {
        let dir = fixture();
        let reader = FakeReader {
            has_spellfix: false,
            ..FakeReader::healthy()
        };
        let harness = Harness::new(reader);
        harness
            .indexer
            .set_config(vec![dir.path().to_path_buf()], vec![]);
        harness.indexer.start();

        wait_for_rebuilds(&harness.rebuilds, 1);
    }

    #[test]
    fn an_unchanged_configuration_is_a_no_op() {
        let dir = fixture();
        let harness = Harness::new(FakeReader::healthy());
        harness
            .indexer
            .set_config(vec![dir.path().to_path_buf()], vec![]);
        harness
            .indexer
            .apply_config(vec![dir.path().to_path_buf()], vec![]);

        assert!(harness.indexer.dispatcher().scans().is_empty());
        assert!(
            harness
                .deleted
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_empty()
        );
    }

    #[test]
    fn a_changed_configuration_forgets_and_rescans() {
        let old = fixture();
        let new = fixture();
        std::fs::write(new.path().join("b.txt"), "notes").expect("seed file");

        let harness = Harness::new(FakeReader::healthy());
        harness
            .indexer
            .set_config(vec![old.path().to_path_buf()], vec![]);
        harness
            .indexer
            .apply_config(vec![new.path().to_path_buf()], vec![]);

        assert_eq!(wait_for(&harness.deleted, 1), [old.path().to_path_buf()]);
        wait_for(&harness.events, 2);
        // The rescan's success clears its pending root and restarts the
        // watch exactly once, after the reconfigure stopped it.
        wait_for(&harness.watcher.starts, 1);
        assert_eq!(
            *harness
                .watcher
                .stops
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
            1
        );
    }

    #[test]
    fn pending_roots_compact_and_clear_on_success() {
        let harness = Harness::new(FakeReader::healthy());
        let inner = Arc::clone(&harness.indexer.inner);
        let root = PathBuf::from("/home/ada");
        inner.mark_full_scan_roots_pending(&[root.join("code"), root.clone()]);
        assert_eq!(
            inner
                .pending_full_scan_roots
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            [root]
        );
        assert!(inner.has_pending_full_scan_roots());
        inner.mark_full_scan_succeeded(&root);
        assert!(!inner.has_pending_full_scan_roots());
    }

    #[test]
    fn vocabulary_rebuilds_throttle_to_ten_minutes() {
        let harness = Harness::new(FakeReader::healthy());
        let inner = Arc::clone(&harness.indexer.inner);
        assert!(inner.should_rebuild_vocabulary());
        assert!(!inner.should_rebuild_vocabulary());
    }

    #[test]
    fn queries_answer_nothing_without_a_database() {
        let reader = FakeReader {
            open: false,
            ..FakeReader::healthy()
        };
        let harness = Harness::new(reader);
        assert!(
            harness
                .indexer
                .query("report", 10, &SearchOptions::default())
                .is_empty()
        );
    }

    trait SortClone {
        fn sort_clone(self) -> Self;
    }

    impl SortClone for Vec<PathBuf> {
        fn sort_clone(mut self) -> Self {
            self.sort();
            self
        }
    }
}
