//! The file-indexer client: the engine side of the stdio service.
//!
//! Ports the files-service `FileIndexer`: it finds the
//! `vicinae-file-indexer` helper, starts it, keeps it configured, asks it
//! queries, and tracks its scans — restarting it with backoff when it
//! crashes, and reporting every scan it never finishes as interrupted.
//!
//! The wire shapes are shared with [`crate::indexer_service`]: requests go
//! out as `FileIndexer/<method>` frames, replies come back by id, and scan
//! completions arrive as `FileIndexer/scanStatusChanged` events.
//!
//! # Deltas from the C++, on purpose
//!
//! * The C++ restarts on a Qt timer while the event loop keeps serving;
//!   here the reader thread sleeps the backoff itself and restarts inline,
//!   because a dead process has nothing left to route.
//! * Queries block their caller instead of returning a future: the C++
//!   future resolves on the reply or not at all, and a blocking wait with
//!   the same two outcomes is the honest sync translation. A crash settles
//!   every outstanding query empty, the way the C++ answers nothing.
//! * The helper search drops the compile-time `VICINAE_LIBEXEC_PATH` for
//!   `../libexec/vicinae` beside the executable, which covers the same
//!   installed layout without baking a prefix in.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use compass_db::query_engine::IndexerFileResult;

use crate::indexer_service::{
    WireCategory, WireIndexerConfig, WireQueryRequest, WireQueryResponse, WireScanState,
    WireScanStatus, indexer_category,
};

/// The helper binary's file name.
pub const HELPER_PROGRAM: &str = "vicinae-file-indexer";

/// Crash restarts before giving up, like the C++ `MAX_RESTART_ATTEMPTS`.
pub const MAX_RESTART_ATTEMPTS: u32 = 5;

/// The first restart delay; each attempt doubles it, like the C++
/// `BASE_RESTART_DELAY_MS * (1 << (crash - 1))`.
pub const BASE_RESTART_DELAY: Duration = Duration::from_millis(1000);

/// Where the helper is looked for: beside the current executable, then in
/// `../libexec/vicinae`, the installed layout the C++ candidates cover.
#[must_use]
pub fn helper_candidates(exe_dir: &Path, program: &str) -> Vec<PathBuf> {
    let mut name = program.to_owned();
    if cfg!(windows) {
        name.push_str(".exe");
    }
    let mut out = Vec::with_capacity(2);
    out.push(exe_dir.join(&name));
    if let Some(parent) = exe_dir.parent() {
        out.push(parent.join("libexec").join("vicinae").join(&name));
    }
    out
}

/// The first candidate that is a regular file, or nothing when the helper
/// is not installed — in which case file search does not work, and says so.
#[must_use]
pub fn find_helper_program(program: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    helper_candidates(dir, program)
        .into_iter()
        .find(|path| path.is_file())
}

/// The backoff before attempt `attempt` (1-based): the base doubled each
/// time, saturating instead of overflowing on absurd counts.
#[must_use]
pub fn restart_delay(attempt: u32) -> Duration {
    restart_delay_from(BASE_RESTART_DELAY, attempt)
}

/// The same backoff from an explicit base, so tests can shrink the waits
/// without changing what production sleeps.
#[must_use]
pub fn restart_delay_from(base: Duration, attempt: u32) -> Duration {
    let shift = attempt.saturating_sub(1).min(30);
    base.saturating_mul(1 << shift)
}

/// A scan the helper reported, tracked until it terminates.
pub type ActiveScan = WireScanStatus;

/// Reports one scan completion, including synthesized interruptions.
pub type ScanCallback = Arc<dyn Fn(ActiveScan) + Send + Sync>;

/// Whether a scan state ends the scan: anything but started.
#[must_use]
pub fn is_terminal_state(state: WireScanState) -> bool {
    state != WireScanState::Started
}

type QueryReply = Result<WireQueryResponse, String>;

