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
}
