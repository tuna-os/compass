//! Watching the directories the catalog is scanned from, so what is installed
//! or removed while the engine runs shows up without a restart.
//!
//! `AppService` watches the application directories with a
//! `QFileSystemWatcher` and rescans 500 ms after the last change
//! (`m_rescanDebounce`), reinstalling the watches after every scan in case the
//! set of directories that exist has changed. That is ported here over
//! `notify`, with the same debounce: every event restarts the wait, so a
//! package manager writing forty desktop files costs one scan.
//!
//! The extension directories are watched the same way, as
//! `ExtensionRegistry` does with its own 100 ms debounce: see
//! [`watch_extensions`].
//!
//! One declared difference: the C++ watch is on each directory itself, so a
//! file added to a subdirectory (`applications/kde4/`) is only noticed with
//! the next change at the top. The watch here is recursive, since the scan it
//! triggers is recursive too.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::{RwLock, mpsc};

use crate::serve::EngineState;

/// `AppService`'s `m_rescanDebounce`.
pub const APPLICATIONS_DEBOUNCE: Duration = Duration::from_millis(500);

/// `ExtensionRegistry`'s `m_rescanDebounce`.
pub const EXTENSIONS_DEBOUNCE: Duration = Duration::from_millis(100);

/// The file whose arrival makes a directory an extension.
const MANIFEST: &str = "package.json";

/// A debounced watch over a set of directories.
#[derive(Debug)]
pub struct DirWatch {
    watcher: RecommendedWatcher,
    events: mpsc::UnboundedReceiver<()>,
    watched: Vec<PathBuf>,
    mode: RecursiveMode,
    debounce: Duration,
}

impl DirWatch {
    /// A watch that reports a change once `debounce` has passed with no
    /// further event. `relevant` says whether an event on a path is worth a
    /// rescan; an error from the platform watcher (a full queue) always is,
    /// since something may have been missed.
    ///
    /// # Errors
    ///
    /// When the platform watcher cannot be created.
    pub fn new(
        mode: RecursiveMode,
        debounce: Duration,
        relevant: impl Fn(&Path) -> bool + Send + 'static,
    ) -> notify::Result<Self> {
        let (sender, events) = mpsc::unbounded_channel();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let wanted = match &event {
                Ok(event) => !event.kind.is_access() && event.paths.iter().any(|p| relevant(p)),
                Err(_) => true,
            };
            if wanted {
                let _ = sender.send(());
            }
        })?;
        Ok(Self {
            watcher,
            events,
            watched: Vec::new(),
            mode,
            debounce,
        })
    }

    /// Watches the directories of `dirs` that exist, and stops watching the
    /// rest. Nothing is touched when that set has not changed, as
    /// `reinstallWatches` insists: on some platforms a re-added watch floods
    /// events.
    pub fn watch(&mut self, dirs: &[PathBuf]) {
        let mut desired: Vec<PathBuf> = dirs.iter().filter(|dir| dir.is_dir()).cloned().collect();
        desired.sort();
        desired.dedup();
        if desired == self.watched {
            return;
        }
        for dir in std::mem::take(&mut self.watched) {
            let _ = self.watcher.unwatch(&dir);
        }
        for dir in desired {
            match self.watcher.watch(&dir, self.mode) {
                Ok(()) => self.watched.push(dir),
                Err(error) => tracing::warn!(dir = %dir.display(), %error, "cannot watch"),
            }
        }
    }

    /// The directories being watched, sorted.
    #[must_use]
    pub fn watched(&self) -> &[PathBuf] {
        &self.watched
    }

    /// Waits for a relevant change, then for the debounce to pass quietly.
    /// `false` once the watcher has stopped.
    pub async fn changed(&mut self) -> bool {
        if self.events.recv().await.is_none() {
            return false;
        }
        loop {
            match tokio::time::timeout(self.debounce, self.events.recv()).await {
                Ok(Some(())) => {}
                Ok(None) | Err(_) => return true,
            }
        }
    }
}

/// Rescans the applications whenever their directories change, for as long
/// as the engine runs.
pub async fn watch_applications(state: Arc<RwLock<EngineState>>) {
    watch_applications_with(state, APPLICATIONS_DEBOUNCE).await;
}

