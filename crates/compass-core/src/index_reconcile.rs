//! What the indexer does when its settings change, and what it does at startup.
//!
//! Ports `buildReconcilePlan`, the pending-full-scan bookkeeping and `start()`
//! from `file-indexer.cpp`. All of it is set arithmetic over scan roots, built
//! on [`crate::scan_roots`].
//!
//! Both halves answer the same question from different directions: **which
//! files should be in the index that are not, and which are in it that should
//! not be.** Getting the first wrong means a search that silently misses
//! files; getting the second wrong means a search that returns files the user
//! asked it to forget, which is the worse of the two.

use std::path::{Path, PathBuf};

use crate::scan_roots::{compact_subtrees, is_covered_by_any, is_same_or_descendant_of};

/// What a settings change asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcilePlan {
    /// Subtrees whose rows must leave the index.
    pub delete_subtrees: Vec<PathBuf>,
    /// Roots that must be scanned.
    pub scan_roots: Vec<PathBuf>,
}

/// Whether a new root's files are already indexed by an old root that stays.
///
/// The word that matters is **stays**. An old root covering this one is no use
/// if that old root is itself on its way out, because its rows are about to be
/// deleted — treating it as coverage would leave the new root unscanned and its
/// files gone from the index.
#[must_use]
pub fn covered_by_remaining_root(
    new_root: &Path,
    old_roots: &[PathBuf],
    new_roots: &[PathBuf],
) -> bool {
    old_roots.iter().any(|old_root| {
        is_same_or_descendant_of(new_root, old_root) && is_covered_by_any(old_root, new_roots)
    })
}

/// What changing the roots and exclusions requires.
///
/// Four rules, each answering a different way the settings can move:
///
/// 1. An old root the new roots no longer cover is **deleted**. The user
///    stopped indexing it, and an index that keeps answering with its files is
///    ignoring them.
/// 2. A new root not already covered by a *remaining* old root is **scanned**.
/// 3. A new exclusion is **deleted**, unless it was already excluded — in which
///    case there is nothing of it in the index to remove.
/// 4. An exclusion that has been lifted is **scanned**, but only if it falls
///    inside a root: those files were skipped while it stood, and nothing else
///    would ever go back for them.
///
/// Then every scan root inside a new exclusion is dropped, which is what keeps
/// rule 2 from re-indexing something rule 3 just deleted, and both lists are
/// compacted so no subtree is scanned or deleted twice.
///
/// The C++ has a fourth guard, skipping an old exclusion that is still
/// excluded before rule 4 considers it. It is not ported: no mutation could
/// make it fail, because the drop that follows removes exactly the same paths.
/// A guard whose whole effect is undone by a later line reads like it is
/// carrying weight, and it is not.
#[must_use]
pub fn build_reconcile_plan(
    old_roots: &[PathBuf],
    old_exclusions: &[PathBuf],
    new_roots: &[PathBuf],
    new_exclusions: &[PathBuf],
) -> ReconcilePlan {
    let mut plan = ReconcilePlan::default();

    for old_root in old_roots {
        if !is_covered_by_any(old_root, new_roots) {
            plan.delete_subtrees.push(old_root.clone());
        }
    }

    for new_root in new_roots {
        if !covered_by_remaining_root(new_root, old_roots, new_roots) {
            plan.scan_roots.push(new_root.clone());
        }
    }

    for new_exclusion in new_exclusions {
        if !is_covered_by_any(new_exclusion, old_exclusions) {
            plan.delete_subtrees.push(new_exclusion.clone());
        }
    }

    for old_exclusion in old_exclusions {
        if is_covered_by_any(old_exclusion, new_roots) {
            plan.scan_roots.push(old_exclusion.clone());
        }
    }

    plan.scan_roots
        .retain(|path| !is_covered_by_any(path, new_exclusions));
    plan.delete_subtrees = compact_subtrees(std::mem::take(&mut plan.delete_subtrees));
    plan.scan_roots = compact_subtrees(std::mem::take(&mut plan.scan_roots));

    plan
}

/// The full scans that have been asked for and have not finished.
///
/// A full scan of a large tree takes minutes and the launcher can be closed
/// inside one. Without this, a settings change followed by a restart leaves a
/// root that was never scanned and never will be — the config already lists it,
/// so the startup path sees nothing to do.
#[derive(Debug, Default)]
pub struct PendingFullScans {
    roots: Vec<PathBuf>,
}

impl PendingFullScans {
    /// Nothing pending.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The roots still waiting, in the order they compact to.
    #[must_use]
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Whether any full scan is outstanding.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// Records that these roots need a full scan.
    ///
    /// Compacted on the way in, so asking for `~` while `~/code` is already
    /// pending leaves one entry rather than two overlapping ones.
    pub fn mark_pending(&mut self, roots: &[PathBuf]) {
        self.roots.extend(roots.iter().cloned());
        self.roots = compact_subtrees(std::mem::take(&mut self.roots));
    }

