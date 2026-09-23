//! The file-indexer service binary: JSON-RPC over stdio, like
//! `vicinae-file-indexer`.
//!
//! Everything lives in [`vicinae::indexer_service`] so it can be tested; this
//! is startup and wiring only: legacy databases out, schema ensured, the
//! indexer and its watch backend built, then frames until stdin closes.

use std::io::BufReader;
use std::sync::{Arc, Mutex};

use compass_core::file_walk::IndexWalk;
use compass_db::db_writer::DbWriter;
use compass_db::file_indexer::FileIndexer;
use compass_db::query_pool::QueryPool;
use compass_db::query_reader::IndexReader;
use compass_db::sqlite_reader::SqliteReader;
use compass_db::sqlite_writer::{SqliteWriter, prepare_file_index_database};
use vicinae::indexer_service::{
    DATABASE_FILE_NAME, QUERY_WORKER_COUNT, StdoutSink, database_path, remove_legacy_db_files,
    watch_backend,
};
use vicinae::indexer_watch::RecentDirs;

fn main() -> std::process::ExitCode {
    let Some(cache_home) = compass_core::xdg_dirs::cache_home() else {
        eprintln!("vicinae-file-indexer: no cache home, giving up");
        return std::process::ExitCode::FAILURE;
    };
    let Some(data_home) = compass_core::xdg_dirs::data_home() else {
        eprintln!("vicinae-file-indexer: no data home, giving up");
        return std::process::ExitCode::FAILURE;
    };
    let home = compass_core::xdg_dirs::home_dir();
    let config_home = compass_core::xdg_dirs::config_home();

    remove_legacy_db_files(&data_home);
    let db_path = database_path(&cache_home);
    if let Err(error) = prepare_file_index_database(&db_path) {
        eprintln!("vicinae-file-indexer: cannot prepare {db_path:?}: {error}");
        return std::process::ExitCode::FAILURE;
    }

    // Open once eagerly: every factory below reopens the same path, so a
    // failure here fails fast instead of panicking a worker later.
    if SqliteWriter::open(&db_path).is_err() || SqliteReader::open(&db_path).is_err() {
        eprintln!("vicinae-file-indexer: cannot open {db_path:?}");
        return std::process::ExitCode::FAILURE;
    }
    let writer = Arc::new(DbWriter::new({
        let db_path = db_path.clone();
        move || SqliteWriter::open(&db_path).expect("verified at startup")
    }));
    // Verified above: these factories only fail if the database vanishes
    // mid-run, which is a crash anyway.
    let indexer = FileIndexer::new(
        writer,
        {
            let db_path = db_path.clone();
            move || SqliteReader::open(&db_path).expect("verified at startup")
        },
        DATABASE_FILE_NAME,
    );
    let pool = QueryPool::new(QUERY_WORKER_COUNT, {
        let db_path = db_path.clone();
        move || SqliteReader::open(&db_path).expect("verified at startup")
    });

    // One reader held for the watcher's dynamic refreshes; the lock only
    // ever blocks a minute-timer thread.
    let recent_reader = Mutex::new(SqliteReader::open(&db_path).expect("verified at startup"));
    let recent: RecentDirs = Arc::new(move |limit| {
        recent_reader
            .lock()
            .map(|reader| reader.recent_directories(limit))
            .unwrap_or_default()
    });

    let mut xdg_dirs = Vec::with_capacity(2);
    xdg_dirs.extend(config_home);
    xdg_dirs.extend(Some(data_home));
    let backend = watch_backend(
        &indexer,
        home.clone(),
        xdg_dirs,
        IndexWalk::new(home.as_deref()),
        recent,
    );
    let (starter, stopper) = backend.controls();
    // The backend outlives the closures through this handle; dropping it at
    // shutdown stops the watch.
    let _backend = backend;
    indexer.set_watcher_controls(starter, stopper);

    let sink = Arc::new(StdoutSink::default());
    let service = vicinae::indexer_service::IndexerService::new(indexer, pool, sink);
    service.run_stdio(&mut BufReader::new(std::io::stdin()));

    std::process::ExitCode::SUCCESS
}
