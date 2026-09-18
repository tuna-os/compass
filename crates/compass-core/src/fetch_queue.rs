//! The image fetcher's scheduler: which pending requests are in flight, and
//! which wait.
//!
//! A port of `NetworkFetcher` (`src/server/src/services/image-fetcher/`),
//! minus the transport. The C++ class is a Qt worker on its own thread with a
//! `QNetworkDiskCache` behind it; what is portable — and what actually decides
//! what a person sees while a list of remote icons loads — is the queue in
//! front of that. This module is that queue as a state machine: it is told
//! what happened and answers with what to start next, so the transport can be
//! anything.
//!
//! # The queue is last-in-first-out, and that is not a mistake
//!
//! `startRequests` takes from the *back* of the deque that `fetch` pushes to.
//! For the case this exists to serve — a list whose rows each want an image,
//! scrolled quickly — that is the right end: the newest requests are the rows
//! now on screen, and the oldest are rows the person scrolled past. Serving
//! them first-in-first-out would spend all six slots on images nobody is
//! looking at any more.
//!
//! The cost is that under sustained load an early request can wait
//! indefinitely. That is real, and it is the C++'s behaviour, so it is pinned
//! by a test rather than quietly fixed here.

use std::collections::HashMap;

/// How many requests the C++ allows in flight at once.
pub const DEFAULT_CONCURRENCY: usize = 6;

/// The largest the on-disk image cache is allowed to grow, from
/// `Omnicast::IMAGE_DISK_CACHE_MAX_SIZE`.
pub const IMAGE_DISK_CACHE_MAX_SIZE: u64 = 5 * 1024 * 1024 * 1024;

/// A handle on one request, unique for as long as it is live.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FetchId(u64);

impl FetchId {
    /// The handle's value, for a caller that has to key something else by it.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A request the caller should now send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartRequest {
    /// The request this starts.
    pub id: FetchId,
    /// What to fetch.
    pub url: String,
}

/// One queued, not-yet-started request.
#[derive(Debug, Clone)]
struct Queued {
    /// The request's handle.
    id: FetchId,
    /// What it wants fetched.
    url: String,
}

/// The fetcher's queue: what is waiting, what is in flight.
#[derive(Debug)]
pub struct FetchQueue {
    /// How many may be in flight at once.
    concurrency: usize,
    /// Waiting requests, oldest first; taken from the back.
    queue: Vec<Queued>,
    /// In-flight requests, by handle.
    in_flight: HashMap<FetchId, String>,
    /// The next handle to hand out.
    next_id: u64,
}

impl Default for FetchQueue {
    fn default() -> Self {
        Self::with_concurrency(DEFAULT_CONCURRENCY)
    }
}

impl FetchQueue {
    /// A queue that allows [`DEFAULT_CONCURRENCY`] requests in flight.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A queue that allows `concurrency` requests in flight.
    ///
    /// A `concurrency` of 0 would never start anything, so it is raised to 1.
    #[must_use]
    pub fn with_concurrency(concurrency: usize) -> Self {
        Self {
            concurrency: concurrency.max(1),
            queue: Vec::new(),
            in_flight: HashMap::new(),
            next_id: 0,
        }
    }

    /// How many requests are in flight.
    #[must_use]
    pub fn in_flight_count(&self) -> usize {
        self.in_flight.len()
    }

    /// How many requests are waiting for a slot.
    #[must_use]
    pub fn queued_count(&self) -> usize {
        self.queue.len()
    }

    /// Whether `id` names a request that is in flight.
    #[must_use]
    pub fn is_in_flight(&self, id: FetchId) -> bool {
        self.in_flight.contains_key(&id)
    }

    /// Ask for `url`, returning the request's handle and whatever should now
    /// be started — which may include this request, or not, or an older one.
    pub fn fetch(&mut self, url: impl Into<String>) -> (FetchId, Vec<StartRequest>) {
        let id = FetchId(self.next_id);
        self.next_id += 1;
        self.queue.push(Queued {
            id,
            url: url.into(),
        });
        (id, self.start_requests())
    }

    /// Report that `id` finished, returning what should now be started.
    ///
    /// A handle that is not in flight — already finished, or aborted — frees
    /// no slot, which is what keeps a duplicate completion from letting a
    /// seventh request through.
    pub fn finished(&mut self, id: FetchId) -> Vec<StartRequest> {
        self.in_flight.remove(&id);
        self.start_requests()
    }

    /// Abort `id`, returning whether the transport should be told to stop.
    ///
    /// # The freed slot is not refilled
    ///
    /// The C++ `aborted` handler erases the reply and emits `abortRequested`;
    /// it does not call `startRequests`. So aborting an in-flight request
    /// leaves a slot empty until some *other* request finishes. This port does
    /// the same, and `aborting_an_in_flight_request_does_not_start_the_next_one`
    /// pins it: calling `start_requests` here would be the better scheduler and
    /// the wrong port, and a person comparing the two builds under a fast
    /// scroll would see Compass issue requests Vicinae does not.
    pub fn abort(&mut self, id: FetchId) -> bool {
        let was_queued = self.queue.iter().position(|queued| queued.id == id);
        if let Some(index) = was_queued {
            self.queue.remove(index);
        }
        self.in_flight.remove(&id);
        true
    }

    /// Fill every free slot from the back of the queue.
    fn start_requests(&mut self) -> Vec<StartRequest> {
        let mut started = Vec::new();
        while !self.queue.is_empty() && self.in_flight.len() < self.concurrency {
            let queued = self.queue.pop().expect("the queue is not empty");
            self.in_flight.insert(queued.id, queued.url.clone());
            started.push(StartRequest {
                id: queued.id,
                url: queued.url,
            });
        }
        started
    }
}
