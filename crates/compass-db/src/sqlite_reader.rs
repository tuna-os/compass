//! The [`IndexReader`] that talks to SQLite.
//!
//! Implements the three read queries `FileIndexerDatabase` serves the query
//! engine — strict candidates, skeleton candidates, spellfix suggestions —
//! over a [`Database`]. The SQL mirrors the C++ row for row: same tables,
//! same match strings, same category filter, same skeleton rank order.
//!
//! One reader owns one connection on one thread, the way the C++ query pool
//! gives each worker its own engine: `Database` is `Send` but not `Sync`, so
//! sharing an index across threads means one reader per thread, not one
//! reader under a lock.
//!
//! The scanner owns the schema; this reader only queries. It never migrates,
//! never writes, and never encrypts: the C++ engine keeps the file index
//! unencrypted.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use compass_sqlcipher_sys::{Database, Statement};

use crate::db_writer::{ScanRecord, ScanStatus, ScanType};
use crate::query_engine::{IndexedFileCategory, SearchCandidate, SearchOptions};
use crate::query_policy::SpellfixSuggestion;
use crate::query_reader::IndexReader;

/// One file-index database, open for reading.
pub struct SqliteReader {
    db: Database,
}

impl SqliteReader {
    /// Opens the file index at `path`.
    ///
    /// # Errors
    ///
    /// Returns [`compass_sqlcipher_sys::Error`] if the file cannot be opened.
    pub fn open(path: &Path) -> Result<Self, compass_sqlcipher_sys::Error> {
        Ok(Self {
            db: Database::open(path, &[])?,
        })
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
        let mut stmt = match self.db.prepare(&sql) {
            Ok(stmt) => stmt,
            Err(error) => {
                tracing::warn!(error = ?error, table, "preparing the candidate query");
                return Vec::new();
            }
        };
        if let Err(error) = bind_search(&mut stmt, query, limit, options) {
            tracing::warn!(error = ?error, table, "binding the candidate query");
            return Vec::new();
        }
        let mut results = Vec::new();
        loop {
            match stmt.step() {
                Ok(true) => {}
                Ok(false) => break,
                Err(error) => {
                    tracing::warn!(error = ?error, table, "reading candidate rows");
                    break;
                }
            }
            let Some(path) = stmt.column_text(0) else {
                continue;
            };
            results.push(SearchCandidate {
                path: PathBuf::from(path),
                category: category_from_db(stmt.column_int64(1)),
                mime_type: stmt.column_text(2),
            });
        }
        results
    }
}

impl IndexReader for SqliteReader {
    /// Always true: a constructed reader holds its connection, and
    /// [`Database`] has no close. It exists so the engine treats every reader
    /// uniformly.
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

    fn spellfix_suggestions(&self, word: &str, top: i32, prefix: bool) -> Vec<SpellfixSuggestion> {
        let mut stmt = match self.db.prepare(SPELLFIX_SQL) {
            Ok(stmt) => stmt,
            Err(error) => {
                tracing::warn!(error = ?error, "preparing the spellfix query");
                return Vec::new();
            }
        };
        let pattern = spellfix_pattern(word, prefix);
        if let Err(error) = bind_spellfix(&mut stmt, &pattern, top) {
            tracing::warn!(error = ?error, "binding the spellfix query");
            return Vec::new();
        }
        let mut results = Vec::new();
        loop {
            match stmt.step() {
                Ok(true) => {}
                Ok(false) => break,
                Err(error) => {
                    tracing::warn!(error = ?error, "reading spellfix rows");
                    break;
                }
            }
            let Some(found) = stmt.column_text(0) else {
                continue;
            };
            results.push(SpellfixSuggestion {
                word: found,
                distance: i32::try_from(stmt.column_int64(1)).unwrap_or(i32::MAX),
                score: i32::try_from(stmt.column_int64(2)).unwrap_or(i32::MAX),
                rank: stmt.column_int64(3),
            });
        }
        results
    }

