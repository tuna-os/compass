//! Watching important directories for structural changes, on `notify`.
//!
//! Ports `LinuxImportantDirectoryWatcher`. The policy it enforces already
//! lives in shared crates — [`important_roots`], [`build_watch_set`], and the
//! [`WatchEvent`] mapping — so this is the live half: one non-recursive watch
//! per directory in the set, new directories watched as they appear, queue
//! overflow degrading to a rescan, and dynamic leaf watches from the
//! recent-directories list.
//!
//! [`important_roots`]: compass_core::watch_policy::important_roots
//! [`build_watch_set`]: compass_core::watch_policy::build_watch_set
//! [`WatchEvent`]: compass_core::watch_events::WatchEvent
//!
//! # Deltas from the C++, on purpose
//!
//! * The C++ drains inotify on its own thread behind a mutex; here the crate
//!   calls the callback on its thread and [`DirWatcher::drain`] maps and
//!   extends watches on the caller's thread, so all watch state lives in one
//!   place with no locking.
//! * A `notify` error on the channel degrades like an overflow: either way the
//!   watcher can no longer say what changed, which is what `Degraded` means.
//! * Removing a dynamic watch drops it from the maps directly. The C++ relies
//!   on the kernel's `IN_IGNORED` arriving later to clean up, because its maps
//!   live behind the event thread; with single-threaded state there is nothing
//!   to reconcile.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use compass_core::file_walk::IndexWalk;
use compass_core::watch_events::WatchEvent;
use compass_core::watch_policy::{
    MAX_WATCH_DEPTH, WATCH_BUDGET, build_watch_set, important_roots,
};
use notify::{
    Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
    event::{CreateKind, Flag, ModifyKind},
};

/// A live watch over the important directories.
#[derive(Debug)]
pub struct DirWatcher {
    watcher: RecommendedWatcher,
    /// Raw crate events, in arrival order.
    events: mpsc::Receiver<notify::Result<Event>>,
    /// Every watched directory, with how deep below its root it sits.
    depths: HashMap<PathBuf, usize>,
    /// The watched directories that came from the dynamic list rather than
    /// the breadth-first set.
    dynamic: HashSet<PathBuf>,
    /// The roots the set was built from.
    roots: Vec<PathBuf>,
    /// The scan policy new directories are held to.
    walk: IndexWalk,
    /// Whether placing watches gave up and left directories to the scan.
    budget_exhausted: bool,
}

impl DirWatcher {
    /// Starts watching the important directories under `home`.
    ///
    /// `home_entries` lists the home directory's direct contents and
    /// `xdg_dirs` the config and data homes, the way the watcher computes its
    /// roots; see [`important_roots`].
    ///
    /// [`important_roots`]: compass_core::watch_policy::important_roots
    pub fn new(
        home: Option<&Path>,
        home_entries: &[PathBuf],
        is_directory: impl Fn(&Path) -> bool,
        is_symlink: impl Fn(&Path) -> bool,
        xdg_dirs: &[PathBuf],
        walk: IndexWalk,
    ) -> notify::Result<Self> {
        let (sender, events) = mpsc::channel();
        let watcher = RecommendedWatcher::new(
            move |event| {
                let _ = sender.send(event);
            },
            notify::Config::default(),
        )?;

        let roots = important_roots(
            home,
            home_entries,
            &is_directory,
            &is_symlink,
            xdg_dirs,
        );
        let mut watcher = Self {
            watcher,
            events,
            depths: HashMap::new(),
            dynamic: HashSet::new(),
            roots,
            walk,
            budget_exhausted: false,
        };
        let set = build_watch_set(&watcher.roots, &watcher.walk, WATCH_BUDGET);
        watcher.budget_exhausted = set.budget_exhausted;
        for (dir, depth) in &set.watches {
            watcher.add_watch(dir, *depth);
        }
        tracing::info!(count = watcher.depths.len(), "watching directories");
        Ok(watcher)
    }

