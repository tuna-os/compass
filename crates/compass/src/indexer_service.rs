//! The file-indexer service: JSON-RPC over stdio, like `compass-file-indexer`.
//!
//! Ports `IndexerService` and the `main.cpp` frame loop. The wire protocol is
//! the figura `file-indexer.fig` contract, which figura only emits as glaze
//! C++ — so the message shapes live here as serde types, kept field-for-field
//! with the IDL: `Service/method` routing, string enums, `snake_case`
//! fields, `u32` length-prefixed frames over stdin/stdout.
//!
//! # Deltas from the C++, on purpose
//!
//! * Queries run on the [`QueryPool`] the C++ service owns, not on the
//!   indexer's own engine: same threads, same async replies.
//! * A malformed frame is ignored exactly like the C++ `route` returning
//!   early, and unparseable params run with defaults the way glaze leaves a
//!   failed `read_json` payload default-constructed.
//! * A negative wire `limit` clamps to zero rows instead of wrapping into a
//!   huge unsigned the way the C++ `int`→`size_t` conversion does — nobody
//!   sends one, and zero is the only answer that is not an accident.
//! * Startup owns the schema through
//!   [`prepare_file_index_database`](compass_db::sqlite_writer::prepare_file_index_database):
//!   the writer only writes, so a fresh or stale-stamped database gets its
//!   schema here, never migrated.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use compass_core::file_walk::IndexWalk;
use compass_core::watch_events::BACKGROUND_UPDATE_INTERVAL_SECS;
use compass_db::db_writer::{IndexDatabase, ScanStatus, ScanType};
use compass_db::file_indexer::FileIndexer;
use compass_db::query_engine::{IndexedFileCategory, SearchOptions};
use compass_db::query_pool::{QueryJob, QueryPool};
use compass_db::query_reader::IndexReader;
use compass_db::scan::ScanEvent;
use serde::{Deserialize, Serialize};

use crate::indexer_watch::{IndexWatchBackend, RecentDirs};

/// The index file's name.
///
/// Not the C++ engine's `file-indexer.db`, though it sits in the same
/// directory: Compass owns its schema (ADR-0017), and the two engines stamping
/// different versions on one file would purge each other's index at every
/// start. It is a cache, so nothing is migrated — the first scan fills it.
pub const DATABASE_FILE_NAME: &str = "compass-file-index.db";

/// Query workers, like the C++ `QUERY_WORKER_COUNT`.
pub const QUERY_WORKER_COUNT: usize = 3;

/// A file kind on the wire: `FileCategory` in `file-indexer.fig`, serialized
/// by glaze as these same strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireCategory {
    /// None of the below.
    Other,
    /// A directory, whatever its name.
    Directory,
    /// An image.
    Image,
    /// A video.
    Video,
    /// A sound.
    Audio,
    /// Something to read.
    Document,
    /// An archive.
    Archive,
    /// An application.
    Application,
}

/// Which shape a scan was: `ScanKind` in `file-indexer.fig`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireScanKind {
    /// A full scan.
    Full,
    /// An incremental scan.
    Incremental,
}

/// Where a scan stands: `ScanState` in `file-indexer.fig`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireScanState {
    /// Recorded or running.
    Started,
    /// Finished.
    Succeeded,
    /// Stopped with an error.
    Failed,
    /// Stopped early.
    Interrupted,
}

/// `IndexerConfig` in `file-indexer.fig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WireIndexerConfig {
    /// Entrypoint directories to index.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Subtrees that never earn scans.
    #[serde(default)]
    pub excluded_paths: Vec<String>,
}

/// `QueryRequest` in `file-indexer.fig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WireQueryRequest {
    /// The text to rank.
    #[serde(default)]
    pub text: String,
    /// How many hits to keep.
    #[serde(default)]
    pub limit: i32,
    /// Only rows of this category, when set.
    #[serde(default)]
    pub category: Option<WireCategory>,
}

/// `FileMatch` in `file-indexer.fig`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireFileMatch {
    /// Absolute path to the file.
    pub path: String,
    /// The indexer's own relevance score.
    pub rank: f64,
    /// What kind of file it is.
    pub category: WireCategory,
    /// Its MIME type, when the indexer knows one.
    pub mime_type: Option<String>,
}