    fn list_indexed_directory_files(&self, path: &Path) -> HashSet<PathBuf> {
        let Some(dir_id) = file_id(&self.db, path) else {
            return HashSet::new();
        };
        let mut stmt = match self
            .db
            .prepare("SELECT path FROM indexed_file WHERE parent_id = :parent_id")
        {
            Ok(stmt) => stmt,
            Err(error) => {
                tracing::warn!(error = ?error, "listing the indexed directory");
                return HashSet::new();
            }
        };
        if stmt.bind_int64(":parent_id", dir_id).is_err() {
            tracing::warn!("binding the indexed directory");
            return HashSet::new();
        }
        let mut paths = HashSet::new();
        loop {
            match stmt.step() {
                Ok(true) => {}
                Ok(false) => break,
                Err(error) => {
                    tracing::warn!(error = ?error, "reading indexed directory rows");
                    break;
                }
            }
            if let Some(found) = stmt.column_text(0) {
                paths.insert(PathBuf::from(found));
            }
        }
        paths
    }

    fn tracks_file(&self, path: &Path) -> bool {
        let mut stmt = match self
            .db
            .prepare("SELECT COUNT(*) FROM indexed_file WHERE path = :path")
        {
            Ok(stmt) => stmt,
            Err(error) => {
                tracing::warn!(error = ?error, "checking the tracked file");
                return false;
            }
        };
        if stmt.bind_text(":path", &path.to_string_lossy()).is_err() {
            return false;
        }
        match stmt.step() {
            Ok(true) => stmt.column_int64(0) != 0,
            _ => false,
        }
    }

    fn last_successful_scan(&self, path: &Path) -> Option<ScanRecord> {
        let mut stmt = match self.db.prepare(
            "SELECT id, status, created_at, entrypoint, type, finished_at, indexed_file_count \
             FROM scan_history \
             WHERE type in (:type1, :type2) \
             AND status = :status \
             AND entrypoint = :entrypoint \
             ORDER BY created_at DESC LIMIT 1",
        ) {
            Ok(stmt) => stmt,
            Err(error) => {
                tracing::warn!(error = ?error, "looking up the last successful scan");
                return None;
            }
        };
        if stmt.bind_int64(":type1", ScanType::Full as i64).is_err()
            || stmt
                .bind_int64(":type2", ScanType::Incremental as i64)
                .is_err()
            || stmt
                .bind_int64(":status", ScanStatus::Succeeded as i64)
                .is_err()
            || stmt
                .bind_text(":entrypoint", &path.to_string_lossy())
                .is_err()
        {
            return None;
        }
        match stmt.step() {
            Ok(true) => map_scan_record(&stmt),
            _ => None,
        }
    }
}

/// The row id for `path`, when indexed.
pub(crate) fn file_id(db: &Database, path: &Path) -> Option<i64> {
    let mut stmt = db
        .prepare("SELECT id FROM indexed_file WHERE path = :path")
        .ok()?;
    stmt.bind_text(":path", &path.to_string_lossy()).ok()?;
    match stmt.step() {
        Ok(true) => Some(stmt.column_int64(0)),
        _ => None,
    }
}

/// The spellfix lookup: suggestions near `:pattern`, at most `:top`.
const SPELLFIX_SQL: &str = "SELECT word, distance, score, rank \
    FROM spellfix_vocab \
    WHERE word MATCH :pattern AND top = :top";

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

/// Binds the candidate parameters: the match string, the row cap, and the
/// optional category.
fn bind_search(
    stmt: &mut Statement<'_>,
    query: &str,
    limit: usize,
    options: &SearchOptions,
) -> Result<(), compass_sqlcipher_sys::Error> {
    stmt.bind_text(":search", query)?;
    stmt.bind_int64(":limit", i64::try_from(limit).unwrap_or(i64::MAX))?;
    match options.category {
        Some(category) => stmt.bind_int64(":category", category as i64)?,
        None => stmt.bind_null(":category")?,
    }
    Ok(())
}

/// Binds the spellfix parameters: the lowered pattern and the suggestion cap.
fn bind_spellfix(
    stmt: &mut Statement<'_>,
    pattern: &str,
    top: i32,
) -> Result<(), compass_sqlcipher_sys::Error> {
    stmt.bind_text(":pattern", pattern)?;
    stmt.bind_int64(":top", i64::from(top))?;
    Ok(())
}

