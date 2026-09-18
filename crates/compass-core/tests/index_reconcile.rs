//! What the indexer does when its settings change, and at startup.
//!
//! Ported from `file-indexer.cpp`.

use std::path::PathBuf;

use compass_core::index_reconcile::{
    Entrypoint, PendingFullScans, ScanRecord, ScanStatus, StartupAction, build_reconcile_plan,
    covered_by_remaining_root, startup_plan,
};

fn owned(items: &[&str]) -> Vec<PathBuf> {
    items.iter().map(PathBuf::from).collect()
}

fn paths(items: &[PathBuf]) -> Vec<String> {
    items
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

fn succeeded(id: i64) -> Option<ScanRecord> {
    Some(ScanRecord {
        id,
        status: ScanStatus::Succeeded,
    })
}

fn interrupted(id: i64) -> Option<ScanRecord> {
    Some(ScanRecord {
        id,
        status: ScanStatus::Interrupted,
    })
}

#[test]
fn nothing_changing_asks_for_nothing() {
    let plan = build_reconcile_plan(&owned(&["/a"]), &[], &owned(&["/a"]), &[]);

    assert!(plan.delete_subtrees.is_empty());
    assert!(plan.scan_roots.is_empty());
}

#[test]
fn a_removed_root_is_deleted_from_the_index() {
    // The user stopped indexing it. An index that keeps answering with its
    // files is ignoring them.
    let plan = build_reconcile_plan(&owned(&["/a", "/b"]), &[], &owned(&["/a"]), &[]);

    assert_eq!(paths(&plan.delete_subtrees), ["/b"]);
}

#[test]
fn an_added_root_is_scanned() {
    let plan = build_reconcile_plan(&owned(&["/a"]), &[], &owned(&["/a", "/b"]), &[]);

    assert_eq!(paths(&plan.scan_roots), ["/b"]);
}

#[test]
fn a_root_already_inside_a_root_that_stays_is_not_scanned_again() {
    let plan = build_reconcile_plan(&owned(&["/a"]), &[], &owned(&["/a", "/a/deep"]), &[]);

    assert!(plan.scan_roots.is_empty());
}

#[test]
fn a_root_inside_one_that_is_going_away_is_scanned() {
    // The old root covers it, but its rows are about to be deleted. Treating
    // that as coverage leaves the new root unscanned and its files gone.
    let plan = build_reconcile_plan(&owned(&["/a"]), &[], &owned(&["/a/deep"]), &[]);

    assert_eq!(paths(&plan.delete_subtrees), ["/a"]);
    assert_eq!(paths(&plan.scan_roots), ["/a/deep"]);
}

#[test]
fn coverage_by_a_remaining_root_is_the_whole_question() {
    let old = owned(&["/a"]);

    assert!(covered_by_remaining_root(
        &PathBuf::from("/a/deep"),
        &old,
        &owned(&["/a"])
    ));
    assert!(!covered_by_remaining_root(
        &PathBuf::from("/a/deep"),
        &old,
        &owned(&["/other"])
    ));
}

#[test]
fn a_new_exclusion_is_deleted() {
    let plan = build_reconcile_plan(
        &owned(&["/a"]),
        &[],
        &owned(&["/a"]),
        &owned(&["/a/secret"]),
    );

    assert_eq!(paths(&plan.delete_subtrees), ["/a/secret"]);
}

#[test]
fn an_exclusion_that_was_already_excluded_is_not_deleted_again() {
    // There is nothing of it in the index to remove.
    let plan = build_reconcile_plan(
        &owned(&["/a"]),
        &owned(&["/a/secret"]),
        &owned(&["/a"]),
        &owned(&["/a/secret"]),
    );

    assert!(plan.delete_subtrees.is_empty());
}

#[test]
fn a_lifted_exclusion_inside_a_root_is_scanned() {
    // Those files were skipped while the exclusion stood, and nothing else
    // would ever go back for them.
    let plan = build_reconcile_plan(&owned(&["/a"]), &owned(&["/a/was"]), &owned(&["/a"]), &[]);

    assert_eq!(paths(&plan.scan_roots), ["/a/was"]);
}

#[test]
fn a_lifted_exclusion_outside_every_root_is_not_scanned() {
    let plan = build_reconcile_plan(
        &owned(&["/a"]),
        &owned(&["/elsewhere"]),
        &owned(&["/a"]),
        &[],
    );

    assert!(plan.scan_roots.is_empty());
}

#[test]
fn a_root_inside_a_new_exclusion_is_not_scanned() {
    // Otherwise the plan would re-index exactly what it just deleted.
    let plan = build_reconcile_plan(&[], &[], &owned(&["/a/secret"]), &owned(&["/a"]));

    assert!(plan.scan_roots.is_empty());
}

#[test]
fn overlapping_deletes_collapse() {
    let plan = build_reconcile_plan(&owned(&["/a", "/a/deep"]), &[], &[], &[]);

    assert_eq!(paths(&plan.delete_subtrees), ["/a"]);
}

#[test]
fn overlapping_scans_collapse() {
    let plan = build_reconcile_plan(&[], &[], &owned(&["/a", "/a/deep"]), &[]);

    assert_eq!(paths(&plan.scan_roots), ["/a"]);
}

#[test]
fn swapping_one_root_for_another_does_both() {
    let plan = build_reconcile_plan(&owned(&["/old"]), &[], &owned(&["/new"]), &[]);

    assert_eq!(paths(&plan.delete_subtrees), ["/old"]);
    assert_eq!(paths(&plan.scan_roots), ["/new"]);
}

#[test]
fn a_pending_root_is_remembered() {
    let mut pending = PendingFullScans::new();
    pending.mark_pending(&owned(&["/a"]));

    assert!(!pending.is_empty());
    assert_eq!(paths(pending.roots()), ["/a"]);
}

#[test]
fn overlapping_pending_roots_collapse() {
    let mut pending = PendingFullScans::new();
    pending.mark_pending(&owned(&["/a/code"]));
    pending.mark_pending(&owned(&["/a"]));

    assert_eq!(paths(pending.roots()), ["/a"]);
}

#[test]
fn a_finished_scan_clears_what_it_covered() {
    // A scan of `/a` has covered the pending `/a/code`; leaving it behind
    // scans it a second time for nothing.
    let mut pending = PendingFullScans::new();
    pending.mark_pending(&owned(&["/a/code", "/b"]));

    pending.mark_succeeded(&PathBuf::from("/a"));

    assert_eq!(paths(pending.roots()), ["/b"]);
}

#[test]
fn a_finished_scan_does_not_clear_a_root_above_it() {
    let mut pending = PendingFullScans::new();
    pending.mark_pending(&owned(&["/a"]));

    pending.mark_succeeded(&PathBuf::from("/a/code"));

    assert_eq!(paths(pending.roots()), ["/a"]);
}

#[test]
fn a_pending_root_the_settings_dropped_is_forgotten() {
    // Finishing its scan would index files the user has just said to leave
    // alone.
    let mut pending = PendingFullScans::new();
    pending.mark_pending(&owned(&["/a", "/b"]));

    pending.prune(&owned(&["/a"]), &[]);

    assert_eq!(paths(pending.roots()), ["/a"]);
}

#[test]
fn a_pending_root_inside_a_new_exclusion_is_forgotten() {
    let mut pending = PendingFullScans::new();
    pending.mark_pending(&owned(&["/a/secret"]));

    pending.prune(&owned(&["/a"]), &owned(&["/a/secret"]));

    assert!(pending.is_empty());
}

#[test]
fn the_wanted_pending_roots_are_reported_without_changing_what_is_pending() {
    let mut pending = PendingFullScans::new();
    pending.mark_pending(&owned(&["/a", "/b"]));

    let wanted = pending.roots_for(&owned(&["/a"]), &[]);

    assert_eq!(paths(&wanted), ["/a"]);
    // The filter answers a question; it does not decide the outcome. A config
    // change that is later undone must not have lost `/b`.
    assert_eq!(paths(pending.roots()), ["/a", "/b"]);
}

#[test]
fn a_first_start_scans_everything_fully() {
    let plan = startup_plan(&[Entrypoint {
        path: PathBuf::from("/a"),
        last_full: None,
        last_incremental: None,
    }]);

    assert_eq!(plan.actions, [StartupAction::FullScan(PathBuf::from("/a"))]);
}

#[test]
fn a_first_start_does_not_watch_until_the_scan_is_done() {
    // A full scan walks the tree it is also being told about, and the events
    // would be for files the scan is reading anyway.
    let plan = startup_plan(&[Entrypoint {
        path: PathBuf::from("/a"),
        last_full: None,
        last_incremental: None,
    }]);

    assert!(!plan.start_watcher);
}

#[test]
fn a_started_index_watches_from_the_beginning() {
    let plan = startup_plan(&[Entrypoint {
        path: PathBuf::from("/a"),
        last_full: succeeded(1),
        last_incremental: None,
    }]);

    assert!(plan.start_watcher);
    assert_eq!(
        plan.actions,
        [StartupAction::IncrementalScan(PathBuf::from("/a"))]
    );
}

#[test]
fn an_unfinished_full_scan_is_redone_from_scratch() {
    // The index holds part of that tree and nothing records how much. An
    // incremental pass would compare against the cut-off the interrupted scan
    // wrote and skip everything it never reached.
    let plan = startup_plan(&[Entrypoint {
        path: PathBuf::from("/a"),
        last_full: interrupted(7),
        last_incremental: None,
    }]);

    assert_eq!(
        plan.actions,
        [
            StartupAction::MarkInterrupted(7),
            StartupAction::FullScan(PathBuf::from("/a"))
        ]
    );
    assert!(!plan.start_watcher);
}

#[test]
fn a_full_scan_that_failed_is_treated_the_same_as_one_that_was_stopped() {
    let plan = startup_plan(&[Entrypoint {
        path: PathBuf::from("/a"),
        last_full: Some(ScanRecord {
            id: 9,
            status: ScanStatus::Failed,
        }),
        last_incremental: None,
    }]);

    assert_eq!(plan.actions[0], StartupAction::MarkInterrupted(9));
}

#[test]
fn a_full_scan_still_recorded_as_running_is_redone() {
    // The process died mid-scan, so the row still says Started. Trusting it
    // would treat a half-built index as complete.
    let plan = startup_plan(&[Entrypoint {
        path: PathBuf::from("/a"),
        last_full: Some(ScanRecord {
            id: 3,
            status: ScanStatus::Started,
        }),
        last_incremental: None,
    }]);

    assert_eq!(plan.actions[0], StartupAction::MarkInterrupted(3));
    assert_eq!(
        plan.actions[1],
        StartupAction::FullScan(PathBuf::from("/a"))
    );
}

#[test]
fn an_unfinished_incremental_scan_is_marked_before_the_next_one() {
    let plan = startup_plan(&[Entrypoint {
        path: PathBuf::from("/a"),
        last_full: succeeded(1),
        last_incremental: interrupted(2),
    }]);

    assert_eq!(
        plan.actions,
        [
            StartupAction::MarkInterrupted(2),
            StartupAction::IncrementalScan(PathBuf::from("/a"))
        ]
    );
}

#[test]
fn a_finished_incremental_scan_is_left_alone() {
    let plan = startup_plan(&[Entrypoint {
        path: PathBuf::from("/a"),
        last_full: succeeded(1),
        last_incremental: succeeded(2),
    }]);

    assert_eq!(
        plan.actions,
        [StartupAction::IncrementalScan(PathBuf::from("/a"))]
    );
}

#[test]
fn one_root_needing_a_full_scan_holds_the_watcher_back_for_all_of_them() {
    let plan = startup_plan(&[
        Entrypoint {
            path: PathBuf::from("/a"),
            last_full: succeeded(1),
            last_incremental: None,
        },
        Entrypoint {
            path: PathBuf::from("/b"),
            last_full: None,
            last_incremental: None,
        },
    ]);

    assert!(!plan.start_watcher);
}

#[test]
fn no_roots_at_all_still_starts_the_watcher() {
    let plan = startup_plan(&[]);

    assert!(plan.actions.is_empty());
    assert!(plan.start_watcher);
}
