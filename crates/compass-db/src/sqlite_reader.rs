//! The [`IndexReader`] that talks to SQLite.
//!
//! Implements the three read queries `FileIndexerDatabase` serves the query
//! engine — strict candidates, skeleton candidates, vocabulary suggestions —
//! over a [`Connection`]. The SQL mirrors the C++ row for row: same tables,
//! same match strings, same category filter, same skeleton rank order.
//!
//! One reader owns one connection on one thread, the way the C++ query pool
//! gives each worker its own engine: `Connection` is `Send` but not `Sync`, so
//! sharing an index across threads means one reader per thread, not one
//! reader under a lock.
//!
//! The scanner owns the schema; this reader only queries. It never migrates,
//! never writes, and never encrypts: the C++ engine keeps the file index
//! unencrypted.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use compass_core::watch_events::DYNAMIC_WATCH_COUNT;
use compass_sqlcipher_sys::rusqlite::{
    self, Connection, OptionalExtension as _, Row, named_params,
};

use crate::db_writer::{ScanRecord, ScanStatus, ScanType};
use crate::query_engine::{IndexedFileCategory, SearchCandidate, SearchOptions};
use crate::query_policy::VocabularySuggestion;
use crate::query_reader::IndexReader;
use crate::vocabulary::Vocabulary;

/// One file-index database, open for reading.
pub struct SqliteReader {
    db: Connection,
    /// The vocabulary as last loaded, and the `data_version` it was loaded at.
    vocabulary: RefCell<Option<(i64, Vocabulary)>>,
}

impl SqliteReader {
    /// Opens the file index at `path`.
    ///
    /// # Errors
    ///
    /// Returns SQLite's error if the file cannot be opened.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Ok(Self {
            db: compass_sqlcipher_sys::open(path, &[])?,
            vocabulary: RefCell::new(None),
        })
    }

    /// Runs `f` over the typo vocabulary, reloading it only when another
    /// connection has committed since the last load.
    fn with_vocabulary<T>(&self, f: impl FnOnce(&Vocabulary) -> T) -> T {
        let version = self
            .db
            .query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))
            .ok();
        let mut cache = self.vocabulary.borrow_mut();
        let fresh = matches!((&*cache, version), (Some((cached, _)), Some(now)) if *cached == now);
        if !fresh {
            *cache = Some((version.unwrap_or(-1), self.load_vocabulary()));
        }
        let (_, words) = cache.get_or_insert_with(|| (-1, Vocabulary::default()));
        f(words)
    }

    fn load_vocabulary(&self) -> Vocabulary {
        let mut stmt = match self.db.prepare("SELECT word, rank FROM vocabulary") {
            Ok(stmt) => stmt,
            Err(error) => {
                tracing::warn!(error = ?error, "reading the vocabulary");
                return Vocabulary::default();
            }
        };
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?))
        });
        let words = match rows {
            Ok(rows) => rows
                .map_while(Result::ok)
                .filter_map(|(word, rank)| Some((word?, rank)))
                .collect(),
            Err(error) => {
                tracing::warn!(error = ?error, "reading the vocabulary");
                Vec::new()
            }
        };
        Vocabulary::new(words)
    }

    /// Runs a candidate query over one FTS table.
    fn search_in(
        &self,
        table: &str,
        order_by_rank: bool,
        query: &str,
        limit: usize,
        options: &SearchOptions,
    ) -> Vec<SearchCandidate> {
        let sql = candidate_sql(table, order_by_rank);
        let mut stmt = match self.db.prepare_cached(&sql) {
            Ok(stmt) => stmt,
            Err(error) => {
                tracing::warn!(error = ?error, table, "preparing the candidate query");
                return Vec::new();
            }
        };
        let params = named_params! {
            ":search": query,
            ":limit": i64::try_from(limit).unwrap_or(i64::MAX),
            ":category": options.category.map(|category| category as i64),
        };
        let mut rows = match stmt.query(params) {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(error = ?error, table, "binding the candidate query");
                return Vec::new();
            }
        };
        let mut results = Vec::new();
        loop {
            let row = match rows.next() {
                Ok(Some(row)) => row,
                Ok(None) => break,
                Err(error) => {
                    tracing::warn!(error = ?error, table, "reading candidate rows");
                    break;
                }
            };
            match candidate(row) {
                Ok(Some(found)) => results.push(found),
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(error = ?error, table, "reading candidate rows");
                    break;
                }
            }
        }
        results
    }
}

