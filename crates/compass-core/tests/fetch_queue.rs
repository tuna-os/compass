//! What the image fetcher's queue starts, and when.
//!
//! Read off `NetworkFetcher` (`src/server/src/services/image-fetcher/`).

use compass_core::fetch_queue::{DEFAULT_CONCURRENCY, FetchQueue, IMAGE_DISK_CACHE_MAX_SIZE};

#[test]
fn six_requests_run_at_once() {
    assert_eq!(DEFAULT_CONCURRENCY, 6);
}

#[test]
fn the_disk_cache_is_capped_at_five_gigabytes() {
    assert_eq!(IMAGE_DISK_CACHE_MAX_SIZE, 5 * 1024 * 1024 * 1024);
}

#[test]
fn the_first_request_starts_immediately() {
    let mut queue = FetchQueue::new();
    let (id, started) = queue.fetch("https://example.test/a.png");
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].id, id);
    assert_eq!(started[0].url, "https://example.test/a.png");
    assert!(queue.is_in_flight(id));
}

#[test]
fn requests_past_the_concurrency_limit_wait() {
    let mut queue = FetchQueue::new();
    for index in 0..DEFAULT_CONCURRENCY {
        let (_, started) = queue.fetch(format!("https://example.test/{index}.png"));
        assert_eq!(started.len(), 1, "request {index} should start at once");
    }
    assert_eq!(queue.in_flight_count(), DEFAULT_CONCURRENCY);

    let (_, started) = queue.fetch("https://example.test/seventh.png");
    assert!(started.is_empty(), "the seventh request has no slot");
    assert_eq!(queue.queued_count(), 1);
    assert_eq!(queue.in_flight_count(), DEFAULT_CONCURRENCY);
}

#[test]
fn a_finished_request_frees_its_slot_for_the_next() {
    let mut queue = FetchQueue::with_concurrency(1);
    let (first, _) = queue.fetch("https://example.test/first.png");
    let (second, started) = queue.fetch("https://example.test/second.png");
    assert!(started.is_empty());

    let started = queue.finished(first);
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].id, second);
    assert!(!queue.is_in_flight(first));
    assert!(queue.is_in_flight(second));
}

#[test]
fn the_queue_is_served_newest_first() {
    // startRequests takes from the back of the deque fetch pushes to. For a
    // list being scrolled this is the right end: the newest requests are the
    // rows now on screen.
    let mut queue = FetchQueue::with_concurrency(1);
    let (_first, _) = queue.fetch("https://example.test/first.png");
    let (_second, _) = queue.fetch("https://example.test/second.png");
    let (third, _) = queue.fetch("https://example.test/third.png");

    let started = queue.finished(_first);
    assert_eq!(started.len(), 1);
    assert_eq!(
        started[0].id, third,
        "the most recently asked-for image should be served first, not the oldest"
    );
}

#[test]
fn an_early_request_can_starve_under_sustained_load() {
    // The cost of serving newest-first, pinned rather than fixed: as long as
    // new requests keep arriving, the first one never runs.
    let mut queue = FetchQueue::with_concurrency(1);
    let (blocker, _) = queue.fetch("https://example.test/blocker.png");
    let (starved, _) = queue.fetch("https://example.test/starved.png");

    let mut running = blocker;
    for round in 0..20 {
        queue.fetch(format!("https://example.test/new-{round}.png"));
        let started = queue.finished(running);
        assert_eq!(started.len(), 1);
        assert_ne!(started[0].id, starved, "round {round} should not reach it");
        running = started[0].id;
    }
    assert_eq!(queue.queued_count(), 1, "one request is still waiting");
}

#[test]
fn aborting_a_queued_request_removes_it_from_the_queue() {
    let mut queue = FetchQueue::with_concurrency(1);
    let (running, _) = queue.fetch("https://example.test/running.png");
    let (waiting, _) = queue.fetch("https://example.test/waiting.png");
    assert_eq!(queue.queued_count(), 1);

    assert!(queue.abort(waiting));
    assert_eq!(queue.queued_count(), 0);

    let started = queue.finished(running);
    assert!(started.is_empty(), "an aborted request must never start");
}

