//! When a filesystem change turns into a scan, and which scans run.
//!
//! Ported from `ScanDispatcher`.

use std::path::{Path, PathBuf};

use compass_core::scan_dispatch::{
    DEBOUNCE_MAX_DELAY_SECS, DEBOUNCE_QUIET_SECS, Debouncer, Interrupted, ScanQueue, ScanType,
    WORKER_COUNT,
};

const T0: u64 = 1_000;

fn root() -> PathBuf {
    PathBuf::from("/root")
}

#[test]
fn the_shipped_timings_are_what_they_are() {
    // Every other test in this file reads these through the constants, so a
    // changed constant moves the expectation with it and nothing notices.
    // Three controls were silent until this test existed. The numbers are the
    // C++'s, and they are a judgement about how long a user will tolerate a
    // stale index against how much churn the indexer will chase.
    assert_eq!(DEBOUNCE_QUIET_SECS, 5);
    assert_eq!(DEBOUNCE_MAX_DELAY_SECS, 30);
    assert_eq!(WORKER_COUNT, 2);
}

#[test]
fn a_change_waits_out_a_quiet_period() {
    let mut debouncer = Debouncer::new();
    debouncer.record(&root(), ScanType::Incremental, T0);

    assert_eq!(debouncer.next_deadline(), Some(T0 + DEBOUNCE_QUIET_SECS));
}

#[test]
fn a_scan_is_not_due_before_its_deadline() {
    let mut debouncer = Debouncer::new();
    debouncer.record(&root(), ScanType::Incremental, T0);

    assert!(debouncer.take_due(T0 + DEBOUNCE_QUIET_SECS - 1).is_empty());
    assert_eq!(debouncer.pending().len(), 1);
}

#[test]
fn a_scan_due_exactly_now_runs_now() {
    // `pending.deadline > now` decides what is *kept*, so equality is due.
    let mut debouncer = Debouncer::new();
    debouncer.record(&root(), ScanType::Incremental, T0);

    assert_eq!(debouncer.take_due(T0 + DEBOUNCE_QUIET_SECS).len(), 1);
}

#[test]
fn a_second_change_pushes_the_deadline_out() {
    // Saving a file is several events; a build is thousands. Scanning on each
    // would keep the indexer re-reading a tree that is still changing.
    let mut debouncer = Debouncer::new();
    debouncer.record(&root(), ScanType::Incremental, T0);
    debouncer.record(&root(), ScanType::Incremental, T0 + 1);

    assert_eq!(
        debouncer.next_deadline(),
        Some(T0 + 1 + DEBOUNCE_QUIET_SECS)
    );
}

#[test]
fn a_directory_that_never_goes_quiet_is_scanned_anyway() {
    // The ceiling. A log directory, a build tree, a folder mid-download: each
    // is written to every few seconds, and without the `min` its scan is
    // pushed back for as long as the writing lasts, which is to say never run.
    let mut debouncer = Debouncer::new();
    debouncer.record(&root(), ScanType::Incremental, T0);

    for second in 1..=60 {
        debouncer.record(&root(), ScanType::Incremental, T0 + second);
    }

    assert_eq!(
        debouncer.next_deadline(),
        Some(T0 + DEBOUNCE_MAX_DELAY_SECS)
    );
}

#[test]
fn the_ceiling_is_measured_from_the_first_event_not_the_last() {
    let mut debouncer = Debouncer::new();
    debouncer.record(&root(), ScanType::Incremental, T0);
    debouncer.record(&root(), ScanType::Incremental, T0 + DEBOUNCE_MAX_DELAY_SECS);

    assert_eq!(
        debouncer.next_deadline(),
        Some(T0 + DEBOUNCE_MAX_DELAY_SECS)
    );
}