/// One candidate row: the path, its category, and its MIME name. A row with
/// no path is skipped.
fn candidate(row: &Row<'_>) -> rusqlite::Result<Option<SearchCandidate>> {
    let Some(path) = row.get::<_, Option<String>>(0)? else {
        return Ok(None);
    };
    Ok(Some(SearchCandidate {
        path: PathBuf::from(path),
        category: category_from_db(row.get(1)?),
        mime_type: row.get(2)?,
    }))
}

impl IndexReader for SqliteReader {
    /// Always true: a constructed reader holds its connection, and the reader
    /// has no close. It exists so the engine treats every reader uniformly.
    fn is_open(&self) -> bool {
        true
    }

    fn search_candidates(
        &self,
        query: &str,
        limit: usize,
        options: &SearchOptions,
    ) -> Vec<SearchCandidate> {
        self.search_in("path_idx", false, query, limit, options)
    }

    fn search_skeleton_candidates(
        &self,
        query: &str,
        limit: usize,
        options: &SearchOptions,
    ) -> Vec<SearchCandidate> {
        self.search_in("skeleton_idx", true, query, limit, options)
    }

    fn vocabulary_suggestions(
        &self,
        word: &str,
        top: i32,
        prefix: bool,
    ) -> Vec<VocabularySuggestion> {
        let top = usize::try_from(top).unwrap_or(0);
        self.with_vocabulary(|words| crate::vocabulary::suggest(words, word, top, prefix))
    }