#[test]
fn aborting_an_in_flight_request_stops_tracking_it() {
    let mut queue = FetchQueue::with_concurrency(2);
    let (first, _) = queue.fetch("https://example.test/first.png");
    assert!(queue.is_in_flight(first));

    assert!(queue.abort(first));
    assert!(!queue.is_in_flight(first));
}

#[test]
fn aborting_an_in_flight_request_does_not_start_the_next_one() {
    // The C++ abort handler erases the reply and emits abortRequested without
    // calling startRequests, so the freed slot stays empty until some other
    // request finishes. Reproduced deliberately; see the module docs.
    let mut queue = FetchQueue::with_concurrency(1);
    let (running, _) = queue.fetch("https://example.test/running.png");
    let (waiting, _) = queue.fetch("https://example.test/waiting.png");

    queue.abort(running);
    assert_eq!(queue.in_flight_count(), 0);
    assert_eq!(
        queue.queued_count(),
        1,
        "the waiting request is still waiting"
    );
    assert!(!queue.is_in_flight(waiting));
}

#[test]
fn the_slot_an_abort_freed_is_taken_by_the_next_completion() {
    // The other half of the rule above: the queue is not wedged, it just waits
    // for a different event to pump it.
    let mut queue = FetchQueue::with_concurrency(2);
    let (first, _) = queue.fetch("https://example.test/first.png");
    let (second, _) = queue.fetch("https://example.test/second.png");
    let (third, _) = queue.fetch("https://example.test/third.png");
    assert_eq!(queue.queued_count(), 1);

    queue.abort(first);
    assert_eq!(queue.queued_count(), 1, "the abort alone pumps nothing");

    let started = queue.finished(second);
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].id, third);
}

#[test]
fn finishing_a_request_twice_does_not_open_an_extra_slot() {
    let mut queue = FetchQueue::with_concurrency(1);
    let (first, _) = queue.fetch("https://example.test/first.png");
    queue.fetch("https://example.test/second.png");
    queue.fetch("https://example.test/third.png");

    let started = queue.finished(first);
    assert_eq!(started.len(), 1);

    let again = queue.finished(first);
    assert!(
        again.is_empty(),
        "a duplicate completion must not exceed the limit"
    );
    assert_eq!(queue.in_flight_count(), 1);
}

#[test]
fn every_request_gets_its_own_handle() {
    let mut queue = FetchQueue::new();
    let (first, _) = queue.fetch("https://example.test/same.png");
    let (second, _) = queue.fetch("https://example.test/same.png");
    assert_ne!(
        first, second,
        "two requests for one URL are two requests, not one"
    );
}

#[test]
fn a_concurrency_of_zero_would_wedge_the_queue_so_it_is_raised() {
    let mut queue = FetchQueue::with_concurrency(0);
    let (id, started) = queue.fetch("https://example.test/a.png");
    assert_eq!(
        started.len(),
        1,
        "something has to start, or nothing ever will"
    );
    assert!(queue.is_in_flight(id));
}

#[test]
fn a_full_queue_drains_completely_when_everything_finishes() {
    let mut queue = FetchQueue::with_concurrency(2);
    let mut live: Vec<_> = (0..5)
        .map(|index| queue.fetch(format!("https://example.test/{index}.png")).0)
        .collect();
    live.retain(|id| queue.is_in_flight(*id));

    let mut finished = 0;
    while let Some(id) = live.pop() {
        finished += 1;
        for start in queue.finished(id) {
            live.push(start.id);
        }
    }
    assert_eq!(finished, 5, "every request should eventually run");
    assert_eq!(queue.queued_count(), 0);
    assert_eq!(queue.in_flight_count(), 0);
}