/// [`watch_applications`] with its debounce given, for tests.
pub async fn watch_applications_with(state: Arc<RwLock<EngineState>>, debounce: Duration) {
    let mut watch = match DirWatch::new(RecursiveMode::Recursive, debounce, |_| true) {
        Ok(watch) => watch,
        Err(error) => {
            tracing::warn!(%error, "cannot watch the application directories; new applications need a restart");
            return;
        }
    };
    let dirs = state.read().await.app_index().application_dirs().to_vec();
    watch.watch(&dirs);
    tracing::info!(watched = ?watch.watched(), "watching the application directories");
    while watch.changed().await {
        let scan = state.read().await.app_index().application_scan();
        let Ok(fresh) = tokio::task::spawn_blocking(move || scan.build()).await else {
            continue;
        };
        state.write().await.replace_applications(fresh);
        watch.watch(&dirs);
    }
}

/// What the extension watch covers: each extension directory, and each
/// extension in it, one level deep.
fn extension_watch_dirs(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = roots.to_vec();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        dirs.extend(
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
                .map(|entry| entry.path()),
        );
    }
    dirs
}

/// Whether an event at `path` can change what the registry scans: an entry
/// appearing in or leaving an extension directory, or a manifest written.
/// A build writing its bundle is neither, until its `package.json` lands.
fn extension_event_matters(roots: &[PathBuf], path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == MANIFEST)
        || path
            .parent()
            .is_some_and(|parent| roots.iter().any(|root| root == parent))
}

/// Rescans the installed extensions whenever their directories change, as
/// `ExtensionRegistry` does with its `QFileSystemWatcher` and 100 ms
/// debounce, so an extension a developer builds into place joins the root
/// search without a restart.
///
/// The C++ watches each extension directory itself. That sees an extension
/// appear, and not the manifest a build writes into it afterwards; `vicinae
/// develop` creates the directory before it builds, so the rescan comes too
/// early and the extension waits for the next change. Each extension's own
/// directory is watched here too, for its `package.json` only.
pub async fn watch_extensions(state: Arc<RwLock<EngineState>>) {
    watch_extensions_with(state, EXTENSIONS_DEBOUNCE).await;
}

/// [`watch_extensions`] with its debounce given, for tests.
pub async fn watch_extensions_with(state: Arc<RwLock<EngineState>>, debounce: Duration) {
    let roots = state.read().await.app_index().extension_dirs().to_vec();
    if roots.is_empty() {
        return;
    }
    let relevant = {
        let roots = roots.clone();
        move |path: &Path| extension_event_matters(&roots, path)
    };
    let mut watch = match DirWatch::new(RecursiveMode::NonRecursive, debounce, relevant) {
        Ok(watch) => watch,
        Err(error) => {
            tracing::warn!(%error, "cannot watch the extension directories; new extensions need a restart");
            return;
        }
    };
    watch.watch(&extension_watch_dirs(&roots));
    tracing::info!(roots = ?roots, "watching the extension directories");
    while watch.changed().await {
        state.write().await.rescan_extensions();
        watch.watch(&extension_watch_dirs(&roots));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_burst_of_changes_is_one_change_and_an_ignored_path_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let mut watch = DirWatch::new(
            RecursiveMode::Recursive,
            Duration::from_millis(100),
            |path| path.extension().is_some_and(|e| e == "desktop"),
        )
        .unwrap();
        watch.watch(&[dir.path().to_path_buf(), dir.path().join("missing")]);
        assert_eq!(
            watch.watched(),
            [dir.path().to_path_buf()],
            "only what exists"
        );

        std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(400), watch.changed())
                .await
                .is_err(),
            "not a desktop file"
        );

        for n in 0..5 {
            std::fs::write(dir.path().join(format!("a{n}.desktop")), "x").unwrap();
        }
        assert!(
            tokio::time::timeout(Duration::from_secs(5), watch.changed())
                .await
                .unwrap()
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(300), watch.changed())
                .await
                .is_err(),
            "the burst was reported once"
        );
    }

    #[test]
    fn only_an_entry_of_an_extension_directory_or_a_manifest_matters() {
        let roots = [PathBuf::from("/data/vicinae/extensions")];
        let matters = |path: &str| extension_event_matters(&roots, Path::new(path));
        assert!(matters("/data/vicinae/extensions/clock"));
        assert!(matters("/data/vicinae/extensions/clock/package.json"));
        assert!(!matters("/data/vicinae/extensions/clock/list.js"));
        assert!(!matters("/data/vicinae/extensions/clock/assets/icon.png"));

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("clock")).unwrap();
        std::fs::create_dir_all(dir.path().join(".staging-clock")).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "").unwrap();
        let mut dirs = extension_watch_dirs(&[dir.path().to_path_buf()]);
        dirs.sort();
        assert_eq!(
            dirs,
            [dir.path().to_path_buf(), dir.path().join("clock")],
            "an install's staging directory is not an extension"
        );
    }
}