    fn list_indexed_directory_files(&self, path: &Path) -> HashSet<PathBuf> {
        let Some(dir_id) = file_id(&self.db, path) else {
            return HashSet::new();
        };
        let mut stmt = match self
            .db
            .prepare_cached("SELECT path FROM indexed_file WHERE parent_id = :parent_id")
        {
            Ok(stmt) => stmt,
            Err(error) => {
                tracing::warn!(error = ?error, "listing the indexed directory");
                return HashSet::new();
            }
        };
        let rows = match stmt.query_map(named_params! { ":parent_id": dir_id }, |row| {
            row.get::<_, Option<String>>(0)
        }) {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(error = ?error, "binding the indexed directory");
                return HashSet::new();
            }
        };
        let mut paths = HashSet::new();
        for row in rows {
            match row {
                Ok(Some(found)) => {
                    paths.insert(PathBuf::from(found));
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(error = ?error, "reading indexed directory rows");
                    break;
                }
            }
        }
        paths
    }

    fn tracks_file(&self, path: &Path) -> bool {
        self.db
            .prepare_cached("SELECT COUNT(*) FROM indexed_file WHERE path = :path")
            .and_then(|mut stmt| {
                stmt.query_row(named_params! { ":path": path.to_string_lossy() }, |row| {
                    row.get::<_, i64>(0)
                })
            })
            .inspect_err(|error| tracing::warn!(error = ?error, "checking the tracked file"))
            .is_ok_and(|count| count != 0)
    }

    fn last_successful_scan(&self, path: &Path) -> Option<ScanRecord> {
        self.db
            .prepare_cached(
                "SELECT id, status, created_at, entrypoint, type, finished_at, indexed_file_count \
                 FROM scan_history \
                 WHERE type in (:type1, :type2) \
                 AND status = :status \
                 AND entrypoint = :entrypoint \
                 ORDER BY created_at DESC LIMIT 1",
            )
            .and_then(|mut stmt| {
                stmt.query_row(
                    named_params! {
                        ":type1": ScanType::Full as i64,
                        ":type2": ScanType::Incremental as i64,
                        ":status": ScanStatus::Succeeded as i64,
                        ":entrypoint": path.to_string_lossy(),
                    },
                    map_scan_record,
                )
            })
            .inspect_err(|error| {
                if !matches!(error, rusqlite::Error::QueryReturnedNoRows) {
                    tracing::warn!(error = ?error, "looking up the last successful scan");
                }
            })
            .ok()
            .flatten()
    }

    fn last_scan(&self, path: &Path, scan_type: ScanType) -> Option<ScanRecord> {
        self.db
            .prepare_cached(
                "SELECT id, status, created_at, entrypoint, type, finished_at, indexed_file_count \
                 FROM scan_history \
                 WHERE type = :type \
                 AND entrypoint = :entrypoint \
                 ORDER BY created_at DESC LIMIT 1",
            )
            .and_then(|mut stmt| {
                stmt.query_row(
                    named_params! {
                        ":type": scan_type as i64,
                        ":entrypoint": path.to_string_lossy(),
                    },
                    map_scan_record,
                )
            })
            .inspect_err(|error| {
                if !matches!(error, rusqlite::Error::QueryReturnedNoRows) {
                    tracing::warn!(error = ?error, "looking up the last scan");
                }
            })
            .ok()
            .flatten()
    }

    fn has_vocabulary(&self) -> bool {
        self.db
            .query_row("SELECT 1 FROM vocabulary LIMIT 1", [], |_| Ok(()))
            .is_ok()
    }

    fn recent_directories(&self, limit: usize) -> Vec<PathBuf> {
        let mut stmt = match self.db.prepare_cached(
            "SELECT path FROM indexed_file WHERE type = 1 \
             ORDER BY last_modified_at DESC LIMIT :limit",
        ) {
            Ok(stmt) => stmt,
            Err(error) => {
                tracing::warn!(error = ?error, "listing recent directories");
                return Vec::new();
            }
        };
        let Ok(rows) = stmt.query_map(
            named_params! { ":limit": i64::try_from(limit).unwrap_or(i64::MAX) },
            |row| row.get::<_, Option<String>>(0),
        ) else {
            return Vec::new();
        };
        // No caller keeps more than the watcher's dynamic set; the SQL LIMIT
        // caps the rows anyway, this only bounds the upfront allocation.
        let mut dirs = Vec::with_capacity(limit.min(DYNAMIC_WATCH_COUNT));
        dirs.extend(rows.map_while(Result::ok).flatten().map(PathBuf::from));
        dirs
    }
}

/// The row id for `path`, when indexed.
pub(crate) fn file_id(db: &Connection, path: &Path) -> Option<i64> {
    db.prepare_cached("SELECT id FROM indexed_file WHERE path = :path")
        .and_then(|mut stmt| {
            stmt.query_row(named_params! { ":path": path.to_string_lossy() }, |row| {
                row.get(0)
            })
            .optional()
        })
        .ok()
        .flatten()
}

/// The strict/skeleton candidate lookup over `table`, one of the two FTS
/// tables: the row, its category, and its MIME name, narrowed by the category
/// filter the C++ appends, skeleton results in rank order.
fn candidate_sql(table: &str, order_by_rank: bool) -> String {
    format!(
        "SELECT f.path, f.category, mt.name \
        FROM indexed_file f \
        JOIN {table} ON {table}.rowid = f.id \
        LEFT JOIN mime_type mt ON mt.id = f.mime_type_id \
        WHERE {table} MATCH :search \
        AND (:category IS NULL OR f.category = :category) \
        {order} \
        LIMIT :limit",
        order = if order_by_rank {
            format!("ORDER BY {table}.rank")
        } else {
            String::new()
        },
    )
}

