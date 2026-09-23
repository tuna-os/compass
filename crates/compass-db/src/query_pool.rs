//! A pool of query workers, one engine each.
//!
//! Ports `QueryPool`: jobs queue up, each worker thread runs them through
//! its own [`FileIndexerQueryEngine`], and results come back on the
//! [`QueryJob::on_result`] callback. Past [`MAX_PENDING_JOBS`] queued jobs
//! the stalest job drops, completed empty on the spot — a fast typist's
//! intermediate queries never pile up behind each other.
//!
//! The engines arrive ready-made, one factory call per worker: production
//! opens a [`crate::sqlite_reader::SqliteReader`] per thread, the way the
//! C++ pool gives each worker its own engine, and tests hand out scripted
//! readers.

use std::collections::VecDeque;
use std::sync::{
    Arc, Condvar, Mutex, PoisonError,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};

use crate::query_engine::{IndexerFileResult, SearchOptions};
use crate::query_reader::{FileIndexerQueryEngine, IndexReader};

/// How many jobs may wait before the stalest one drops.
pub const MAX_PENDING_JOBS: usize = 8;

/// One query and where its results go.
pub struct QueryJob {
    /// The text to rank.
    pub text: String,
    /// How many hits to keep.
    pub limit: usize,
    /// What to narrow by.
    pub options: SearchOptions,
    /// Receives the ranking, on a worker thread. Empty when the job went
    /// stale waiting.
    pub on_result: Box<dyn FnOnce(Vec<IndexerFileResult>) + Send>,
}

struct Shared {
    state: Mutex<VecDeque<QueryJob>>,
    update: Condvar,
    active: AtomicBool,
}

/// A pool of query workers, one engine each.
///
/// `new` spawns the threads; dropping shuts them down, draining what is
/// queued first.
pub struct QueryPool {
    workers: Vec<JoinHandle<()>>,
    shared: Arc<Shared>,
}

impl QueryPool {
    /// Starts `worker_count` workers, each querying an engine from
    /// `make_reader`. With zero workers jobs queue until they go stale, as
    /// in the C++.
    pub fn new<R: IndexReader>(worker_count: usize, make_reader: impl Fn() -> R) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(VecDeque::new()),
            update: Condvar::new(),
            active: AtomicBool::new(true),
        });
        let mut workers = Vec::new();
        for _ in 0..worker_count {
            let worker_shared = Arc::clone(&shared);
            let engine = FileIndexerQueryEngine::new(make_reader());
            workers.push(thread::spawn(move || {
                loop {
                    let job = {
                        let mut queue = worker_shared
                            .state
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner);
                        while queue.is_empty() && worker_shared.active.load(Ordering::SeqCst) {
                            queue = worker_shared
                                .update
                                .wait(queue)
                                .unwrap_or_else(PoisonError::into_inner);
                        }
                        if queue.is_empty() {
                            break;
                        }
                        queue.pop_front().expect("queue checked non-empty")
                    };
                    let results = engine.query(&job.text, job.limit, &job.options);
                    (job.on_result)(results);
                }
            }));
        }
        Self { workers, shared }
    }

    /// Queues a query. Past [`MAX_PENDING_JOBS`] waiting jobs the stalest one
    /// completes empty first, outside the lock — the callback is user code.
    pub fn submit(&self, job: QueryJob) {
        let stale = {
            let mut queue = self
                .shared
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let stale = if queue.len() >= MAX_PENDING_JOBS {
                queue.pop_front()
            } else {
                None
            };
            queue.push_back(job);
            self.shared.update.notify_one();
            stale
        };
        if let Some(stale) = stale {
            (stale.on_result)(Vec::new());
        }
    }
}

impl Drop for QueryPool {
    fn drop(&mut self) {
        self.shared.active.store(false, Ordering::SeqCst);
        self.shared.update.notify_all();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::Duration;

    use crate::query_engine::{IndexedFileCategory, SearchCandidate};
    use crate::query_policy::SpellfixSuggestion;

    /// A reader answering one fixed path per query string.
    struct FakeReader {
        hits: HashMap<String, PathBuf>,
    }

    impl IndexReader for FakeReader {
        fn is_open(&self) -> bool {
            true
        }

        fn search_candidates(
            &self,
            query: &str,
            _limit: usize,
            _options: &SearchOptions,
        ) -> Vec<crate::query_engine::SearchCandidate> {
            self.hits
                .get(query)
                .map(|path| SearchCandidate {
                    path: path.clone(),
                    category: IndexedFileCategory::Other,
                    mime_type: None,
                })
                .into_iter()
                .collect()
        }

        fn search_skeleton_candidates(
            &self,
            _query: &str,
            _limit: usize,
            _options: &SearchOptions,
        ) -> Vec<SearchCandidate> {
            Vec::new()
        }

        fn spellfix_suggestions(
            &self,
            _word: &str,
            _top: i32,
            _prefix: bool,
        ) -> Vec<SpellfixSuggestion> {
            Vec::new()
        }
    }

    fn submit(pool: &QueryPool, text: &str) -> mpsc::Receiver<Vec<IndexerFileResult>> {
        let (send, receive) = mpsc::channel();
        pool.submit(QueryJob {
            text: text.to_owned(),
            limit: 10,
            options: SearchOptions::default(),
            on_result: Box::new(move |results| {
                let _ = send.send(results);
            }),
        });
        receive
    }

    #[test]
    fn queued_queries_come_back_ranked() {
        let dir = tempfile::tempdir().expect("tempdir");
        let report = dir.path().join("report.txt");
        std::fs::write(&report, "x").expect("file");
        let pool = QueryPool::new(2, || FakeReader {
            hits: HashMap::from([("\"report\"".to_owned(), report.clone())]),
        });

        let results = submit(&pool, "report")
            .recv_timeout(Duration::from_secs(10))
            .expect("an answer");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, report);
    }

    #[test]
    fn the_stalest_job_completes_empty_past_capacity() {
        let pool = QueryPool::new(0, || FakeReader {
            hits: HashMap::new(),
        });
        let mut receivers = Vec::new();
        for _ in 0..=MAX_PENDING_JOBS {
            receivers.push(submit(&pool, "report"));
        }
        // No worker ever runs: the first job went stale, the rest wait.
        let stale = receivers
            .remove(0)
            .recv_timeout(Duration::from_secs(10))
            .expect("the stale answer");
        assert!(stale.is_empty());
        assert_eq!(receivers.len(), MAX_PENDING_JOBS);
    }

    #[test]
    fn dropping_drains_what_is_queued() {
        let dir = tempfile::tempdir().expect("tempdir");
        let report = dir.path().join("report.txt");
        std::fs::write(&report, "x").expect("file");
        let mut receivers = Vec::new();
        {
            let pool = QueryPool::new(1, || FakeReader {
                hits: HashMap::from([("\"report\"".to_owned(), report.clone())]),
            });
            for _ in 0..3 {
                receivers.push(submit(&pool, "report"));
            }
        }
        for receive in receivers {
            let results = receive
                .recv_timeout(Duration::from_secs(10))
                .expect("an answer");
            assert_eq!(results.len(), 1);
        }
    }
}
