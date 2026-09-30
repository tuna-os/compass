//! Hot reload: notice when scripts change on disk, and say which.
//!
//! Two halves. [`ScriptWatcher`] turns filesystem events under the search
//! paths into a debounced "something changed" signal, using `notify`.
//! [`ScriptSet`] turns a fresh [`Scan`] into what actually changed — added,
//! changed, removed — by comparing manifests and sources, so a host rebuilds
//! only those instances and an editor's save-via-rename dance counts as one
//! change, not three.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use compass_extension_api::ExtensionId;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::discovery::{DiscoveredScript, Scan};

/// How long a burst of events is allowed to settle before it is reported.
pub const DEBOUNCE: Duration = Duration::from_millis(150);

/// Watches script search paths recursively.
#[derive(Debug)]
pub struct ScriptWatcher {
    _watcher: RecommendedWatcher,
    events: mpsc::UnboundedReceiver<()>,
    watched: Vec<PathBuf>,
}

impl ScriptWatcher {
    /// Starts watching every path in `roots` that exists. A root that does not
    /// exist yet is skipped; a host that wants the user's directory watched
    /// from the start creates it first.
    ///
    /// # Errors
    ///
    /// When the platform watcher cannot be created.
    pub fn new(roots: &[PathBuf]) -> notify::Result<Self> {
        let (sender, events) = mpsc::unbounded_channel();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                // Errors (including queue overflow) are reported as a change too:
                // either way the only safe answer is to rescan.
                if event.as_ref().map_or(true, |e| !e.kind.is_access()) {
                    let _ = sender.send(());
                }
            })?;
        let mut watched = Vec::new();
        for root in roots.iter().filter(|root| root.is_dir()) {
            match watcher.watch(root, RecursiveMode::Recursive) {
                Ok(()) => watched.push(root.clone()),
                Err(error) => {
                    tracing::warn!(root = %root.display(), %error, "cannot watch scripts")
                }
            }
        }
        Ok(Self {
            _watcher: watcher,
            events,
            watched,
        })
    }

    /// The roots actually being watched.
    #[must_use]
    pub fn watched(&self) -> &[PathBuf] {
        &self.watched
    }

    /// Waits for a change, then for [`DEBOUNCE`] of quiet. Returns `false`
    /// when the watcher has stopped.
    pub async fn changed(&mut self) -> bool {
        if self.events.recv().await.is_none() {
            return false;
        }
        loop {
            match tokio::time::timeout(DEBOUNCE, self.events.recv()).await {
                Ok(Some(())) => {}
                Ok(None) | Err(_) => return true,
            }
        }
    }
}

/// What a reload changed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Reload {
    /// Scripts that were not loaded before.
    pub added: Vec<ExtensionId>,
    /// Scripts whose manifest or source changed.
    pub changed: Vec<ExtensionId>,
    /// Scripts that are gone.
    pub removed: Vec<ExtensionId>,
}

impl Reload {
    /// Whether nothing changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.changed.is_empty() && self.removed.is_empty()
    }
}

/// The scripts currently known, keyed by id.
#[derive(Debug, Default)]
pub struct ScriptSet {
    scripts: BTreeMap<ExtensionId, DiscoveredScript>,
}

impl ScriptSet {
    /// An empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the set with what `scan` found and reports the difference.
    pub fn apply(&mut self, scan: Scan) -> Reload {
        let mut reload = Reload::default();
        let mut next: BTreeMap<ExtensionId, DiscoveredScript> = scan
            .scripts
            .into_iter()
            .map(|script| (script.manifest.id.clone(), script))
            .collect();
        for (id, script) in &next {
            match self.scripts.get(id) {
                None => reload.added.push(id.clone()),
                Some(previous) if previous != script => reload.changed.push(id.clone()),
                Some(_) => {}
            }
        }
        reload.removed = self
            .scripts
            .keys()
            .filter(|id| !next.contains_key(*id))
            .cloned()
            .collect();
        std::mem::swap(&mut self.scripts, &mut next);
        reload
    }

    /// A script by id.
    #[must_use]
    pub fn get(&self, id: &ExtensionId) -> Option<&DiscoveredScript> {
        self.scripts.get(id)
    }

    /// Every script, by id.
    pub fn iter(&self) -> impl Iterator<Item = &DiscoveredScript> {
        self.scripts.values()
    }

    /// How many scripts are loaded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.scripts.len()
    }

    /// Whether no script is loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.scripts.is_empty()
    }
}