/// Reads a stored scan status. Unreachable values fall back to `Pending`:
/// writers only ever store the pinned discriminants.
pub(crate) fn status_from_db(value: i64) -> ScanStatus {
    match value {
        1 => ScanStatus::Started,
        2 => ScanStatus::Interrupted,
        3 => ScanStatus::Failed,
        4 => ScanStatus::Succeeded,
        _ => ScanStatus::Pending,
    }
}

/// Reads a stored scan type. Unreachable values fall back to `Full`: writers
/// only ever store the pinned discriminants, and `Full` is the schema's
/// standing assumption.
pub(crate) fn scan_type_from_db(value: i64) -> ScanType {
    match value {
        1 => ScanType::Incremental,
        _ => ScanType::Full,
    }
}

/// An integer column that reads as 0 when NULL, as the C++ `QVariant::toInt`
/// reads it. `created_at`, `finished_at` and `indexed_file_count` are all
/// nullable.
pub(crate) fn int_or_zero(row: &Row<'_>, column: usize) -> rusqlite::Result<i64> {
    Ok(row.get::<_, Option<i64>>(column)?.unwrap_or(0))
}

/// Reads a scan-history row in `mapScan` column order: id, status,
/// created_at, entrypoint, type, finished_at, indexed_file_count. `None` when
/// the row has no entrypoint.
pub(crate) fn map_scan_record(row: &Row<'_>) -> rusqlite::Result<Option<ScanRecord>> {
    let Some(path) = row.get::<_, Option<String>>(3)? else {
        return Ok(None);
    };
    Ok(Some(ScanRecord {
        id: i32::try_from(int_or_zero(row, 0)?).unwrap_or(i32::MAX),
        status: status_from_db(int_or_zero(row, 1)?),
        created_at: u64::try_from(int_or_zero(row, 2)?).unwrap_or(0),
        finished_at: u64::try_from(int_or_zero(row, 5)?).unwrap_or(0),
        indexed_file_count: int_or_zero(row, 6)?,
        path: PathBuf::from(path),
        scan_type: scan_type_from_db(int_or_zero(row, 4)?),
    }))
}