#[test]
fn a_quiet_period_shorter_than_the_ceiling_still_wins() {
    let mut debouncer = Debouncer::new();
    debouncer.record(&root(), ScanType::Incremental, T0);
    debouncer.record(&root(), ScanType::Incremental, T0 + 2);

    // 2 + 5 is under 30, so the quiet period is what decides.
    assert_eq!(
        debouncer.next_deadline(),
        Some(T0 + 2 + DEBOUNCE_QUIET_SECS)
    );
}

#[test]
fn a_full_and_an_incremental_scan_of_one_path_wait_separately() {
    // They do different work and one is not a substitute for the other.
    let mut debouncer = Debouncer::new();
    debouncer.record(&root(), ScanType::Incremental, T0);
    debouncer.record(&root(), ScanType::Full, T0 + 3);

    assert_eq!(debouncer.pending().len(), 2);
}

#[test]
fn different_paths_wait_separately() {
    let mut debouncer = Debouncer::new();
    debouncer.record(Path::new("/a"), ScanType::Incremental, T0);
    debouncer.record(Path::new("/b"), ScanType::Incremental, T0);

    assert_eq!(debouncer.pending().len(), 2);
}

#[test]
fn taking_what_is_due_leaves_what_is_not() {
    let mut debouncer = Debouncer::new();
    debouncer.record(Path::new("/soon"), ScanType::Incremental, T0);
    debouncer.record(Path::new("/later"), ScanType::Incremental, T0 + 10);

    let due = debouncer.take_due(T0 + DEBOUNCE_QUIET_SECS);

    assert_eq!(due.len(), 1);
    assert_eq!(due[0].path, PathBuf::from("/soon"));
    assert_eq!(debouncer.pending().len(), 1);
}

#[test]
fn a_scan_that_could_not_start_comes_back_rather_than_being_dropped() {
    // The events that asked for it are real, and the scan already running may
    // have passed the files they touched before they were touched.
    let mut debouncer = Debouncer::new();
    debouncer.record(&root(), ScanType::Incremental, T0);
    let due = debouncer.take_due(T0 + DEBOUNCE_QUIET_SECS);
    assert!(debouncer.pending().is_empty());

    debouncer.rearm(&due[0], T0 + 100);

    assert_eq!(
        debouncer.next_deadline(),
        Some(T0 + 100 + DEBOUNCE_QUIET_SECS)
    );
}

#[test]
fn a_rearmed_scan_gets_a_fresh_ceiling_too() {
    let mut debouncer = Debouncer::new();
    debouncer.record(&root(), ScanType::Incremental, T0);
    let due = debouncer.take_due(T0 + DEBOUNCE_QUIET_SECS);

    debouncer.rearm(&due[0], T0 + 100);
    for second in 1..=60 {
        debouncer.record(&root(), ScanType::Incremental, T0 + 100 + second);
    }

    assert_eq!(
        debouncer.next_deadline(),
        Some(T0 + 100 + DEBOUNCE_MAX_DELAY_SECS)
    );
}

#[test]
fn clearing_forgets_everything_waiting() {
    let mut debouncer = Debouncer::new();
    debouncer.record(&root(), ScanType::Incremental, T0);

    debouncer.clear();

    assert_eq!(debouncer.next_deadline(), None);
}

#[test]
fn a_scan_is_accepted_with_an_id() {
    let mut queue = ScanQueue::new();

    assert_eq!(queue.enqueue(&root(), ScanType::Incremental), Some(0));
}

#[test]
fn ids_are_not_reused() {
    let mut queue = ScanQueue::new();
    assert_eq!(
        queue.enqueue(Path::new("/a"), ScanType::Incremental),
        Some(0)
    );
    queue.finish(0);

    assert_eq!(
        queue.enqueue(Path::new("/b"), ScanType::Incremental),
        Some(1)
    );
}

#[test]
fn a_second_scan_of_the_same_path_is_refused() {
    let mut queue = ScanQueue::new();
    queue.enqueue(&root(), ScanType::Incremental);

    assert_eq!(queue.enqueue(&root(), ScanType::Incremental), None);
}

