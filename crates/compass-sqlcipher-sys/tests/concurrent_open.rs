//! Connections opened at the same moment all open.
//!
//! `Database::open` switches the file to WAL and registers the tokenizer, and
//! both take a lock. Without a busy timeout, a connection that arrives while
//! another holds it fails at once with "database is locked" — which is how the
//! file indexer's query workers, each opening a reader at startup behind an
//! `expect`, could panic on a race. Found by the file-index quality suite
//! (`compass-db/tests/query_quality.rs`) running its cases in parallel.

use std::sync::{Arc, Barrier};

use compass_sqlcipher_sys::Database;

#[test]
fn many_connections_opened_at_once_all_open() {
    const THREADS: usize = 16;
    for round in 0..8 {
        let dir = tempfile::tempdir().unwrap();
        let path = Arc::new(dir.path().join("race.db"));
        let barrier = Arc::new(Barrier::new(THREADS));
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let (path, barrier) = (Arc::clone(&path), Arc::clone(&barrier));
                std::thread::spawn(move || {
                    barrier.wait();
                    Database::open(&path, &[]).map(drop)
                })
            })
            .collect();
        for handle in handles {
            if let Err(error) = handle.join().unwrap() {
                panic!("round {round}: a concurrent open failed: {error}");
            }
        }
    }
}