/// The engine side of one file-indexer helper process.
///
/// Built behind [`Arc`] because the reader thread outlives any borrow: it
/// routes replies and scan events until the process dies, then restarts it.
pub struct IndexerClient {
    inner: Arc<Inner>,
}

struct Inner {
    config: Mutex<WireIndexerConfig>,
    want_running: AtomicBool,
    state: Mutex<State>,
    next_id: AtomicI64,
    pending: Mutex<HashMap<i64, std::sync::mpsc::Sender<QueryReply>>>,
    scans: Mutex<HashMap<i32, ActiveScan>>,
    scan_callback: Mutex<Option<ScanCallback>>,
    program_override: Mutex<Option<PathBuf>>,
    base_delay: Mutex<Duration>,
}

struct State {
    generation: u64,
    stdin: Option<ChildStdin>,
    stdin_owner: Option<u64>,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for IndexerClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexerClient")
            .field("running", &self.is_running())
            .finish_non_exhaustive()
    }
}

impl IndexerClient {
    /// Stages the client: nothing runs until [`start`](Self::start).
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(Inner {
                config: Mutex::new(WireIndexerConfig::default()),
                want_running: AtomicBool::new(false),
                state: Mutex::new(State {
                    generation: 0,
                    stdin: None,
                    stdin_owner: None,
                    reader: None,
                }),
                next_id: AtomicI64::new(1),
                pending: Mutex::new(HashMap::new()),
                scans: Mutex::new(HashMap::new()),
                scan_callback: Mutex::new(None),
                program_override: Mutex::new(None),
                base_delay: Mutex::new(BASE_RESTART_DELAY),
            }),
        })
    }

    /// Shrinks the crash-restart backoff, so the give-up test does not sleep
    /// through the production delays. Production never calls this.
    pub fn set_base_restart_delay(&self, delay: Duration) {
        *self
            .inner
            .base_delay
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = delay;
    }

    /// Reports every scan completion to `callback`, replacing any previous
    /// one — including the interruptions synthesized when the helper dies.
    pub fn set_scan_callback(&self, callback: impl Fn(ActiveScan) + Send + Sync + 'static) {
        *self
            .inner
            .scan_callback
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(callback));
    }

    /// Whether the helper process currently runs.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stdin
            .is_some()
    }

    /// Starts the helper with the current configuration, unless it runs.
    pub fn start(self: &Arc<Self>) {
        self.inner.want_running.store(true, Ordering::SeqCst);
        self.ensure_thread();
    }

    /// Starts an explicit helper binary instead of searching for one.
    ///
    /// The tests point this at a scripted fake; production uses [`start`](Self::start).
    pub fn start_with(self: &Arc<Self>, program: &Path) {
        *self
            .inner
            .program_override
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(program.to_path_buf());
        self.start();
    }

    /// Stops wanting the helper: closes its stdin so it exits, and joins
    /// the reader. Attempts are counted per run, so a later
    /// [`start`](Self::start) begins them over.
    pub fn stop(&self) {
        self.inner.want_running.store(false, Ordering::SeqCst);
        let reader = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            state.generation += 1;
            state.stdin = None;
            state.stdin_owner = None;
            state.reader.take()
        };
        if let Some(reader) = reader {
            let _ = reader.join();
        }
    }

    /// Replaces the configuration, sending it when the helper runs —
    /// otherwise the next start sends it, the way `sendConfigure` guards on
    /// `isRunning` and preferences resend on change.
    pub fn configure(&self, paths: Vec<String>, excluded_paths: Vec<String>) {
        *self
            .inner
            .config
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = WireIndexerConfig {
            paths,
            excluded_paths,
        };
        self.send_configure();
    }

    /// Asks a running helper to rebuild; silent when it does not run.
    pub fn rebuild_index(&self) {
        self.write_frame("FileIndexer/rebuildIndex", &());
    }

    /// Asks the helper a query, waiting for its reply.
    ///
    /// A helper that is not running answers nothing, like the C++ ready-made
    /// empty future; a crash or a reply error settles the same empty way.
    /// A negative `limit` rides along untouched — the service clamps it.
    #[must_use]
    pub fn query(
        &self,
        text: &str,
        limit: i32,
        category: Option<WireCategory>,
    ) -> Vec<IndexerFileResult> {
        let (sender, receiver) = std::sync::mpsc::channel();
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        {
            let mut pending = self
                .inner
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            pending.insert(id, sender);
        }
        let params = serde_json::json!({"req": WireQueryRequest {
            text: text.to_owned(),
            limit,
            category,
        }});
        if self.write_raw(id, "FileIndexer/query", &params).is_none() {
            self.inner
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id);
            return Vec::new();
        }
        match receiver.recv() {
            Ok(Ok(response)) => response
                .matches
                .into_iter()
                .map(|wire| IndexerFileResult {
                    path: PathBuf::from(wire.path),
                    rank: wire.rank,
                    category: indexer_category(wire.category),
                    mime_type: wire.mime_type,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The scans the helper reported and never finished.
    #[must_use]
    pub fn active_scans(&self) -> Vec<ActiveScan> {
        self.inner
            .scans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }

    /// Spawns the reader thread unless one runs.
    fn ensure_thread(self: &Arc<Self>) {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if state.reader.is_some() {
            return;
        }
        let client = Arc::clone(self);
        let generation = state.generation;
        state.reader = Some(std::thread::spawn(move || client.supervise(generation)));
    }

    /// One thread per wanted lifetime: start the process, route it until it
    /// dies, restart with backoff while it is still wanted, and give up
    /// after [`MAX_RESTART_ATTEMPTS`] crashes — the count spans restarts, so
    /// a helper that starts only to die does not loop forever.
    fn supervise(self: Arc<Self>, generation: u64) {
        let mut crashes: u32 = 0;
        loop {
            if !self.inner.want_running.load(Ordering::SeqCst) {
                break;
            }
            let Some((stdout, child)) = self.start_process(generation) else {
                break;
            };
            // A stop() that landed while the helper spawned leaves this
            // generation stale: close the stdin just stored so the helper
            // sees EOF, reap it, and leave without routing — otherwise
            // stop() joins a supervisor that never comes home.
            if !self.inner.want_running.load(Ordering::SeqCst)
                || self.current_generation() != generation
            {
                drop(stdout);
                let mut child = child;
                self.drop_stdin();
                child.kill().ok();
                child.wait().ok();
                break;
            }
            let status = self.route(stdout, child);
            if !self.inner.want_running.load(Ordering::SeqCst) {
                break;
            }
            if self.current_generation() != generation {
                break;
            }
            match status {
                Some(status) if status.success() => {
                    tracing::info!("file indexer exited cleanly");
                    break;
                }
                status => {
                    tracing::warn!(?status, "file indexer process ended");
                }
            }
            self.interrupt_scans();
            self.fail_pending("the file indexer died");
            crashes += 1;
            if crashes > MAX_RESTART_ATTEMPTS {
                tracing::error!(
                    crashes,
                    "file indexer crashed too many times, giving up on restart"
                );
                break;
            }
            let delay = restart_delay_from(self.base_delay(), crashes);
            tracing::warn!(?delay, attempt = crashes, "restarting the file indexer");
            std::thread::sleep(delay);
        }
        self.release(generation);
    }

    fn base_delay(&self) -> Duration {
        *self
            .inner
            .base_delay
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Closes the helper's stdin, so it sees EOF and exits politely.
    fn drop_stdin(&self) {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.stdin = None;
        state.stdin_owner = None;
    }

    /// Leaves the supervisor: forgets the reader, and the stdin this
    /// generation stored — a newer generation's stdin is not ours to close.
    /// After a give-up or a clean exit nothing runs, so [`is_running`](Self::is_running)
    /// stops claiming otherwise.
    fn release(&self, generation: u64) {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if state.stdin_owner == Some(generation) {
            state.stdin = None;
            state.stdin_owner = None;
        }
        state.reader = None;
    }

    /// Resolves the helper, spawns it, and sends the configuration.
    ///
    /// Returns the stdout to route plus the child to reap — the supervisor
    /// owns the wait, so no helper ever stays a zombie. Nothing means there
    /// is nothing to route: no helper installed, or it would not spawn.
    fn start_process(
        &self,
        generation: u64,
    ) -> Option<(BufReader<std::process::ChildStdout>, Child)> {
        let program = self
            .inner
            .program_override
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .or_else(|| find_helper_program(HELPER_PROGRAM));
        let Some(program) = program else {
            tracing::warn!(
                "could not find vicinae-file-indexer helper binary, file search will not work"
            );
            return None;
        };
        let mut child = match Command::new(&program)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                tracing::error!(program = ?program, error = %error, "failed to start the file indexer");
                return None;
            }
        };
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        std::thread::spawn(move || drain_stderr(stderr));
        {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            state.stdin = stdin;
            state.stdin_owner = Some(generation);
        }
        self.send_configure();
        Some((BufReader::new(stdout?), child))
    }

    /// Routes stdout frames until EOF, then reaps the child and reports how
    /// it ended.
    fn route(
        &self,
        stdout: BufReader<std::process::ChildStdout>,
        mut child: Child,
    ) -> Option<std::process::ExitStatus> {
        self.route_frames(stdout);
        child.wait().ok()
    }

    /// Routes length-prefixed JSON frames until the helper closes stdout:
    /// replies complete their query by id, scan events update the tracked
    /// scans, and anything else is logged and ignored like a failed route.
    fn route_frames(&self, mut stdout: BufReader<std::process::ChildStdout>) {
        loop {
            let mut header = [0u8; 4];
            if stdout.read_exact(&mut header).is_err() {
                return;
            }
            let mut payload = vec![0u8; u32::from_le_bytes(header) as usize];
            if stdout.read_exact(&mut payload).is_err() {
                return;
            }
            self.route_frame(&payload);
        }
    }

    /// Routes one JSON payload: a reply by id, a scan event by method.
    fn route_frame(&self, frame: &[u8]) {
        let value: serde_json::Value = match serde_json::from_slice(frame) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(error = %error, "ignoring an unparsable file indexer frame");
                return;
            }
        };
        if let Some(id) = value.get("id").and_then(serde_json::Value::as_i64) {
            self.complete_query(id, &value);
        } else if let Some(method) = value.get("method").and_then(serde_json::Value::as_str) {
            self.route_event(method, &value);
        } else {
            tracing::warn!("ignoring a file indexer frame with neither id nor method");
        }
    }

    /// Completes the query holding `id`: a result deserializes into matches,
    /// an error — or a result that will not parse — settles empty.
    fn complete_query(&self, id: i64, value: &serde_json::Value) {
        let sender = self
            .inner
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
        let Some(sender) = sender else {
            return;
        };
        if value.get("error").is_some() {
            let _ = sender.send(Err("the file indexer refused the query".to_owned()));
            return;
        }
        let response = value
            .get("result")
            .and_then(|result| serde_json::from_value::<WireQueryResponse>(result.clone()).ok())
            .unwrap_or_default();
        let _ = sender.send(Ok(response));
    }

    /// Tracks a scan event, forgetting scans that terminated.
    fn route_event(&self, method: &str, value: &serde_json::Value) {
        if method != "FileIndexer/scanStatusChanged" {
            tracing::warn!(method, "ignoring an unknown file indexer event");
            return;
        }
        let Some(status) = value
            .get("params")
            .and_then(|params| params.get("status"))
            .and_then(|status| serde_json::from_value::<WireScanStatus>(status.clone()).ok())
        else {
            tracing::warn!("ignoring a scan event that will not parse");
            return;
        };
        let terminal = is_terminal_state(status.state);
        {
            let mut scans = self
                .inner
                .scans
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if terminal {
                scans.remove(&status.scan_id);
            } else {
                scans.insert(status.scan_id, status.clone());
            }
        }
        self.emit_scan(status);
    }

    fn current_generation(&self) -> u64 {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .generation
    }

    /// Sends the current configuration when the helper runs.
    fn send_configure(&self) {
        let config = self
            .inner
            .config
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        self.write_frame(
            "FileIndexer/configure",
            &serde_json::json!({"config": config}),
        );
    }

    /// Writes one request frame, returning its id — or nothing when the
    /// helper does not run.
    fn write_frame(&self, method: &str, params: &impl serde::Serialize) -> Option<i64> {
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        self.write_raw(id, method, params)
    }

    fn write_raw(&self, id: i64, method: &str, params: &impl serde::Serialize) -> Option<i64> {
        let payload = serde_json::to_vec(&serde_json::json!({
            "jsonrpc": "2.0", "method": method, "id": id, "params": params,
        }))
        .ok()?;
        let frame = crate::indexer_service::length_prefix(&payload);
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let stdin = state.stdin.as_mut()?;
        if stdin.write_all(&frame).is_err() {
            return None;
        }
        let _ = stdin.flush();
        Some(id)
    }

    /// Marks every tracked scan interrupted and reports each one, the way
    /// `interruptActiveScans` settles the map the crash orphaned.
    fn interrupt_scans(&self) {
        let scans: Vec<ActiveScan> = {
            let mut scans = self
                .inner
                .scans
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            std::mem::take(&mut *scans).into_values().collect()
        };
        for mut scan in scans {
            scan.state = WireScanState::Interrupted;
            self.emit_scan(scan);
        }
    }

    /// Settles every outstanding query with `message`.
    fn fail_pending(&self, message: &str) {
        let pending = {
            let mut pending = self
                .inner
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            std::mem::take(&mut *pending)
        };
        for (_, sender) in pending {
            let _ = sender.send(Err(message.to_owned()));
        }
    }

    fn emit_scan(&self, scan: ActiveScan) {
        if let Some(callback) = self
            .inner
            .scan_callback
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            callback(scan);
        }
    }
}