    /// Records that a full scan of `root` finished.
    ///
    /// Everything **beneath** `root` is cleared too, not only an exact match: a
    /// scan of `~` has covered the pending `~/code`, and leaving it behind
    /// would scan it a second time for nothing.
    pub fn mark_succeeded(&mut self, root: &Path) {
        self.roots
            .retain(|pending| !is_same_or_descendant_of(pending, root));
    }

    /// Forgets pending roots the settings no longer ask for.
    ///
    /// A root outside the configured set, or inside an exclusion, is no longer
    /// wanted — finishing its scan would index files the user has just said to
    /// leave alone.
    pub fn prune(&mut self, roots: &[PathBuf], exclusions: &[PathBuf]) {
        self.roots.retain(|pending| {
            is_covered_by_any(pending, roots) && !is_covered_by_any(pending, exclusions)
        });
    }

    /// The pending roots that the given settings still want, compacted.
    #[must_use]
    pub fn roots_for(&self, roots: &[PathBuf], exclusions: &[PathBuf]) -> Vec<PathBuf> {
        let wanted: Vec<PathBuf> = self
            .roots
            .iter()
            .filter(|pending| {
                is_covered_by_any(pending, roots) && !is_covered_by_any(pending, exclusions)
            })
            .cloned()
            .collect();

        compact_subtrees(wanted)
    }
}

/// How a recorded scan ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanStatus {
    /// Queued and not started.
    Pending,
    /// Running when the record was last written.
    Started,
    /// Stopped on request.
    Interrupted,
    /// Stopped by an error.
    Failed,
    /// Finished.
    Succeeded,
}

/// A scan the database remembers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanRecord {
    /// Its row id.
    pub id: i64,
    /// How it ended, or how far it got.
    pub status: ScanStatus,
}

/// One configured root and what the database remembers about it.
#[derive(Debug, Clone)]
pub struct Entrypoint {
    /// The configured path.
    pub path: PathBuf,
    /// The last full scan of it.
    pub last_full: Option<ScanRecord>,
    /// The last incremental scan of it.
    pub last_incremental: Option<ScanRecord>,
}

/// One thing startup does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupAction {
    /// Write an `Interrupted` error onto a scan that never finished.
    MarkInterrupted(i64),
    /// Read this root from scratch.
    FullScan(PathBuf),
    /// Read what changed under this root.
    IncrementalScan(PathBuf),
}

/// What startup does, given what the database remembers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupPlan {
    /// In order.
    pub actions: Vec<StartupAction>,
    /// Whether to start watching the filesystem straight away.
    pub start_watcher: bool,
}

/// Works out the startup plan.
///
/// Three cases per root, and the middle one is the reason the other two are not
/// enough:
///
/// * **Never scanned** — a full scan, because there is no index to be
///   incremental against.
/// * **Last full scan did not succeed** — the index holds *part* of that tree,
///   and nothing records how much. An incremental scan would compare against a
///   cut-off the interrupted scan wrote and skip everything it never reached,
///   so the files it missed would stay missing. Full scan, and the old record
///   is marked interrupted so it stops being read as a cut-off.
/// * **Last full scan succeeded** — an incremental scan, with any unfinished
///   incremental scan marked interrupted first for the same reason.
///
/// The watcher starts **only if no full scan was needed**. A full scan walks the
/// tree it is also being told about, and the events it would generate are for
/// files the scan is reading anyway.
#[must_use]
pub fn startup_plan(entrypoints: &[Entrypoint]) -> StartupPlan {
    let mut actions = Vec::new();
    let mut needs_full_scan = false;

    for entrypoint in entrypoints {
        match entrypoint.last_full {
            None => {
                actions.push(StartupAction::FullScan(entrypoint.path.clone()));
                needs_full_scan = true;
            }
            Some(scan) if scan.status != ScanStatus::Succeeded => {
                actions.push(StartupAction::MarkInterrupted(scan.id));
                actions.push(StartupAction::FullScan(entrypoint.path.clone()));
                needs_full_scan = true;
            }
            Some(_) => {
                if let Some(incremental) = entrypoint.last_incremental
                    && incremental.status != ScanStatus::Succeeded
                {
                    actions.push(StartupAction::MarkInterrupted(incremental.id));
                }
                actions.push(StartupAction::IncrementalScan(entrypoint.path.clone()));
            }
        }
    }

    StartupPlan {
        actions,
        start_watcher: !needs_full_scan,
    }
}