/// Reads the stored category number. Unknown values are `Other` on purpose:
/// the C++ reads the column straight into the enum, where anything outside
/// the schema default is an accident, not a category.
fn category_from_db(value: i64) -> IndexedFileCategory {
    match value {
        1 => IndexedFileCategory::Directory,
        2 => IndexedFileCategory::Image,
        3 => IndexedFileCategory::Video,
        4 => IndexedFileCategory::Audio,
        5 => IndexedFileCategory::Document,
        6 => IndexedFileCategory::Archive,
        7 => IndexedFileCategory::Application,
        _ => IndexedFileCategory::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discriminants_match_the_stored_numbers() {
        assert_eq!(IndexedFileCategory::Other as i64, 0);
        assert_eq!(IndexedFileCategory::Directory as i64, 1);
        assert_eq!(IndexedFileCategory::Image as i64, 2);
        assert_eq!(IndexedFileCategory::Video as i64, 3);
        assert_eq!(IndexedFileCategory::Audio as i64, 4);
        assert_eq!(IndexedFileCategory::Document as i64, 5);
        assert_eq!(IndexedFileCategory::Archive as i64, 6);
        assert_eq!(IndexedFileCategory::Application as i64, 7);
    }

    #[test]
    fn categories_round_trip_with_other_as_the_fallback() {
        for (number, category) in [
            (0, IndexedFileCategory::Other),
            (1, IndexedFileCategory::Directory),
            (2, IndexedFileCategory::Image),
            (3, IndexedFileCategory::Video),
            (4, IndexedFileCategory::Audio),
            (5, IndexedFileCategory::Document),
            (6, IndexedFileCategory::Archive),
            (7, IndexedFileCategory::Application),
        ] {
            assert_eq!(category_from_db(number), category);
        }
        assert_eq!(category_from_db(-1), IndexedFileCategory::Other);
        assert_eq!(category_from_db(99), IndexedFileCategory::Other);
    }

    #[test]
    fn scan_types_round_trip_with_full_as_the_fallback() {
        assert_eq!(scan_type_from_db(0), ScanType::Full);
        assert_eq!(scan_type_from_db(1), ScanType::Incremental);
        assert_eq!(scan_type_from_db(-1), ScanType::Full);
        assert_eq!(scan_type_from_db(99), ScanType::Full);
    }

    #[test]
    fn candidate_queries_match_filter_limit_and_rank() {
        let strict = candidate_sql("path_idx", false);
        assert!(strict.contains("path_idx MATCH :search"));
        assert!(strict.contains("(:category IS NULL OR f.category = :category)"));
        assert!(strict.contains("LIMIT :limit"));
        assert!(!strict.contains("ORDER BY"));

        let skeleton = candidate_sql("skeleton_idx", true);
        assert!(skeleton.contains("skeleton_idx MATCH :search"));
        assert!(skeleton.contains("ORDER BY skeleton_idx.rank"));
    }

    /// A live database with the shared writer schema plus seed rows. These
    /// `live_*` tests need the sys crate, so they run in CI — not in the
    /// header-less scratch crate, which runs everything else with
    /// `-- --skip live_`.
    fn live_seed() -> (tempfile::TempDir, SqliteReader) {
        let dir = tempfile::tempdir().expect("tempdir");
        let reader = SqliteReader::open(&dir.path().join("index.db")).expect("open");
        for statement in crate::sqlite_writer::WRITER_SCHEMA {
            reader.db.execute_batch(statement).expect("schema");
        }
        for statement in [
            "INSERT INTO mime_type(name) VALUES ('text/plain')",
            "INSERT INTO indexed_file(path, skeleton_path, category, mime_type_id) \
             VALUES ('/home/ada/report.txt', 'rprt txt', 5, 1)",
            "INSERT INTO indexed_file(path, skeleton_path, category) \
             VALUES ('/home/ada/photo.jpg', 'pht jpg', 2)",
            "INSERT INTO vocabulary(word, rank) VALUES ('report', 5)",
        ] {
            reader.db.execute_batch(statement).expect("seed");
        }
        (dir, reader)
    }

    #[test]
    fn live_strict_search_maps_rows() {
        let (_dir, reader) = live_seed();
        let results = reader.search_candidates("\"report\"", 10, &SearchOptions::default());
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, PathBuf::from("/home/ada/report.txt"));
        assert_eq!(results[0].category, IndexedFileCategory::Document);
        assert_eq!(results[0].mime_type.as_deref(), Some("text/plain"));
    }

    #[test]
    fn live_category_filter_narrows() {
        let (_dir, reader) = live_seed();
        let photo = SearchOptions {
            category: Some(IndexedFileCategory::Image),
        };
        assert!(
            reader
                .search_candidates("\"report\"", 10, &photo)
                .is_empty()
        );
        let hits = reader.search_candidates("\"photo\"", 10, &photo);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].mime_type, None);
    }

    #[test]
    fn live_skeleton_search_finds_skeletons() {
        let (_dir, reader) = live_seed();
        let results = reader.search_skeleton_candidates("\"rprt\"", 10, &SearchOptions::default());
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, PathBuf::from("/home/ada/report.txt"));
    }

    #[test]
    fn live_vocabulary_suggests_close_words() {
        let (_dir, reader) = live_seed();
        for prefix in [true, false] {
            let suggestions = reader.vocabulary_suggestions("reprot", 20, prefix);
            let found = suggestions
                .iter()
                .find(|suggestion| suggestion.word == "report")
                .expect("report is suggested");
            assert_eq!(found.rank, 5);
        }
    }

    #[test]
    fn live_vocabulary_is_reloaded_after_another_connection_writes_it() {
        let (dir, reader) = live_seed();
        assert!(
            reader
                .vocabulary_suggestions("invioce", 20, false)
                .is_empty()
        );
        // A rebuild lands from the writer's connection, not this one.
        let writer =
            compass_sqlcipher_sys::open(&dir.path().join("index.db"), &[]).expect("writer");
        writer
            .execute_batch("INSERT INTO vocabulary(word, rank) VALUES ('invoice', 2)")
            .expect("vocabulary write");
        let found = reader.vocabulary_suggestions("invioce", 20, false);
        assert_eq!(found.first().map(|s| s.word.as_str()), Some("invoice"));
    }

    #[test]
    fn live_reader_reports_open() {
        let (_dir, reader) = live_seed();
        assert!(reader.is_open());
    }

    #[test]
    fn live_last_successful_scan_returns_the_latest() {
        let (_dir, reader) = live_seed();
        for statement in [
            "INSERT INTO scan_history(entrypoint, type, status, created_at) \
             VALUES ('/home/ada', 0, 4, 1000)",
            "INSERT INTO scan_history(entrypoint, type, status, created_at) \
             VALUES ('/home/ada', 1, 4, 2000)",
            "INSERT INTO scan_history(entrypoint, type, status, created_at) \
             VALUES ('/home/ada', 0, 3, 3000)",
            "INSERT INTO scan_history(entrypoint, type, status, created_at) \
             VALUES ('/home/ada/docs', 0, 4, 4000)",
        ] {
            reader.db.execute_batch(statement).expect("scan seed");
        }

        let scan = reader
            .last_successful_scan(Path::new("/home/ada"))
            .expect("a succeeded scan");
        assert_eq!(scan.scan_type, ScanType::Incremental);
        assert_eq!(scan.status, ScanStatus::Succeeded);
        assert_eq!(scan.created_at, 2000);
        assert_eq!(scan.path, PathBuf::from("/home/ada"));

        let docs = reader
            .last_successful_scan(Path::new("/home/ada/docs"))
            .expect("the docs scan");
        assert_eq!(docs.created_at, 4000);
    }

    #[test]
    fn live_last_successful_scan_ignores_failures() {
        let (_dir, reader) = live_seed();
        reader
            .db
            .execute_batch(
                "INSERT INTO scan_history(entrypoint, type, status, created_at) \
                 VALUES ('/home/ada', 0, 3, 1000)",
            )
            .expect("scan seed");
        assert_eq!(reader.last_successful_scan(Path::new("/home/ada")), None);
        assert_eq!(
            reader.last_successful_scan(Path::new("/home/ada/nowhere")),
            None
        );
    }

    #[test]
    fn live_last_scan_returns_the_latest_of_one_shape() {
        let (_dir, reader) = live_seed();
        for statement in [
            "INSERT INTO scan_history(entrypoint, type, status, created_at) \
             VALUES ('/home/ada', 0, 4, 1000)",
            "INSERT INTO scan_history(entrypoint, type, status, created_at) \
             VALUES ('/home/ada', 0, 3, 2000)",
            "INSERT INTO scan_history(entrypoint, type, status, created_at) \
             VALUES ('/home/ada', 1, 4, 3000)",
        ] {
            reader.db.execute_batch(statement).expect("scan seed");
        }

        // Failures count: the orchestrator restarts from those, not just
        // successes — so the latest full scan is the failed one at 2000.
        let full = reader
            .last_scan(Path::new("/home/ada"), ScanType::Full)
            .expect("a full scan");
        assert_eq!(full.status, ScanStatus::Failed);
        assert_eq!(full.created_at, 2000);
        let incremental = reader
            .last_scan(Path::new("/home/ada"), ScanType::Incremental)
            .expect("an incremental scan");
        assert_eq!(incremental.created_at, 3000);
        assert_eq!(
            reader.last_scan(Path::new("/home/ada/nowhere"), ScanType::Full),
            None
        );
    }

    #[test]
    fn live_vocabulary_reports_words_and_missing_tables() {
        let (_dir, reader) = live_seed();
        assert!(reader.has_vocabulary());

        let missing = tempfile::tempdir().expect("tempdir");
        let bare = SqliteReader::open(&missing.path().join("empty.db")).expect("open");
        assert!(!bare.has_vocabulary());
        assert_eq!(bare.last_scan(Path::new("/home/ada"), ScanType::Full), None);
    }

    #[test]
    fn live_recent_directories_lists_newest_dirs_first() {
        let (_dir, reader) = live_seed();
        for statement in [
            "INSERT INTO indexed_file(path, skeleton_path, type, last_modified_at) \
             VALUES ('/home/ada/old', 'ld', 1, 1000)",
            "INSERT INTO indexed_file(path, skeleton_path, type, last_modified_at) \
             VALUES ('/home/ada/new', 'nw', 1, 3000)",
            "INSERT INTO indexed_file(path, skeleton_path, type, last_modified_at) \
             VALUES ('/home/ada/mid', 'md', 1, 2000)",
            "INSERT INTO indexed_file(path, skeleton_path, type, last_modified_at) \
             VALUES ('/home/ada/file.txt', 'fl txt', 0, 4000)",
        ] {
            reader.db.execute_batch(statement).expect("directory seed");
        }

        // Files never qualify, however fresh — only type 1 rows do.
        assert_eq!(
            reader.recent_directories(10),
            [
                PathBuf::from("/home/ada/new"),
                PathBuf::from("/home/ada/mid"),
                PathBuf::from("/home/ada/old"),
            ]
        );
        assert_eq!(
            reader.recent_directories(2),
            [
                PathBuf::from("/home/ada/new"),
                PathBuf::from("/home/ada/mid"),
            ]
        );

        let missing = tempfile::tempdir().expect("tempdir");
        let bare = SqliteReader::open(&missing.path().join("empty.db")).expect("open");
        assert!(bare.recent_directories(10).is_empty());
    }

    #[test]
    fn live_directory_listing_returns_direct_children() {
        let (_dir, reader) = live_seed();
        for statement in [
            "INSERT INTO indexed_file(path, skeleton_path) \
             VALUES ('/home/ada/docs', 'dcs')",
            "INSERT INTO indexed_file(path, skeleton_path, parent_id) \
             VALUES ('/home/ada/docs/a.txt', 'a txt', \
             (SELECT id FROM indexed_file WHERE path = '/home/ada/docs'))",
            "INSERT INTO indexed_file(path, skeleton_path, parent_id) \
             VALUES ('/home/ada/docs/b.txt', 'b txt', \
             (SELECT id FROM indexed_file WHERE path = '/home/ada/docs'))",
        ] {
            reader.db.execute_batch(statement).expect("file seed");
        }

        let children = reader.list_indexed_directory_files(Path::new("/home/ada/docs"));
        assert_eq!(
            children,
            HashSet::from([
                PathBuf::from("/home/ada/docs/a.txt"),
                PathBuf::from("/home/ada/docs/b.txt"),
            ])
        );
        assert!(
            reader
                .list_indexed_directory_files(Path::new("/home/ada/photo.jpg"))
                .is_empty()
        );
        assert!(
            reader
                .list_indexed_directory_files(Path::new("/home/ada/nowhere"))
                .is_empty()
        );
    }

    #[test]
    fn live_tracks_file_reports_indexed_paths() {
        let (_dir, reader) = live_seed();
        assert!(reader.tracks_file(Path::new("/home/ada/report.txt")));
        assert!(!reader.tracks_file(Path::new("/home/ada/missing.txt")));
    }
}