/// `QueryResponse` in `file-indexer.fig`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct WireQueryResponse {
    /// The ranked rows.
    #[serde(default)]
    pub matches: Vec<WireFileMatch>,
}

/// `ScanStatusEvent` in `file-indexer.fig`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireScanStatus {
    /// The dispatcher's id for the scan.
    pub scan_id: i32,
    /// Which shape the scan was.
    pub kind: WireScanKind,
    /// Where the scan stands.
    pub state: WireScanState,
    /// What was scanned.
    pub entrypoint: String,
    /// How many files the scan processed.
    pub processed_file_count: u32,
}

/// One incoming JSON-RPC frame: a `JsonRpcRequest` in the generated code.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RpcRequest {
    #[serde(default)]
    jsonrpc: String,
    #[serde(default)]
    method: String,
    #[serde(default)]
    id: i64,
    #[serde(default)]
    params: serde_json::Value,
}

/// `FileIndexer_configure_Params`: the single `config` parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
struct ConfigureParams {
    #[serde(default)]
    config: WireIndexerConfig,
}

/// `FileIndexer_query_Params`: the single `req` parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
struct QueryParams {
    #[serde(default)]
    req: WireQueryRequest,
}

/// The indexer's own category for a wire one, or nothing when the wire names
/// nothing known — an unknown category filters nothing, the way a failed
/// glaze parse leaves the C++ options default-constructed.
#[must_use]
pub fn indexer_category(category: WireCategory) -> IndexedFileCategory {
    match category {
        WireCategory::Other => IndexedFileCategory::Other,
        WireCategory::Directory => IndexedFileCategory::Directory,
        WireCategory::Image => IndexedFileCategory::Image,
        WireCategory::Video => IndexedFileCategory::Video,
        WireCategory::Audio => IndexedFileCategory::Audio,
        WireCategory::Document => IndexedFileCategory::Document,
        WireCategory::Archive => IndexedFileCategory::Archive,
        WireCategory::Application => IndexedFileCategory::Application,
    }
}

/// The wire category for an indexer one.
#[must_use]
pub fn wire_category(category: IndexedFileCategory) -> WireCategory {
    match category {
        IndexedFileCategory::Other => WireCategory::Other,
        IndexedFileCategory::Directory => WireCategory::Directory,
        IndexedFileCategory::Image => WireCategory::Image,
        IndexedFileCategory::Video => WireCategory::Video,
        IndexedFileCategory::Audio => WireCategory::Audio,
        IndexedFileCategory::Document => WireCategory::Document,
        IndexedFileCategory::Archive => WireCategory::Archive,
        IndexedFileCategory::Application => WireCategory::Application,
    }
}

/// The wire kind for a scan shape.
#[must_use]
pub fn wire_scan_kind(scan_type: ScanType) -> WireScanKind {
    match scan_type {
        ScanType::Full => WireScanKind::Full,
        ScanType::Incremental => WireScanKind::Incremental,
    }
}

/// The wire state for a scan status: anything not finished reads as started,
/// the way the C++ `toScanState` folds pending and running together.
#[must_use]
pub fn wire_scan_state(status: ScanStatus) -> WireScanState {
    match status {
        ScanStatus::Succeeded => WireScanState::Succeeded,
        ScanStatus::Failed => WireScanState::Failed,
        ScanStatus::Interrupted => WireScanState::Interrupted,
        ScanStatus::Pending | ScanStatus::Started => WireScanState::Started,
    }
}

/// One length-prefixed payload out, without the prefix.
///
/// Object-safe so the service holds it behind [`Arc`]: replies leave on pool
/// worker threads while events leave on dispatcher ones.
pub trait FrameSink: Send + Sync {
    /// Sends one JSON payload; the sink adds the length prefix.
    fn send(&self, frame: &[u8]);
}

/// [`FrameSink`] over stdout, flushing every frame like the C++
/// `StdoutTransport`.
#[derive(Debug)]
pub struct StdoutSink {
    out: Mutex<std::io::Stdout>,
}