/// Logs the helper's stderr line by line, the way the C++ splits on
/// newlines and forwards each through the subprocess log.
fn drain_stderr(stderr: Option<impl Read + Send + 'static>) {
    let Some(stderr) = stderr else {
        return;
    };
    for line in BufReader::new(stderr).lines() {
        match line {
            Ok(line) => tracing::warn!(target: "vicinae-file-indexer", "{line}"),
            Err(_) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_search_covers_the_installed_layout() {
        let exe_dir = Path::new("/app/bin");
        assert_eq!(
            helper_candidates(exe_dir, "vicinae-file-indexer"),
            [
                PathBuf::from("/app/bin/vicinae-file-indexer"),
                PathBuf::from("/app/libexec/vicinae/vicinae-file-indexer"),
            ]
        );
        assert!(find_helper_program("vicinae-no-such-helper").is_none());
    }

    #[test]
    fn restart_backoff_doubles_from_one_second() {
        assert_eq!(restart_delay(1), Duration::from_secs(1));
        assert_eq!(restart_delay(2), Duration::from_secs(2));
        assert_eq!(restart_delay(3), Duration::from_secs(4));
        assert_eq!(restart_delay(0), Duration::from_secs(1));
        assert_eq!(
            restart_delay_from(Duration::from_millis(5), 3),
            Duration::from_millis(20)
        );
        assert_eq!(restart_delay(u32::MAX), Duration::MAX);
    }

    #[test]
    fn only_unfinished_scans_stay_tracked() {
        assert!(!is_terminal_state(WireScanState::Started));
        assert!(is_terminal_state(WireScanState::Succeeded));
        assert!(is_terminal_state(WireScanState::Failed));
        assert!(is_terminal_state(WireScanState::Interrupted));
    }

    #[test]
    fn a_stopped_client_answers_nothing_and_tracks_nothing() {
        let client = IndexerClient::new();
        assert!(!client.is_running());
        assert!(client.active_scans().is_empty());
        assert!(client.query("report", 10, None).is_empty());
        client.rebuild_index();
        client.stop();
    }
}
