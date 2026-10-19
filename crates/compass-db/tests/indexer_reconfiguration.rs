//! Current indexing boundaries against a real temporary SQLite database.
//! Source files remain intact; only derived index rows are reconciled.

use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::{Duration, SystemTime};

use compass_db::db_writer::{DbWriter, FileEvent, FileEventType, IndexDatabase, ReconcileError};
use compass_db::file_indexer::FileIndexer;
use compass_db::sqlite_reader::SqliteReader;
use compass_db::sqlite_writer::{SqliteWriter, prepare_file_index_database};

struct Fixture {
    dir: tempfile::TempDir,
    database: PathBuf,
    files: Vec<PathBuf>,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("index.db");
        prepare_file_index_database(&database).unwrap();
        let files: Vec<PathBuf> = [
            "old/report.txt",
            "kept/report.txt",
            "kept/private/secret.txt",
            "kept/private-sibling/public.txt",
            "kept-other/report.txt",
            "literal%_[x]/report.txt",
            "literalX_[x]/report.txt",
        ]
        .into_iter()
        .map(|path| dir.path().join(path))
        .collect();
        for path in &files {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "source content must stay intact").unwrap();
        }
        let mut writer = SqliteWriter::open(&database).unwrap();
        writer.index_files(&files);
        Self {
            dir,
            database,
            files,
        }
    }

    fn rows(&self) -> Vec<(i64, PathBuf)> {
        let db = compass_sqlcipher_sys::open(&self.database, &[]).unwrap();
        let mut query = db
            .prepare("SELECT id, path FROM indexed_file ORDER BY path")
            .unwrap();
        query
            .query_map([], |row| {
                Ok((row.get(0)?, PathBuf::from(row.get::<_, String>(1)?)))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn assert_sources_intact(&self) {
        for path in &self.files {
            assert_eq!(
                std::fs::read_to_string(path).unwrap(),
                "source content must stay intact"
            );
        }
    }

    fn indexer(
        &self,
    ) -> (
        FileIndexer<SqliteWriter, SqliteReader>,
        Arc<DbWriter<SqliteWriter>>,
    ) {
        let database = self.database.clone();
        let writer = Arc::new(DbWriter::new(move || {
            SqliteWriter::open(&database).unwrap()
        }));
        let database = self.database.clone();
        let indexer = FileIndexer::new(
            Arc::clone(&writer),
            move || SqliteReader::open(&database).unwrap(),
            "index.db",
        );
        (indexer, writer)
    }
}

fn drain(writer: &DbWriter<SqliteWriter>) {
    let (sender, receiver) = mpsc::channel();
    writer.submit(move |_| sender.send(()).unwrap(), false);
    receiver.recv_timeout(Duration::from_secs(10)).unwrap();
}

#[test]
fn startup_prunes_old_roots_and_new_exclusions_without_rebuilding_retained_rows() {
    let fixture = Fixture::new();
    let root = fixture.dir.path();
    let retained = root.join("kept/report.txt");
    let retained_id = fixture
        .rows()
        .into_iter()
        .find(|(_, path)| path == &retained)
        .unwrap()
        .0;
    // A new helper has no remembered previous configuration. Its database is
    // the cache from before indexing was disabled and its settings changed.
    let (indexer, writer) = fixture.indexer();
    indexer.set_config(vec![root.join("kept/./")], vec![root.join("kept/private")]);
    assert!(indexer.start());
    indexer.dispatcher().wait_until_idle();
    drain(&writer);
    let rows = fixture.rows();
    assert!(
        rows.iter()
            .any(|(id, path)| *id == retained_id && path == &retained)
    );
    assert!(rows.iter().all(|(_, path)| {
        path.starts_with(root.join("kept")) && !path.starts_with(root.join("kept/private"))
    }));
    assert!(
        rows.iter()
            .any(|(_, path)| path == &root.join("kept/private-sibling/public.txt"))
    );
    fixture.assert_sources_intact();

    // Stop this helper, change its roots again, then reopen the same cache.
    drop(indexer);
    drop(writer);
    let (indexer, writer) = fixture.indexer();
    indexer.set_config(vec![root.join("old")], Vec::new());
    assert!(indexer.start());
    indexer.dispatcher().wait_until_idle();
    drain(&writer);
    assert!(
        fixture
            .rows()
            .iter()
            .all(|(_, path)| path.starts_with(root.join("old")))
    );
    assert!(
        fixture
            .rows()
            .iter()
            .any(|(_, path)| path == &root.join("old/report.txt"))
    );
    fixture.assert_sources_intact();
}

#[test]
fn root_empty_roots_sibling_prefixes_and_wildcards_have_component_boundaries() {
    let fixture = Fixture::new();
    let root = fixture.dir.path();
    let mut writer = SqliteWriter::open(&fixture.database).unwrap();
    let before = fixture.rows();
    // This only reconciles database rows. It never scans the filesystem root.
    writer
        .retain_indexed_files(&[PathBuf::from("/")], &[])
        .unwrap();
    assert_eq!(fixture.rows(), before);
    writer
        .retain_indexed_files(&[root.to_path_buf()], &[root.join("literal%_[x]")])
        .unwrap();
    assert!(
        !fixture
            .rows()
            .iter()
            .any(|(_, path)| path.starts_with(root.join("literal%_[x]")))
    );
    assert!(
        fixture
            .rows()
            .iter()
            .any(|(_, path)| path.starts_with(root.join("literalX_[x]")))
    );
    writer
        .retain_indexed_files(&[root.join("kept")], &[root.join("kept/private")])
        .unwrap();
    let kept = fixture.rows();
    assert_eq!(kept.len(), 2);
    writer
        .retain_indexed_files(&[root.join("kept")], &[root.join("kept/private")])
        .unwrap();
    assert_eq!(
        fixture.rows(),
        kept,
        "unchanged boundaries preserve rows and ids"
    );
    writer
        .retain_indexed_files(&[PathBuf::from("/")], &[PathBuf::from("/")])
        .unwrap();
    assert!(
        fixture.rows().is_empty(),
        "excluding / excludes every cached path"
    );
    writer.index_files(&fixture.files);
    writer.retain_indexed_files(&[], &[]).unwrap();
    assert!(
        fixture.rows().is_empty(),
        "an explicit empty root list retains no cached rows"
    );
    fixture.assert_sources_intact();
}

#[cfg(unix)]
#[test]
fn retention_uses_lexical_paths_without_following_symlinks() {
    let fixture = Fixture::new();
    let root = fixture.dir.path();
    std::os::unix::fs::symlink(root.join("old"), root.join("alias")).unwrap();
    let alias = root.join("alias/report.txt");
    let mut writer = SqliteWriter::open(&fixture.database).unwrap();
    writer.index_files(std::slice::from_ref(&alias));
    writer
        .retain_indexed_files(&[root.join("alias")], &[])
        .unwrap();
    assert_eq!(
        fixture
            .rows()
            .iter()
            .map(|(_, path)| path)
            .collect::<Vec<_>>(),
        [&alias]
    );
    fixture.assert_sources_intact();
}

#[test]
fn late_scanner_writes_cannot_reinsert_removed_roots_or_excluded_subtrees() {
    let fixture = Fixture::new();
    let root = fixture.dir.path();
    let database = fixture.database.clone();
    let writer = DbWriter::new(move || SqliteWriter::open(&database).unwrap());
    // A batch already queued by the old scanner is drained before retention.
    writer.index_files(fixture.files.clone());
    writer
        .retain_indexed_files(vec![root.join("kept")], vec![root.join("kept/private")])
        .unwrap();
    // A scheduler may have pulled these from its pending queue before it was
    // cancelled. The boundary is checked on execution, not on submission.
    writer.index_files(fixture.files.clone());
    writer.index_events(
        fixture
            .files
            .iter()
            .map(|path| FileEvent {
                event_type: FileEventType::Modify,
                path: path.clone(),
                event_time: SystemTime::now(),
                is_directory: false,
                size_bytes: Some(1),
            })
            .collect(),
    );
    drain(&writer);
    assert_eq!(fixture.rows().len(), 2);
    assert!(fixture.rows().iter().all(|(_, path)| {
        path.starts_with(root.join("kept")) && !path.starts_with(root.join("kept/private"))
    }));
    fixture.assert_sources_intact();
}

#[test]
fn live_changes_prune_rows_before_new_scans_and_preserve_sources() {
    let fixture = Fixture::new();
    let root = fixture.dir.path();
    let (indexer, writer) = fixture.indexer();
    indexer.set_config(vec![root.join("old"), root.join("kept")], Vec::new());
    assert!(indexer.start());
    assert!(indexer.apply_config(vec![root.join("kept")], vec![root.join("kept/private")]));
    indexer.dispatcher().wait_until_idle();
    drain(&writer);
    assert!(fixture.rows().iter().all(|(_, path)| {
        path.starts_with(root.join("kept")) && !path.starts_with(root.join("kept/private"))
    }));
    fixture.assert_sources_intact();
}

#[test]
fn reconciliation_failure_refuses_startup_scans() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("missing-schema.db");
    let mut db = SqliteWriter::open(&database).unwrap();
    assert!(matches!(
        db.retain_indexed_files(&[], &[]),
        Err(ReconcileError::Database(_))
    ));
    drop(db);
    let write_path = database.clone();
    let writer = Arc::new(DbWriter::new(move || {
        SqliteWriter::open(&write_path).unwrap()
    }));
    let indexer = FileIndexer::new(
        writer,
        move || SqliteReader::open(&database).unwrap(),
        "index.db",
    );
    indexer.set_config(vec![dir.path().to_path_buf()], Vec::new());
    assert!(!indexer.start());
    assert!(indexer.dispatcher().scans().is_empty());
}

#[test]
fn failed_reconciliation_also_pauses_late_scanner_writes() {
    let fixture = Fixture::new();
    let root = fixture.dir.path();
    let database = fixture.database.clone();
    let writer = DbWriter::new(move || SqliteWriter::open(&database).unwrap());
    writer
        .retain_indexed_files(vec![root.to_path_buf()], Vec::new())
        .unwrap();
    let db = compass_sqlcipher_sys::open(&fixture.database, &[]).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_cleanup BEFORE DELETE ON indexed_file BEGIN SELECT RAISE(ABORT, 'fixture failure'); END").unwrap();
    let before = fixture.rows();
    assert!(
        writer
            .retain_indexed_files(vec![root.join("kept")], Vec::new())
            .is_err()
    );
    let late = root.join("kept/late.txt");
    std::fs::write(&late, "source").unwrap();
    writer.index_files(vec![late]);
    drain(&writer);
    assert_eq!(
        fixture.rows(),
        before,
        "failed cleanup is atomic and blocks stale writes"
    );
    fixture.assert_sources_intact();
}