impl Default for StdoutSink {
    fn default() -> Self {
        Self {
            out: Mutex::new(std::io::stdout()),
        }
    }
}

impl FrameSink for StdoutSink {
    fn send(&self, frame: &[u8]) {
        use std::io::Write;
        let mut out = self.out.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = out.write_all(&(frame.len() as u32).to_le_bytes());
        let _ = out.write_all(frame);
        let _ = out.flush();
    }
}

/// A reply frame: `{"jsonrpc":"2.0","id":N,"result":...}`, null for void
/// methods the way the C++ `reply(id, nullptr)` writes it.
#[must_use]
pub fn reply_frame(id: i64, result: &impl Serialize) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result}))
        .unwrap_or_default()
}

/// An error frame: `{"jsonrpc":"2.0","method":"","id":N,"error":...}`,
/// mirroring the generated `replyError` — including its empty method.
#[must_use]
pub fn error_frame(id: i64, message: &str) -> Vec<u8> {
    serde_json::to_vec(
        &serde_json::json!({"jsonrpc": "2.0", "method": "", "id": id, "error": message}),
    )
    .unwrap_or_default()
}

/// An event frame: `{"jsonrpc":"2.0","method":M,"params":...}`, the generated
/// `notify` shape, without an id.
#[must_use]
pub fn event_frame(method: &str, params: &impl Serialize) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"jsonrpc": "2.0", "method": method, "params": params}))
        .unwrap_or_default()
}

/// Prefixes a payload with its little-endian length, the stdio framing both
/// ends share.
#[must_use]
pub fn length_prefix(frame: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(frame.len() + 4);
    out.extend_from_slice(&(frame.len() as u32).to_le_bytes());
    out.extend_from_slice(frame);
    out
}

/// The running service: the indexer for configuration and scans, the pool
/// for queries, the sink for replies and scan events.
///
/// Built behind [`Arc`] because the scan-event callback outlives any borrow:
/// it fires on dispatcher threads.
pub struct IndexerService<D: IndexDatabase, R: IndexReader> {
    indexer: FileIndexer<D, R>,
    pool: QueryPool,
    sink: Arc<dyn FrameSink>,
    started: AtomicBool,
}