    /// The roots the watch set was built from.
    #[must_use]
    pub fn root_directories(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Every directory currently watched.
    #[must_use]
    pub fn watched_directories(&self) -> Vec<PathBuf> {
        self.depths.keys().cloned().collect()
    }

    /// Whether placing watches gave up and left directories to the scan.
    ///
    /// Reaching the budget is not an error: the remaining directories fall
    /// back to the scan cadence, which would have covered them anyway.
    #[must_use]
    pub fn budget_exhausted(&self) -> bool {
        self.budget_exhausted
    }

    /// Maps every queued raw event to watch events, extending watches for
    /// directories created since the last drain.
    ///
    /// Only structural changes prompt: creations, removals, and renames, the
    /// way the C++ mask watches `CREATE | DELETE | MOVED_FROM | MOVED_TO |
    /// DELETE_SELF` and nothing else. A content edit changes no structure —
    /// the periodic scan's cutoff catches it — so data and metadata
    /// modifications stay quiet.
    pub fn drain(&mut self) -> Vec<WatchEvent> {
        let mut out = Vec::new();
        while let Ok(received) = self.events.try_recv() {
            let Ok(event) = received else {
                // The watcher failed; like an overflow, it can no longer say
                // what changed.
                out.push(WatchEvent::Degraded);
                continue;
            };
            self.apply(&event, &mut out);
        }
        out
    }

    /// Replaces the dynamic leaf watches with `dirs`.
    ///
    /// Leaves: no automatic extension below them. Directories that stop being
    /// listed are unwatched; unwatched listed directories are added at the
    /// deepest depth, and adding stops at the first failure.
    pub fn set_dynamic_directories(&mut self, dirs: &[PathBuf]) {
        let desired: HashSet<&PathBuf> = dirs.iter().collect();
        let stale: Vec<PathBuf> = self
            .dynamic
            .iter()
            .filter(|path| !desired.contains(path))
            .cloned()
            .collect();
        for path in &stale {
            let _ = self.watcher.unwatch(path);
            self.depths.remove(path);
            self.dynamic.remove(path);
        }
        for path in dirs {
            if self.depths.contains_key(path) {
                continue;
            }
            if !path.is_dir() {
                continue;
            }
            if !self.add_watch(path, MAX_WATCH_DEPTH) {
                break;
            }
            self.dynamic.insert(path.clone());
        }
    }

    /// Watches `dir` at `depth`, unless the budget ran out.
    ///
    /// Out of watches is not an error: the directory falls back to the scan
    /// cadence, exactly as the C++ treats `ENOSPC` and a full budget.
    fn add_watch(&mut self, dir: &Path, depth: usize) -> bool {
        if self.depths.len() >= WATCH_BUDGET {
            self.note_exhausted("watch budget exhausted, remaining directories fall back to periodic scans");
            return false;
        }
        match self.watcher.watch(dir, RecursiveMode::NonRecursive) {
            Ok(()) => {
                self.depths.insert(dir.to_path_buf(), depth);
                true
            }
            Err(error) => {
                if matches!(error.kind, notify::ErrorKind::MaxFilesWatch) {
                    self.note_exhausted(
                        "out of inotify watches (fs.inotify.max_user_watches), remaining directories fall back to periodic scans",
                    );
                }
                false
            }
        }
    }

    /// Marks the budget exhausted, warning once.
    fn note_exhausted(&mut self, message: &str) {
        if !self.budget_exhausted {
            self.budget_exhausted = true;
            tracing::warn!("{message}");
        }
    }

    /// Maps one raw event, watching a newly created directory first.
    fn apply(&mut self, event: &Event, out: &mut Vec<WatchEvent>) {
        if matches!(event.kind, EventKind::Other) && event.flag() == Some(Flag::Rescan) {
            // Queue overflow: some change was lost, so rescan the roots.
            out.push(WatchEvent::Degraded);
            return;
        }
        if !matches!(
            event.kind,
            EventKind::Create(_)
                | EventKind::Remove(_)
                | EventKind::Modify(ModifyKind::Name(_))
        ) {
            return;
        }
        let mut changed = Vec::new();
        for path in &event.paths {
            // The watched directory itself first: a path that is itself
            // watched resolves to its own watch, the way a wd lookup does —
            // a child watched at a deeper level reports under its own name.
            if self.depths.contains_key(path) {
                if matches!(event.kind, EventKind::Remove(_)) {
                    // The watched directory went away: drop it and everything
                    // beneath it quietly, the way IN_IGNORED cleans the maps
                    // without reporting. Component-wise, so a sibling whose
                    // name merely starts the same way survives.
                    self.depths.retain(|watched, _| !watched.starts_with(path));
                    self.dynamic.retain(|watched| !watched.starts_with(path));
                } else {
                    changed.push(path.clone());
                }
                continue;
            }
            let Some(parent) = path.parent() else {
                continue;
            };
            let Some(depth) = self.depths.get(parent).copied() else {
                continue;
            };
            if matches!(event.kind, EventKind::Create(CreateKind::Folder)) {
                self.maybe_watch_new_directory(parent, depth, path);
            }
            changed.push(parent.to_path_buf());
        }
        changed.sort();
        changed.dedup();
        out.extend(changed.into_iter().map(WatchEvent::DirectoryChanged));
    }

    /// Watches a directory created inside a watched one, at its parent's
    /// depth plus one.
    ///
    /// Best effort: whatever it decides, the parent still changed, so the
    /// caller reports that regardless.
    fn maybe_watch_new_directory(&mut self, parent: &Path, parent_depth: usize, path: &Path) {
        if parent_depth >= MAX_WATCH_DEPTH {
            return;
        }
        let is_symlink = path
            .symlink_metadata()
            .map(|metadata| metadata.is_symlink())
            .unwrap_or(false);
        if !path.is_dir() || !self.walk.should_visit(path, is_symlink, true) {
            return;
        }
        self.add_watch(path, parent_depth + 1);
    }
}
