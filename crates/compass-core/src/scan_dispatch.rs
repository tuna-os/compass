//! Deciding when a filesystem change turns into a scan, and which scans run.
//!
//! Ports the parts of `ScanDispatcher` that are decisions rather than threads:
//! the debounce that turns a burst of filesystem events into one scan, and the
//! queue rules that keep two scans off the same directory.
//!
//! Time is passed in rather than read, so the rules can be tested at the
//! boundaries that matter without waiting thirty seconds for each one.

use std::path::{Path, PathBuf};

/// How long a directory must be quiet before its scan runs.
///
/// Saving a file in an editor is several filesystem events in a row, and a
/// build is thousands; scanning on each would keep the indexer permanently
/// busy re-reading a tree that is still changing.
pub const DEBOUNCE_QUIET_SECS: u64 = 5;

/// The longest a scan can be held off, however busy the directory stays.
///
/// Without this ceiling a directory written to every few seconds — a log
/// directory, a build tree, a downloads folder mid-download — would have its
/// scan pushed back for as long as the writing continued, which is to say
/// never scanned at all.
pub const DEBOUNCE_MAX_DELAY_SECS: u64 = 30;

/// How many scans run at once.
pub const WORKER_COUNT: usize = 2;

/// Which kind of scan was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScanType {
    /// Read everything.
    Full,
    /// Read what changed.
    Incremental,
}

/// A scan waiting out its quiet period.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    /// What is to be scanned.
    pub path: PathBuf,
    /// Which kind of scan.
    pub scan_type: ScanType,
    /// When it may run, in seconds.
    pub deadline: u64,
    /// When the first event in this burst arrived.
    pub first_seen: u64,
}

/// The scans waiting to be run.
#[derive(Debug, Default)]
pub struct Debouncer {
    pending: Vec<Pending>,
}

impl Debouncer {
    /// An empty debouncer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything currently waiting.
    #[must_use]
    pub fn pending(&self) -> &[Pending] {
        &self.pending
    }

    /// Records a filesystem change at `now`.
    ///
    /// Pending scans are keyed by **path and type together**, so a full scan
    /// and an incremental scan of the same directory wait separately — they do
    /// different work and one is not a substitute for the other.
    ///
    /// A first event starts a quiet period. A later one pushes the deadline out
    /// again, but never past `first_seen + DEBOUNCE_MAX_DELAY_SECS`: the
    /// `min` is what stops a directory that is written to forever from being
    /// scanned never.
    pub fn record(&mut self, path: &Path, scan_type: ScanType, now: u64) {
        if let Some(existing) = self
            .pending
            .iter_mut()
            .find(|item| item.path == path && item.scan_type == scan_type)
        {
            existing.deadline =
                (now + DEBOUNCE_QUIET_SECS).min(existing.first_seen + DEBOUNCE_MAX_DELAY_SECS);
            return;
        }

        self.pending.push(Pending {
            path: path.to_path_buf(),
            scan_type,
            deadline: now + DEBOUNCE_QUIET_SECS,
            first_seen: now,
        });
    }

    /// When the scheduler next has something to do.
    #[must_use]
    pub fn next_deadline(&self) -> Option<u64> {
        self.pending.iter().map(|item| item.deadline).min()
    }

    /// Takes everything whose deadline has arrived, in insertion order.
    ///
    /// The comparison is `deadline > now` for *keeping*, so a scan due exactly
    /// now runs now.
    pub fn take_due(&mut self, now: u64) -> Vec<Pending> {
        let mut due = Vec::new();
        self.pending.retain(|item| {
            if item.deadline > now {
                return true;
            }
            due.push(item.clone());
            false
        });
        due
    }

    /// Puts a scan back that could not be started, restarting its quiet period.
    ///
    /// A scan refused because one is already running on that path is **not
    /// dropped**: the events that asked for it are real and the running scan
    /// may already have passed the files they touched.
    pub fn rearm(&mut self, scan: &Pending, now: u64) {
        self.record(&scan.path, scan.scan_type, now);
    }

    /// Forgets everything waiting.
    pub fn clear(&mut self) {
        self.pending.clear();
    }
}

/// A scan that has been accepted, running or waiting for a worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    /// Its id, which is what an interrupt names.
    pub id: i64,
    /// What is being scanned.
    pub path: PathBuf,
    /// Which kind of scan.
    pub scan_type: ScanType,
    /// Whether a worker has picked it up.
    pub running: bool,
}

/// The accepted scans: what is running and what is queued.
#[derive(Debug, Default)]
pub struct ScanQueue {
    scans: Vec<Accepted>,
    next_id: i64,
}

impl ScanQueue {
    /// An empty queue, handing out ids from 0.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything accepted, in the order it was accepted.
    ///
    /// The C++ reports its running scans before its queued ones, but takes the
    /// running ones out of an unordered map, so their order is not a
    /// behaviour anything could rely on. Insertion order is defined, which is
    /// the better answer for a list something might display.
    #[must_use]
    pub fn scans(&self) -> &[Accepted] {
        &self.scans
    }

    /// Accepts a scan, or refuses it because that path is already being
    /// scanned.
    ///
    /// The duplicate check is on the **path alone**, not the path and type: two
    /// scans of one directory would read the same files twice and race each
    /// other's writes, whatever kinds they are.
    pub fn enqueue(&mut self, path: &Path, scan_type: ScanType) -> Option<i64> {
        if self.scans.iter().any(|scan| scan.path == path) {
            return None;
        }

        let id = self.next_id;
        self.next_id += 1;
        self.scans.push(Accepted {
            id,
            path: path.to_path_buf(),
            scan_type,
            running: false,
        });
        Some(id)
    }

    /// Hands the oldest queued scan to a worker, if there is a free one.
    pub fn start_next(&mut self) -> Option<&Accepted> {
        if self.scans.iter().filter(|scan| scan.running).count() >= WORKER_COUNT {
            return None;
        }

        let next = self.scans.iter().position(|scan| !scan.running)?;
        self.scans[next].running = true;
        self.scans.get(next)
    }

    /// Takes a finished scan off the queue.
    pub fn finish(&mut self, id: i64) {
        self.scans.retain(|scan| scan.id != id);
    }

    /// Interrupts one scan.
    ///
    /// A scan that has not started is simply removed; a running one is
    /// reported so the caller can stop it, and stays until it finishes, because
    /// a scanner that has been told to stop has not stopped yet.
    pub fn interrupt(&mut self, id: i64) -> Interrupted {
        let Some(index) = self.scans.iter().position(|scan| scan.id == id) else {
            return Interrupted::NoSuchScan;
        };

        if self.scans[index].running {
            return Interrupted::Running;
        }

        self.scans.remove(index);
        Interrupted::Removed
    }

    /// Interrupts everything: queued scans are dropped, running ones reported.
    pub fn interrupt_all(&mut self) -> Vec<i64> {
        let running: Vec<i64> = self
            .scans
            .iter()
            .filter(|scan| scan.running)
            .map(|scan| scan.id)
            .collect();

        self.scans.retain(|scan| scan.running);
        running
    }

    /// Whether there is nothing running and nothing queued.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.scans.is_empty()
    }
}

/// What interrupting a scan did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interrupted {
    /// It was queued and has been dropped.
    Removed,
    /// It is running and has been told to stop.
    Running,
    /// There is no scan with that id.
    NoSuchScan,
}