impl<D: IndexDatabase, R: IndexReader> std::fmt::Debug for IndexerService<D, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexerService")
            .field("started", &self.started.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl<D: IndexDatabase, R: IndexReader> IndexerService<D, R> {
    /// Serves `indexer` with `pool`, reporting to `sink`.
    ///
    /// Scan completions leave as `FileIndexer/scanStatusChanged` events, the
    /// way the C++ constructor wires `setScanEventCallback` to `emit`.
    #[must_use]
    pub fn new(indexer: FileIndexer<D, R>, pool: QueryPool, sink: Arc<dyn FrameSink>) -> Arc<Self> {
        let service = Arc::new(Self {
            indexer,
            pool,
            sink,
            started: AtomicBool::new(false),
        });
        let events = Arc::clone(&service);
        service
            .indexer
            .set_scan_event_callback(move |event| events.emit_scan_status(&event));
        service
    }

    /// Routes one JSON payload, the generated `Server::route`.
    ///
    /// Unknown methods and unparseable frames are ignored; unparseable params
    /// run with defaults.
    pub fn handle_frame(&self, frame: &[u8]) {
        let request: RpcRequest = match serde_json::from_slice(frame) {
            Ok(request) => request,
            Err(_) => return,
        };
        match request.method.as_str() {
            "FileIndexer/configure" => {
                let params: ConfigureParams =
                    serde_json::from_value(request.params).unwrap_or_default();
                let paths = to_paths(params.config.paths);
                let excluded = to_paths(params.config.excluded_paths);
                // First configuration starts the indexer; later ones apply,
                // like `m_started` in the C++ service.
                if self.started.swap(true, Ordering::SeqCst) {
                    self.indexer.apply_config(paths, excluded);
                } else {
                    self.indexer.set_config(paths, excluded);
                    self.indexer.start();
                }
                self.reply(request.id, &());
            }
            "FileIndexer/rebuildIndex" => {
                self.indexer.rebuild_index();
                self.reply(request.id, &());
            }
            "FileIndexer/query" => {
                let params: QueryParams =
                    serde_json::from_value(request.params).unwrap_or_default();
                let sink = Arc::clone(&self.sink);
                let id = request.id;
                self.pool.submit(QueryJob {
                    text: params.req.text,
                    limit: params.req.limit.max(0) as usize,
                    options: SearchOptions {
                        category: params.req.category.map(indexer_category),
                    },
                    on_result: Box::new(move |results| {
                        let response = WireQueryResponse {
                            matches: results
                                .into_iter()
                                .map(|result| WireFileMatch {
                                    path: result.path.to_string_lossy().into_owned(),
                                    rank: result.rank,
                                    category: wire_category(result.category),
                                    mime_type: result.mime_type,
                                })
                                .collect(),
                        };
                        sink.send(&reply_frame(id, &response));
                    }),
                });
            }
            _ => {}
        }
    }

    /// Reads length-prefixed frames until EOF, the `listen` loop: 4-byte
    /// little-endian length, then that many JSON bytes.
    pub fn run_stdio(&self, input: &mut impl BufRead) {
        loop {
            let mut header = [0u8; 4];
            if input.read_exact(&mut header).is_err() {
                return;
            }
            let mut payload = vec![0u8; u32::from_le_bytes(header) as usize];
            if input.read_exact(&mut payload).is_err() {
                return;
            }
            self.handle_frame(&payload);
        }
    }

    fn reply(&self, id: i64, result: &impl Serialize) {
        self.sink.send(&reply_frame(id, result));
    }

    fn emit_scan_status(&self, event: &ScanEvent) {
        let status = WireScanStatus {
            scan_id: event.scan_id,
            kind: wire_scan_kind(event.scan_type),
            state: wire_scan_state(event.status),
            entrypoint: event.entrypoint.to_string_lossy().into_owned(),
            processed_file_count: event.processed_file_count as u32,
        };
        self.sink.send(&event_frame(
            "FileIndexer/scanStatusChanged",
            &serde_json::json!({"status": status}),
        ));
    }
}

fn to_paths(values: Vec<String>) -> Vec<PathBuf> {
    values.into_iter().map(PathBuf::from).collect()
}

/// The database file under a cache home: `<cache>/compass/file-indexer/`
/// plus [`DATABASE_FILE_NAME`], the way `databasePath` appends to
/// `cacheDir`.
#[must_use]
pub fn database_path(cache_home: &Path) -> PathBuf {
    cache_home
        .join("compass")
        .join("file-indexer")
        .join(DATABASE_FILE_NAME)
}

/// Removes the databases the versions before `database_path`, with their
/// write-ahead logs: `<data>/compass/file-indexer.db` and
/// `<data>/compass/file-indexer/file-indexer.db` (the C++ wrote them under
/// `vicinae`, which the startup migration moves to `compass`).
///
/// Missing files are not errors — most installs never had them.
pub fn remove_legacy_db_files(data_home: &Path) {
    let data = data_home.join("compass");
    for base in [
        data.join("file-indexer.db"),
        data.join("file-indexer").join("file-indexer.db"),
    ] {
        // Order matches the C++: the logs go before the database itself.
        for suffix in ["-wal", "-shm", ""] {
            let mut file = base.as_os_str().to_owned();
            file.push(suffix);
            if std::fs::remove_file(Path::new(&file)).is_ok() {
                tracing::info!(file = ?Path::new(&file), "removed legacy file-index file");
            }
        }
    }
}

/// Builds the watch backend around the service's dispatcher: home layout for
/// the watch set, the config and data homes for its roots, and `recent`
/// answering the dynamic refreshes from a live reader.
#[must_use]
pub fn watch_backend<D, R>(
    indexer: &FileIndexer<D, R>,
    home: Option<PathBuf>,
    xdg_dirs: Vec<PathBuf>,
    walk: IndexWalk,
    recent: RecentDirs,
) -> Arc<IndexWatchBackend<D, R>>
where
    D: IndexDatabase,
    R: IndexReader,
{
    Arc::new(IndexWatchBackend::new(
        indexer.dispatcher(),
        home,
        xdg_dirs,
        walk,
        recent,
        Duration::from_secs(BACKGROUND_UPDATE_INTERVAL_SECS),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::io::Cursor;
    use std::time::Instant;

    use compass_db::db_writer::{DbWriter, FileEvent, ScanRecord, ScanStatus, ScanType};
    use compass_db::query_engine::{SearchCandidate, SearchOptions};
    use compass_db::query_policy::VocabularySuggestion;

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

        fn index_events(&mut self, _events: &[FileEvent]) {}
    }

    #[derive(Clone)]
    struct FakeReader;

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

        fn recent_directories(&self, _limit: usize) -> Vec<PathBuf> {
            Vec::new()
        }
    }

    #[derive(Debug, Default)]
    struct VecSink {
        frames: Mutex<Vec<Vec<u8>>>,
    }

    impl FrameSink for VecSink {
        fn send(&self, frame: &[u8]) {
            self.frames
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(frame.to_vec());
        }
    }

    impl VecSink {
        fn values(&self) -> Vec<serde_json::Value> {
            self.frames
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .map(|frame| serde_json::from_slice(frame).expect("valid frame JSON"))
                .collect()
        }
    }

    fn fixture() -> (Arc<IndexerService<FakeDb, FakeReader>>, Arc<VecSink>) {
        let writer = Arc::new(DbWriter::new(|| FakeDb));
        let indexer = FileIndexer::new(writer, || FakeReader, "test.db");
        let pool = QueryPool::new(QUERY_WORKER_COUNT, || FakeReader);
        let sink = Arc::new(VecSink::default());
        let service = IndexerService::new(indexer, pool, sink.clone());
        (service, sink)
    }

    fn request(id: i64, method: &str, params: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "jsonrpc": "2.0", "method": method, "id": id, "params": params,
        }))
        .expect("request JSON")
    }

    fn wait_for(
        sink: &VecSink,
        mut matches: impl FnMut(&serde_json::Value) -> bool,
        what: &str,
        timeout: Duration,
    ) -> serde_json::Value {
        let start = Instant::now();
        loop {
            if let Some(found) = sink.values().into_iter().find(&mut matches) {
                return found;
            }
            if start.elapsed() > timeout {
                panic!("timed out waiting for {what}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn wire_enums_match_the_fig_spellings() {
        let category: WireCategory =
            serde_json::from_str("\"Directory\"").expect("category parses");
        assert_eq!(category, WireCategory::Directory);
        assert_eq!(
            serde_json::to_string(&WireCategory::Application).expect("category writes"),
            "\"Application\""
        );
        assert_eq!(
            serde_json::to_string(&WireScanKind::Incremental).expect("kind writes"),
            "\"Incremental\""
        );
        assert_eq!(
            serde_json::to_string(&wire_scan_state(ScanStatus::Started)).expect("state writes"),
            "\"Started\""
        );
        assert_eq!(wire_scan_state(ScanStatus::Pending), WireScanState::Started);
        assert_eq!(
            wire_scan_state(ScanStatus::Succeeded),
            WireScanState::Succeeded
        );
        assert_eq!(
            indexer_category(WireCategory::Image),
            IndexedFileCategory::Image
        );
        assert_eq!(
            wire_category(IndexedFileCategory::Audio),
            WireCategory::Audio
        );
    }

    #[test]
    fn configure_replies_null_and_scans_notify() {
        let home = tempfile::tempdir().expect("tempdir");
        let (service, sink) = fixture();
        service.handle_frame(&request(
            1,
            "FileIndexer/configure",
            serde_json::json!({"config": {"paths": [home.path().to_string_lossy()], "excluded_paths": []}}),
        ));

        // The void reply carries a null result, like `reply(id, nullptr)`.
        let reply = wait_for(
            &sink,
            |value| value.get("id") == Some(&serde_json::json!(1)),
            "configure reply",
            Duration::from_secs(5),
        );
        assert_eq!(reply.get("result"), Some(&serde_json::Value::Null));

        // The full scan the first configuration starts ends in a status
        // event naming the entrypoint.
        let event = wait_for(
            &sink,
            |value| {
                value.get("method") == Some(&serde_json::json!("FileIndexer/scanStatusChanged"))
            },
            "scan event",
            Duration::from_secs(15),
        );
        assert_eq!(
            event.pointer("/params/status/entrypoint"),
            Some(&serde_json::Value::String(
                home.path().to_string_lossy().into_owned()
            ))
        );
    }

    #[test]
    fn query_replies_with_matches_for_its_id() {
        let (service, sink) = fixture();
        service.handle_frame(&request(
            7,
            "FileIndexer/query",
            serde_json::json!({"req": {"text": "report", "limit": 10}}),
        ));

        let reply = wait_for(
            &sink,
            |value| value.get("id") == Some(&serde_json::json!(7)),
            "query reply",
            Duration::from_secs(10),
        );
        assert_eq!(
            reply.pointer("/result/matches"),
            Some(&serde_json::json!([]))
        );
    }

    #[test]
    fn rebuild_replies_null() {
        let (service, sink) = fixture();
        service.handle_frame(&request(
            3,
            "FileIndexer/rebuildIndex",
            serde_json::json!({}),
        ));

        let reply = wait_for(
            &sink,
            |value| value.get("id") == Some(&serde_json::json!(3)),
            "rebuild reply",
            Duration::from_secs(10),
        );
        assert_eq!(reply.get("result"), Some(&serde_json::Value::Null));
    }

    #[test]
    fn unknown_methods_and_garbage_are_silent() {
        let (service, sink) = fixture();
        service.handle_frame(&request(9, "FileIndexer/vanish", serde_json::json!({})));
        service.handle_frame(b"not json at all");
        service.handle_frame(&request(
            10,
            "FileIndexer/query",
            serde_json::json!({"req": {"text": "x", "limit": -5}}),
        ));

        // Only the well-formed query answers; the negative limit clamps to
        // zero rows instead of wrapping.
        let reply = wait_for(
            &sink,
            |value| value.get("id") == Some(&serde_json::json!(10)),
            "clamped query reply",
            Duration::from_secs(10),
        );
        assert_eq!(
            reply.pointer("/result/matches"),
            Some(&serde_json::json!([]))
        );
        assert!(
            sink.values()
                .iter()
                .all(|value| value.get("id") != Some(&serde_json::json!(9))),
            "unknown method stays silent"
        );
    }

    #[test]
    fn stdio_loop_routes_framed_requests() {
        let writer = Arc::new(DbWriter::new(|| FakeDb));
        let indexer = FileIndexer::new(writer, || FakeReader, "test.db");
        let pool = QueryPool::new(QUERY_WORKER_COUNT, || FakeReader);
        let sink = Arc::new(VecSink::default());
        let service = IndexerService::new(indexer, pool, sink.clone());
        let framed = length_prefix(&request(
            4,
            "FileIndexer/rebuildIndex",
            serde_json::json!({}),
        ));
        service.run_stdio(&mut Cursor::new(framed));

        let values = sink.values();
        assert_eq!(values.len(), 1, "one frame in, one reply out");
        assert_eq!(values[0].get("id"), Some(&serde_json::json!(4)));
    }

    #[test]
    fn startup_paths_follow_the_xdg_layout() {
        let cache = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            database_path(cache.path()),
            cache
                .path()
                .join("compass")
                .join("file-indexer")
                .join("compass-file-index.db")
        );

        let data = tempfile::tempdir().expect("tempdir");
        let legacy = data.path().join("compass").join("file-indexer.db");
        std::fs::create_dir_all(legacy.parent().expect("parent")).expect("parents");
        std::fs::write(&legacy, "old").expect("legacy db");
        std::fs::write(data.path().join("compass").join("other.db"), "keep").expect("unrelated db");
        remove_legacy_db_files(data.path());
        assert!(!legacy.exists(), "legacy database removed");
        assert!(
            data.path().join("compass").join("other.db").exists(),
            "unrelated files kept"
        );
        // Missing files are not errors.
        remove_legacy_db_files(data.path());
    }
}