/// The spellfix pattern for `word`: lowered, prefix-extended when asked, the
/// way the C++ builds it.
fn spellfix_pattern(word: &str, prefix: bool) -> String {
    let mut pattern = word.to_ascii_lowercase();
    if prefix {
        pattern.push('*');
    }
    pattern
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

/// Reads a scan-history row in `mapScan` column order: id, status,
/// created_at, entrypoint, type, finished_at, indexed_file_count.
pub(crate) fn map_scan_record(stmt: &Statement<'_>) -> Option<ScanRecord> {
    Some(ScanRecord {
        id: i32::try_from(stmt.column_int64(0)).unwrap_or(i32::MAX),
        status: status_from_db(stmt.column_int64(1)),
        created_at: u64::try_from(stmt.column_int64(2)).unwrap_or(0),
        finished_at: u64::try_from(stmt.column_int64(5)).unwrap_or(0),
        indexed_file_count: stmt.column_int64(6),
        path: PathBuf::from(stmt.column_text(3)?),
        scan_type: scan_type_from_db(stmt.column_int64(4)),
    })
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
    fn spellfix_patterns_lowercase_and_star_on_request() {
        assert_eq!(spellfix_pattern("Report", true), "report*");
        assert_eq!(spellfix_pattern("Report", false), "report");
        assert_eq!(spellfix_pattern("", true), "*");
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

    /// A live database with the real schema shape: indexed rows, MIME names,
    /// both FTS tables, and a spellfix vocabulary. These `live_*` tests need
    /// the sys crate, so they run in CI — not in the header-less scratch
    /// crate, which runs everything else with `-- --skip live_`.
    fn live_seed() -> (tempfile::TempDir, SqliteReader) {
        let dir = tempfile::tempdir().expect("tempdir");
        let reader = SqliteReader::open(&dir.path().join("index.db")).expect("open");
        for statement in [
            "CREATE TABLE indexed_file (id INTEGER PRIMARY KEY AUTOINCREMENT, \
             path TEXT UNIQUE NOT NULL, skeleton_path TEXT NOT NULL, \
             category INT NOT NULL DEFAULT 0, mime_type_id INT)",
            "CREATE TABLE mime_type (id INTEGER PRIMARY KEY AUTOINCREMENT, \
             name TEXT UNIQUE NOT NULL)",
            "CREATE TABLE scan_history (id INTEGER PRIMARY KEY AUTOINCREMENT, \
             status INTEGER NOT NULL, created_at INT DEFAULT (unixepoch()), \
             finished_at INT, entrypoint TEXT NOT NULL, error TEXT, \
             type INT NOT NULL, indexed_file_count INT DEFAULT 0)",
            "CREATE VIRTUAL TABLE path_idx USING fts5(path, content=indexed_file, \
             tokenize='fuzzy_trigram remove_diacritics 2')",
            "CREATE TRIGGER path_idx_ai AFTER INSERT ON indexed_file BEGIN \
             INSERT INTO path_idx(rowid, path) VALUES (new.id, new.path); END",
            "CREATE VIRTUAL TABLE skeleton_idx USING fts5(skeleton_path, \
             content=indexed_file, \
             tokenize='fuzzy_trigram remove_diacritics 2 skeleton 1 skipgrams 1')",
            "CREATE TRIGGER skeleton_idx_ai AFTER INSERT ON indexed_file BEGIN \
             INSERT INTO skeleton_idx(rowid, skeleton_path) \
             VALUES (new.id, new.skeleton_path); END",
            "CREATE VIRTUAL TABLE spellfix_vocab USING spellfix1",
            "INSERT INTO mime_type(name) VALUES ('text/plain')",
            "INSERT INTO indexed_file(path, skeleton_path, category, mime_type_id) \
             VALUES ('/home/ada/report.txt', 'rprt txt', 5, 1)",
            "INSERT INTO indexed_file(path, skeleton_path, category) \
             VALUES ('/home/ada/photo.jpg', 'pht jpg', 2)",
            "INSERT INTO spellfix_vocab(word, rank) VALUES ('report', 5)",
        ] {
            reader.db.execute(statement).expect("seed");
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
    fn live_spellfix_suggests_the_vocabulary() {
        let (_dir, reader) = live_seed();
        for prefix in [true, false] {
            let suggestions = reader.spellfix_suggestions("reprot", 20, prefix);
            let found = suggestions
                .iter()
                .find(|suggestion| suggestion.word == "report")
                .expect("report is suggested");
            assert_eq!(found.rank, 5);
        }
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
            reader.db.execute(statement).expect("scan seed");
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
            .execute(
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
            reader.db.execute(statement).expect("file seed");
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
