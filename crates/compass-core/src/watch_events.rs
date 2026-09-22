//! What a filesystem event turns into: which scans get enqueued.
//!
//! Ports the routing half of `FileSystemWatcher` (`handleEvent`,
//! `shouldScanPath`, the background sweep, the dynamic-watch refresh) —
//! leaving the inotify plumbing (a later `notify`-crate slice), the scan
//! dispatcher, and the index database behind. Everything here is pure over the
//! configured roots, so it pins without a running watcher.

use std::path::{Path, PathBuf};

use crate::incremental_scan::Mode;
use crate::scan_roots::{is_covered_by_any, lexically_normal};

/// How deep a background sweep goes: five levels, like the C++.
pub const BACKGROUND_UPDATE_DEPTH: usize = 5;

/// How often the background sweep runs, in seconds: every minute.
pub const BACKGROUND_UPDATE_INTERVAL_SECS: u64 = 60;

/// How many recently-changed directories earn a dynamic watch.
pub const DYNAMIC_WATCH_COUNT: usize = 2048;

/// What the directory watcher reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// Something changed under a watched directory.
    DirectoryChanged(PathBuf),
    /// Watches were lost; fall back to rescanning the roots.
    Degraded,
}

/// A scan the router asks the dispatcher to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanRequest {
    /// What to scan.
    pub path: PathBuf,
    /// Which shape of incremental scan.
    pub mode: Mode,
    /// Whether it waits out the quiet period (`enqueueDebounced`) or runs as
    /// soon as a worker is free (`enqueue`).
    pub debounced: bool,
    /// How deep to descend, in components below the scan path.
    pub max_depth: Option<usize>,
    /// Extra paths to exclude, carried into the scan.
    pub excluded_paths: Vec<PathBuf>,
    /// Extra filenames to exclude, carried into the scan.
    pub excluded_filenames: Vec<String>,
}

/// The roots a watcher serves, and what it serves them with.
#[derive(Debug, Clone, Default)]
pub struct WatchConfig {
    /// The directory entrypoints that earn scans.
    pub entrypoints: Vec<PathBuf>,
    /// Paths that never earn scans, even under an entrypoint.
    pub excluded_paths: Vec<PathBuf>,
    /// Filenames excluded from every scan this watcher asks for.
    pub excluded_filenames: Vec<String>,
}

impl WatchConfig {
    /// A scan request for `path` in `mode`, carrying this watcher's exclusions.
    fn request(
        &self,
        path: PathBuf,
        mode: Mode,
        debounced: bool,
        max_depth: Option<usize>,
    ) -> ScanRequest {
        ScanRequest {
            path,
            mode,
            debounced,
            max_depth,
            excluded_paths: self.excluded_paths.clone(),
            excluded_filenames: self.excluded_filenames.clone(),
        }
    }
}

/// Whether an event at `path` earns a scan.
///
/// Covered by an entrypoint and not by an exclusion, after lexical
/// normalization — `a/../b` and `a//b` are the same directory, and the router
/// decides on the directory, not the spelling.
#[must_use]
pub fn should_scan_path(config: &WatchConfig, path: &Path) -> bool {
    let normalized = lexically_normal(path);
    is_covered_by_any(&normalized, &config.entrypoints)
        && !is_covered_by_any(&normalized, &config.excluded_paths)
}

/// The scans an event earns.
///
/// A change under a covered directory earns one debounced pruned scan of the
/// normalized directory; anything else earns nothing. A degraded watcher
/// cannot say what changed, so every covered entrypoint earns its own debounced
/// pruned scan instead.
#[must_use]
pub fn requests_for_event(config: &WatchConfig, event: &WatchEvent) -> Vec<ScanRequest> {
    match event {
        WatchEvent::DirectoryChanged(dir) => {
            if !should_scan_path(config, dir) {
                return Vec::new();
            }
            vec![config.request(lexically_normal(dir), Mode::Pruned, true, None)]
        }
        WatchEvent::Degraded => config
            .entrypoints
            .iter()
            .filter(|dir| should_scan_path(config, dir))
            .map(|dir| config.request(dir.clone(), Mode::Pruned, true, None))
            .collect(),
    }
}

/// The background sweep's scans: every entrypoint that is still a directory
/// earns an exhaustive scan five deep, run immediately rather than debounced.
///
/// The paths go out as configured, not normalized — the C++ enqueues the
/// entrypoint as it holds it.
#[must_use]
pub fn background_sweep_requests(
    config: &WatchConfig,
    is_directory: impl Fn(&Path) -> bool,
) -> Vec<ScanRequest> {
    config
        .entrypoints
        .iter()
        .filter(|dir| is_directory(dir))
        .map(|dir| {
            config.request(
                dir.clone(),
                Mode::Exhaustive,
                false,
                Some(BACKGROUND_UPDATE_DEPTH),
            )
        })
        .collect()
}

/// The directories that earn a dynamic watch, newest first.
///
/// The database deals the most recently changed directories (up to
/// [`DYNAMIC_WATCH_COUNT`]); the router keeps the ones that would earn scans.
/// Capped here as well as there, so a caller passing more than the count gets
/// the same answer the C++ gets from its database.
#[must_use]
pub fn dynamic_watch_dirs(config: &WatchConfig, recent: Vec<PathBuf>) -> Vec<PathBuf> {
    recent
        .into_iter()
        .filter(|dir| should_scan_path(config, dir))
        .take(DYNAMIC_WATCH_COUNT)
        .collect()
}