#[test]
fn the_duplicate_check_ignores_the_kind_of_scan() {
    // Two scans of one directory read the same files twice and race each
    // other's writes, whatever kinds they are.
    let mut queue = ScanQueue::new();
    queue.enqueue(&root(), ScanType::Incremental);

    assert_eq!(queue.enqueue(&root(), ScanType::Full), None);
}

#[test]
fn a_path_can_be_scanned_again_once_the_first_scan_is_done() {
    let mut queue = ScanQueue::new();
    let first = queue
        .enqueue(&root(), ScanType::Incremental)
        .expect("accepted");
    queue.finish(first);

    assert!(queue.enqueue(&root(), ScanType::Incremental).is_some());
}

#[test]
fn a_running_scan_still_blocks_its_path() {
    let mut queue = ScanQueue::new();
    queue.enqueue(&root(), ScanType::Incremental);
    queue.start_next();

    assert_eq!(queue.enqueue(&root(), ScanType::Full), None);
}

#[test]
fn scans_start_in_the_order_they_were_accepted() {
    let mut queue = ScanQueue::new();
    queue.enqueue(Path::new("/first"), ScanType::Incremental);
    queue.enqueue(Path::new("/second"), ScanType::Incremental);

    let started = queue.start_next().expect("a worker took it").path.clone();

    assert_eq!(started, PathBuf::from("/first"));
}

#[test]
fn no_more_scans_run_at_once_than_there_are_workers() {
    let mut queue = ScanQueue::new();
    for index in 0..WORKER_COUNT + 1 {
        queue.enqueue(
            &PathBuf::from(format!("/path{index}")),
            ScanType::Incremental,
        );
    }

    for _ in 0..WORKER_COUNT {
        assert!(queue.start_next().is_some());
    }

    assert!(queue.start_next().is_none());
}

#[test]
fn a_finished_scan_frees_its_worker() {
    let mut queue = ScanQueue::new();
    for index in 0..WORKER_COUNT + 1 {
        queue.enqueue(
            &PathBuf::from(format!("/path{index}")),
            ScanType::Incremental,
        );
    }
    for _ in 0..WORKER_COUNT {
        queue.start_next();
    }

    queue.finish(0);

    assert!(queue.start_next().is_some());
}

#[test]
fn interrupting_a_queued_scan_drops_it() {
    let mut queue = ScanQueue::new();
    let id = queue
        .enqueue(&root(), ScanType::Incremental)
        .expect("accepted");

    assert_eq!(queue.interrupt(id), Interrupted::Removed);
    assert!(queue.is_idle());
}

#[test]
fn interrupting_a_running_scan_leaves_it_in_place() {
    // A scanner that has been told to stop has not stopped yet, and reporting
    // it gone would let a second scan of the same path start beside it.
    let mut queue = ScanQueue::new();
    let id = queue
        .enqueue(&root(), ScanType::Incremental)
        .expect("accepted");
    queue.start_next();

    assert_eq!(queue.interrupt(id), Interrupted::Running);
    assert_eq!(queue.enqueue(&root(), ScanType::Incremental), None);
}

#[test]
fn interrupting_an_unknown_id_says_so() {
    let mut queue = ScanQueue::new();

    assert_eq!(queue.interrupt(42), Interrupted::NoSuchScan);
}

#[test]
fn interrupting_everything_drops_the_queue_and_names_the_running() {
    let mut queue = ScanQueue::new();
    let running = queue
        .enqueue(Path::new("/running"), ScanType::Incremental)
        .expect("accepted");
    queue.start_next();
    queue.enqueue(Path::new("/queued"), ScanType::Incremental);

    let told_to_stop = queue.interrupt_all();

    assert_eq!(told_to_stop, [running]);
    assert_eq!(queue.scans().len(), 1);
}

#[test]
fn an_empty_queue_is_idle() {
    assert!(ScanQueue::new().is_idle());
}
