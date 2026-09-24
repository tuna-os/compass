//! The [`IndexDatabase`] that writes to SQLite.
//!
//! Implements the mutations `DbWriter` queues — scan records, file upserts
//! and removals, compaction, and the spellfix vocabulary rebuild — over a
//! [`Database`]. The SQL mirrors `file-indexer-db.cpp` row for row: same
//! tables, same upsert, same subtree ranges, same compaction thresholds.
//!
//! One writer owns one connection on one thread: `DbWriter` moves the
//! database into its worker, the way each C++ writer owns its
//! `FileIndexerDatabase`. Reads live next door in
//! [`crate::sqlite_reader`]; unifying the two handles is later work, once the
//! scanner owns the schema and the open path.
//!
//! File metadata comes from the filesystem and from crates: categories from
//! [`compass_core::file_category`], skeletons from
//! [`compass_core::vocabulary`], MIME names from `mime_guess` by extension —
//! the same lookup the C++ spells `QMimeDatabase::MatchExtension`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use compass_core::file_category::{FileCategory, file_category};
use compass_core::vocabulary::{basename, skeletonize_token, tokenize_filename};
use compass_sqlcipher_sys::{Database, Statement};

use crate::db_writer::{FileEvent, FileEventType, IndexDatabase, ScanRecord, ScanStatus, ScanType};
use crate::query_engine::IndexedFileCategory;
use crate::sqlite_reader::{file_id, status_from_db};

/// Below this size compaction never pays.
const COMPACT_MIN_DB_BYTES: i64 = 32 * 1024 * 1024;

/// At this much free space, relative to the file, compaction pays.
const COMPACT_MIN_FREE_PERCENT: i64 = 25;

/// One file-index database, open for writing.
pub struct SqliteWriter {
    db: Database,
    mime_ids: HashMap<String, i64>,
}

impl SqliteWriter {
    /// Opens the file index at `path`.
    ///
    /// The index is unencrypted, as the C++ engine keeps it. The schema
    /// already exists: the scanner owns migrations, this writer only writes.
    ///
    /// # Errors
    ///
    /// Returns [`compass_sqlcipher_sys::Error`] if the file cannot be opened.
    pub fn open(path: &Path) -> Result<Self, compass_sqlcipher_sys::Error> {
        Ok(Self {
            db: Database::open(path, &[])?,
            mime_ids: HashMap::new(),
        })
    }

    /// The cached row id for a MIME name, inserting the name on first use.
    /// Empty names map to `None`: nothing to look up, nothing to store.
    fn mime_id_for(
        db: &Database,
        mime_ids: &mut HashMap<String, i64>,
        name: Option<&str>,
    ) -> Option<i64> {
        let name = name.filter(|name| !name.is_empty())?;
        if let Some(id) = mime_ids.get(name) {
            return Some(*id);
        }
        let mut stmt = db
            .prepare(
                "INSERT INTO mime_type(name) VALUES (:name) \
                 ON CONFLICT(name) DO UPDATE SET name = excluded.name \
                 RETURNING id",
            )
            .ok()?;
        stmt.bind_text(":name", name).ok()?;
        match stmt.step() {
            Ok(true) => {
                let id = stmt.column_int64(0);
                mime_ids.insert(name.to_owned(), id);
                Some(id)
            }
            _ => {
                tracing::warn!(name, "resolving the MIME type id");
                None
            }
        }
    }

    /// The cached parent row id: looked up once per directory per batch.
    fn resolve_parent(
        db: &Database,
        path: &Path,
        parents: &mut HashMap<String, Option<i64>>,
    ) -> Option<i64> {
        let parent = path.parent()?;
        if parent.as_os_str().is_empty() || parent == path {
            return None;
        }
        let key = parent.to_string_lossy().into_owned();
        if let Some(id) = parents.get(&key) {
            return *id;
        }
        let id = file_id(db, parent);
        parents.insert(key, id);
        id
    }

    /// One `PRAGMA` integer, or 0 when the database will not say.
    fn pragma_int(&self, sql: &str) -> i64 {
        let mut stmt = match self.db.prepare(sql) {
            Ok(stmt) => stmt,
            Err(_) => return 0,
        };
        match stmt.step() {
            Ok(true) => stmt.column_int64(0),
            _ => 0,
        }
    }

    /// Inserts or refreshes one row. `modified` and `size` are `None` when
    /// the filesystem would not say; `mime` is the resolved MIME row id.
    #[allow(clippy::too_many_arguments)]
    fn upsert_file(
        db: &Database,
        mime_ids: &mut HashMap<String, i64>,
        parents: &mut HashMap<String, Option<i64>>,
        stmt: &mut Statement<'_>,
        path: &Path,
        modified: Option<i64>,
        is_directory: bool,
        size: Option<i64>,
    ) -> bool {
        let row = UpsertRow {
            path,
            skeleton: skeleton_document(path),
            parent: Self::resolve_parent(db, path, parents),
            modified,
            is_directory,
            category: indexed_category(path, is_directory),
            size,
            mime: Self::mime_id_for(db, mime_ids, mime_name_for(path, is_directory).as_deref()),
        };
        if let Err(error) = bind_upsert(stmt, &row) {
            tracing::warn!(error = ?error, path = ?path, "binding the file upsert");
            return false;
        }
        if let Err(error) = exec_once(stmt) {
            tracing::warn!(error = ?error, path = ?path, "indexing the file");
            return false;
        }
        true
    }
}

impl IndexDatabase for SqliteWriter {
    /// Always true: a constructed writer holds its connection, and
    /// [`Database`] has no close. It exists so the writer treats every
    /// database uniformly.
    fn is_open(&self) -> bool {
        true
    }

    fn update_scan_status(&mut self, scan_id: i32, status: ScanStatus) -> bool {
        let mut stmt = match self
            .db
            .prepare("UPDATE scan_history SET status = :status WHERE id = :id")
        {
            Ok(stmt) => stmt,
            Err(error) => {
                tracing::warn!(error = ?error, scan_id, "preparing the status update");
                return false;
            }
        };
        if stmt.bind_int64(":id", i64::from(scan_id)).is_err()
            || stmt.bind_int64(":status", status as i64).is_err()
            || exec_once(&mut stmt).is_err()
        {
            tracing::warn!(scan_id, "updating the scan status");
            return false;
        }
        true
    }

    fn finalize_scan(&mut self, scan_id: i32, status: ScanStatus, indexed_file_count: i64) -> bool {
        let mut stmt = match self.db.prepare(
            "UPDATE scan_history SET status = :status, finished_at = unixepoch(), \
             indexed_file_count = :count WHERE id = :id",
        ) {
            Ok(stmt) => stmt,
            Err(error) => {
                tracing::warn!(error = ?error, scan_id, "preparing the scan finalization");
                return false;
            }
        };
        if stmt.bind_int64(":id", i64::from(scan_id)).is_err()
            || stmt.bind_int64(":status", status as i64).is_err()
            || stmt.bind_int64(":count", indexed_file_count).is_err()
            || exec_once(&mut stmt).is_err()
        {
            tracing::warn!(scan_id, "finalizing the scan");
            return false;
        }
        true
    }

    fn set_scan_error(&mut self, scan_id: i32, error: &str) -> bool {
        let mut stmt = match self.db.prepare(
            "UPDATE scan_history SET status = :status, error = :error, \
             finished_at = unixepoch() WHERE id = :id",
        ) {
            Ok(stmt) => stmt,
            Err(error) => {
                tracing::warn!(error = ?error, scan_id, "preparing the scan error");
                return false;
            }
        };
        if stmt.bind_int64(":id", i64::from(scan_id)).is_err()
            || stmt
                .bind_int64(":status", ScanStatus::Failed as i64)
                .is_err()
            || stmt.bind_text(":error", error).is_err()
            || exec_once(&mut stmt).is_err()
        {
            tracing::warn!(scan_id, "recording the scan error");
            return false;
        }
        true
    }

    fn prune_scan_history(&mut self, max_age_seconds: i64) -> bool {
        let mut stmt = match self.db.prepare(
            "DELETE FROM scan_history \
             WHERE created_at < unixepoch() - :maxAge \
             AND id NOT IN (SELECT MAX(id) FROM scan_history \
             GROUP BY entrypoint, type, status)",
        ) {
            Ok(stmt) => stmt,
            Err(error) => {
                tracing::warn!(error = ?error, "preparing the history prune");
                return false;
            }
        };
        if stmt.bind_int64(":maxAge", max_age_seconds).is_err() || exec_once(&mut stmt).is_err() {
            tracing::warn!("pruning the scan history");
            return false;
        }
        true
    }

    fn create_scan(&mut self, path: &Path, scan_type: ScanType) -> Result<ScanRecord, String> {
        let fail = |context: &str| {
            tracing::warn!(context, "creating the scan");
            Err("Failed to create scan history".to_owned())
        };
        let mut stmt = match self.db.prepare(
            "INSERT INTO scan_history (entrypoint, type, status) \
             VALUES (:entrypoint, :type, :status) \
             RETURNING id, status, created_at, entrypoint",
        ) {
            Ok(stmt) => stmt,
            Err(_) => return fail("preparing the insert"),
        };
        let entrypoint = path.to_string_lossy();
        if stmt.bind_text(":entrypoint", &entrypoint).is_err()
            || stmt.bind_int64(":type", scan_type as i64).is_err()
            || stmt
                .bind_int64(":status", ScanStatus::Pending as i64)
                .is_err()
        {
            return fail("binding the insert");
        }
        match stmt.step() {
            Ok(true) => Ok(ScanRecord {
                id: i32::try_from(stmt.column_int64(0)).unwrap_or(i32::MAX),
                status: status_from_db(stmt.column_int64(1)),
                created_at: u64::try_from(stmt.column_int64(2)).unwrap_or(0),
                finished_at: 0,
                indexed_file_count: 0,
                path: path.to_path_buf(),
                scan_type,
            }),
            _ => fail("running the insert"),
        }
    }

    fn index_files(&mut self, paths: &[PathBuf]) {
        let tx = match self.db.transaction() {
            Ok(tx) => tx,
            Err(error) => {
                tracing::warn!(error = ?error, "opening the index batch");
                return;
            }
        };
        let mut stmt = match self.db.prepare(UPSERT_SQL) {
            Ok(stmt) => stmt,
            Err(error) => {
                tracing::warn!(error = ?error, "preparing the file upsert");
                return;
            }
        };
        let mut parents = HashMap::new();
        for path in paths {
            let metadata = std::fs::metadata(path);
            let is_directory = metadata.as_ref().is_ok_and(std::fs::Metadata::is_dir);
            let modified = metadata
                .as_ref()
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .map(unix_seconds);
            let size = file_size_bytes(metadata.as_ref().ok(), is_directory);
            if !Self::upsert_file(
                &self.db,
                &mut self.mime_ids,
                &mut parents,
                &mut stmt,
                path,
                modified,
                is_directory,
                size,
            ) {
                return;
            }
        }
        if let Err(error) = tx.commit() {
            tracing::error!(error = ?error, "committing the index batch");
        }
    }

    fn delete_indexed_files(&mut self, paths: &[PathBuf]) {
        let tx = match self.db.transaction() {
            Ok(tx) => tx,
            Err(error) => {
                tracing::warn!(error = ?error, "opening the delete batch");
                return;
            }
        };
        let mut exact = match self
            .db
            .prepare("DELETE FROM indexed_file WHERE path = :path")
        {
            Ok(stmt) => stmt,
            Err(error) => {
                tracing::warn!(error = ?error, "preparing the file delete");
                return;
            }
        };
        let mut subtree = match self
            .db
            .prepare("DELETE FROM indexed_file WHERE path >= :lower AND path < :upper")
        {
            Ok(stmt) => stmt,
            Err(error) => {
                tracing::warn!(error = ?error, "preparing the subtree delete");
                return;
            }
        };
        for path in paths {
            let name = path.to_string_lossy();
            let (lower, upper) = subtree_range(path);
            if exact.bind_text(":path", &name).is_err()
                || subtree.bind_text(":lower", &lower).is_err()
                || subtree.bind_text(":upper", &upper).is_err()
                || exec_once(&mut exact).is_err()
                || exec_once(&mut subtree).is_err()
            {
                tracing::warn!(path = ?path, "deleting the indexed file");
                return;
            }
        }
        if let Err(error) = tx.commit() {
            tracing::error!(error = ?error, "committing the delete batch");
        }
    }

    fn delete_all_indexed_files(&mut self) {
        if let Err(error) = self.db.execute("DELETE FROM indexed_file") {
            tracing::error!(error = ?error, "deleting all indexed files");
        }
    }

    fn compact(&mut self) {
        if let Err(error) = self
            .db
            .execute("INSERT INTO path_idx(path_idx) VALUES('optimize')")
        {
            tracing::warn!(error = ?error, "optimizing the path index");
        }
        if let Err(error) = self
            .db
            .execute("INSERT INTO skeleton_idx(skeleton_idx) VALUES('optimize')")
        {
            tracing::warn!(error = ?error, "optimizing the skeleton index");
        }
        if let Err(error) = self.db.execute("VACUUM") {
            tracing::warn!(error = ?error, "vacuuming the index");
            return;
        }
        if let Err(error) = self.db.execute("PRAGMA wal_checkpoint(TRUNCATE)") {
            tracing::warn!(error = ?error, "checkpointing the index");
        }
    }

    fn needs_compaction(&self) -> bool {
        let page_count = self.pragma_int("PRAGMA page_count");
        let free_count = self.pragma_int("PRAGMA freelist_count");
        let page_size = self.pragma_int("PRAGMA page_size");
        if page_count.saturating_mul(page_size) < COMPACT_MIN_DB_BYTES {
            return false;
        }
        free_count.saturating_mul(100) >= page_count.saturating_mul(COMPACT_MIN_FREE_PERCENT)
    }

    fn rebuild_vocabulary(&mut self) {
        let mut counts: HashMap<String, i64> = HashMap::new();
        let mut stmt = match self.db.prepare("SELECT path FROM indexed_file") {
            Ok(stmt) => stmt,
            Err(error) => {
                tracing::warn!(error = ?error, "listing indexed paths");
                return;
            }
        };
        loop {
            match stmt.step() {
                Ok(true) => {}
                Ok(false) => break,
                Err(error) => {
                    tracing::warn!(error = ?error, "reading indexed paths");
                    return;
                }
            }
            let Some(path) = stmt.column_text(0) else {
                continue;
            };
            for token in tokenize_filename(basename(&path)) {
                *counts.entry(token).or_insert(0) += 1;
            }
        }

        let tx = match self.db.transaction() {
            Ok(tx) => tx,
            Err(error) => {
                tracing::warn!(error = ?error, "opening the vocabulary rebuild");
                return;
            }
        };
        if let Err(error) = self.db.execute("DELETE FROM vocabulary") {
            tracing::error!(error = ?error, "clearing the vocabulary");
            return;
        }
        let mut insert = match self
            .db
            .prepare("INSERT INTO vocabulary(word, rank) VALUES (:word, :rank)")
        {
            Ok(stmt) => stmt,
            Err(error) => {
                tracing::error!(error = ?error, "preparing the vocabulary insert");
                return;
            }
        };
        for (word, count) in &counts {
            if insert.bind_text(":word", word).is_err()
                || insert.bind_int64(":rank", *count).is_err()
                || exec_once(&mut insert).is_err()
            {
                tracing::error!(word = ?word, "inserting the vocabulary word");
                return;
            }
        }
        if let Err(error) = tx.commit() {
            tracing::error!(error = ?error, "committing the vocabulary rebuild");
        }
    }

    fn index_events(&mut self, events: &[FileEvent]) {
        let tx = match self.db.transaction() {
            Ok(tx) => tx,
            Err(error) => {
                tracing::warn!(error = ?error, "opening the event batch");
                return;
            }
        };
        let mut modify = match self.db.prepare(UPSERT_SQL) {
            Ok(stmt) => stmt,
            Err(error) => {
                tracing::warn!(error = ?error, "preparing the event upsert");
                return;
            }
        };
        let mut delete = match self
            .db
            .prepare("DELETE FROM indexed_file WHERE path = :path")
        {
            Ok(stmt) => stmt,
            Err(error) => {
                tracing::warn!(error = ?error, "preparing the event delete");
                return;
            }
        };
        let mut delete_subtree = match self
            .db
            .prepare("DELETE FROM indexed_file WHERE path >= :lower AND path < :upper")
        {
            Ok(stmt) => stmt,
            Err(error) => {
                tracing::warn!(error = ?error, "preparing the event subtree delete");
                return;
            }
        };
        let mut parents = HashMap::new();
        for event in events {
            let ok = match event.event_type {
                FileEventType::Modify => Self::upsert_file(
                    &self.db,
                    &mut self.mime_ids,
                    &mut parents,
                    &mut modify,
                    &event.path,
                    Some(unix_seconds(event.event_time)),
                    event.is_directory,
                    event.size_bytes,
                ),
                FileEventType::Delete => {
                    let name = event.path.to_string_lossy();
                    let (lower, upper) = subtree_range(&event.path);
                    delete.bind_text(":path", &name).is_ok()
                        && delete_subtree.bind_text(":lower", &lower).is_ok()
                        && delete_subtree.bind_text(":upper", &upper).is_ok()
                        && exec_once(&mut delete).is_ok()
                        && exec_once(&mut delete_subtree).is_ok()
                }
            };
            if !ok {
                tracing::error!(path = ?event.path, "indexing the event");
                return;
            }
        }
        if let Err(error) = tx.commit() {
            tracing::error!(error = ?error, "committing the event batch");
        }
    }
}

/// Inserts a file row, refreshing it when the path is already indexed and
/// stamping `indexed_at` on the refresh.
const UPSERT_SQL: &str = "INSERT INTO indexed_file \
    (path, skeleton_path, parent_id, last_modified_at, type, category, size_bytes, mime_type_id) \
    VALUES (:path, :skeleton_path, :parent_id, :last_modified_at, :type, :category, :size_bytes, :mime_type_id) \
    ON CONFLICT (path) DO UPDATE SET last_modified_at = excluded.last_modified_at, \
    skeleton_path = excluded.skeleton_path, parent_id = excluded.parent_id, type = excluded.type, \
    category = excluded.category, size_bytes = excluded.size_bytes, mime_type_id = excluded.mime_type_id, \
    indexed_at = unixepoch()";

/// One row for the upsert, with everything resolved except the binds.
struct UpsertRow<'a> {
    path: &'a Path,
    skeleton: String,
    parent: Option<i64>,
    modified: Option<i64>,
    is_directory: bool,
    category: IndexedFileCategory,
    size: Option<i64>,
    mime: Option<i64>,
}

/// Binds one upsert row. `None` binds NULL, the way the C++ binds an empty
/// `std::optional`.
fn bind_upsert(
    stmt: &mut Statement<'_>,
    row: &UpsertRow<'_>,
) -> Result<(), compass_sqlcipher_sys::Error> {
    stmt.bind_text(":path", &row.path.to_string_lossy())?;
    stmt.bind_text(":skeleton_path", &row.skeleton)?;
    bind_optional_int(stmt, ":parent_id", row.parent)?;
    bind_optional_int(stmt, ":last_modified_at", row.modified)?;
    stmt.bind_int64(":type", i64::from(row.is_directory))?;
    stmt.bind_int64(":category", row.category as i64)?;
    bind_optional_int(stmt, ":size_bytes", row.size)?;
    bind_optional_int(stmt, ":mime_type_id", row.mime)?;
    Ok(())
}

/// Binds an integer or NULL.
fn bind_optional_int(
    stmt: &mut Statement<'_>,
    name: &str,
    value: Option<i64>,
) -> Result<(), compass_sqlcipher_sys::Error> {
    match value {
        Some(value) => stmt.bind_int64(name, value),
        None => stmt.bind_null(name),
    }
}

/// Runs a statement that returns no rows, leaving it rewound for the next
/// binds. `Ok` unless SQLite reports a failure — the caller's warn carries
/// the context.
fn exec_once(stmt: &mut Statement<'_>) -> Result<(), compass_sqlcipher_sys::Error> {
    let result = stmt.step().map(|_| ());
    stmt.reset();
    result
}

/// The skeleton document for a path: every token of the whole path reduced
/// to its consonant skeleton, empties skipped, joined with spaces.
fn skeleton_document(path: &Path) -> String {
    let native = path.to_string_lossy();
    let mut document = String::with_capacity(native.len());
    for token in tokenize_filename(&native) {
        let skeleton = skeletonize_token(&token);
        if skeleton.is_empty() {
            continue;
        }
        if !document.is_empty() {
            document.push(' ');
        }
        document += &skeleton;
    }
    document
}

/// The `[lower, upper)` byte range covering a path's subtree: `lower` is the
/// path plus `/`, `upper` the path plus `'0'` — the byte after `/` — so every
/// descendant sorts inside and the path itself sorts outside.
fn subtree_range(path: &Path) -> (String, String) {
    let base = path.to_string_lossy();
    (format!("{base}/"), format!("{base}0"))
}

/// The MIME name for a file by extension, or `None` for directories and
/// unknown extensions.
fn mime_name_for(path: &Path, is_directory: bool) -> Option<String> {
    if is_directory {
        return None;
    }
    mime_guess::from_path(path)
        .first()
        .map(|mime| mime.essence_str().to_owned())
}

/// The stored category for a path.
fn indexed_category(path: &Path, is_directory: bool) -> IndexedFileCategory {
    match file_category(path, is_directory) {
        FileCategory::Other => IndexedFileCategory::Other,
        FileCategory::Directory => IndexedFileCategory::Directory,
        FileCategory::Image => IndexedFileCategory::Image,
        FileCategory::Video => IndexedFileCategory::Video,
        FileCategory::Audio => IndexedFileCategory::Audio,
        FileCategory::Document => IndexedFileCategory::Document,
        FileCategory::Archive => IndexedFileCategory::Archive,
        FileCategory::Application => IndexedFileCategory::Application,
    }
}

/// The size in bytes, or `None` for directories and unreadable files.
fn file_size_bytes(metadata: Option<&std::fs::Metadata>, is_directory: bool) -> Option<i64> {
    if is_directory {
        return None;
    }
    let size = metadata?.len();
    Some(i64::try_from(size).unwrap_or(i64::MAX))
}

/// Unix seconds for a timestamp, signed the way the C++ casts the duration.
fn unix_seconds(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(elapsed) => i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX),
        Err(before) => i64::try_from(before.duration().as_secs()).map_or(i64::MIN, |secs| -secs),
    }
}

/// The file-index schema, applied to every fresh database by
/// [`ensure_file_index_schema`] and seeded by the `live_*` fixtures so the
/// two cannot drift apart. Mirrors `INIT_SQL`: `IF NOT EXISTS` throughout,
/// the serving indexes, and the update trigger that keeps the skeleton index
/// honest when a row changes.
pub static WRITER_SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS scan_history (id INTEGER PRIMARY KEY AUTOINCREMENT, \
     status INTEGER NOT NULL, created_at INT DEFAULT (unixepoch()), \
     finished_at INT, entrypoint TEXT NOT NULL, error TEXT, \
     type INT NOT NULL, indexed_file_count INT DEFAULT 0)",
    "CREATE INDEX IF NOT EXISTS scan_history_entrypoint_idx \
     ON scan_history(entrypoint, status, created_at)",
    "CREATE TABLE IF NOT EXISTS indexed_file (id INTEGER PRIMARY KEY AUTOINCREMENT, \
     path TEXT UNIQUE NOT NULL, skeleton_path TEXT NOT NULL, \
     parent_id INT, last_modified_at INT, indexed_at INT NOT NULL DEFAULT (unixepoch()), \
     type INT NOT NULL DEFAULT 0, category INT NOT NULL DEFAULT 0, \
     size_bytes INT, mime_type_id INT)",
    "CREATE TABLE IF NOT EXISTS mime_type (id INTEGER PRIMARY KEY AUTOINCREMENT, \
     name TEXT UNIQUE NOT NULL)",
    "CREATE INDEX IF NOT EXISTS indexed_file_parent_id_idx ON indexed_file(parent_id)",
    "CREATE INDEX IF NOT EXISTS indexed_file_dir_mtime_idx \
     ON indexed_file(last_modified_at DESC) WHERE type = 1",
    "CREATE INDEX IF NOT EXISTS indexed_file_category_idx ON indexed_file(category)",
    "CREATE INDEX IF NOT EXISTS indexed_file_mime_type_idx ON indexed_file(mime_type_id)",
    "CREATE VIRTUAL TABLE IF NOT EXISTS path_idx USING fts5(path, content=indexed_file, \
     tokenize='fuzzy_trigram remove_diacritics 2')",
    "CREATE TRIGGER IF NOT EXISTS path_idx_ai AFTER INSERT ON indexed_file BEGIN \
     INSERT INTO path_idx(rowid, path) VALUES (new.id, new.path); END",
    "CREATE TRIGGER IF NOT EXISTS path_idx_ad AFTER DELETE ON indexed_file BEGIN \
     INSERT INTO path_idx(path_idx, rowid, path) VALUES('delete', old.id, old.path); END",
    "CREATE VIRTUAL TABLE IF NOT EXISTS skeleton_idx USING fts5(skeleton_path, \
     content=indexed_file, \
     tokenize='fuzzy_trigram remove_diacritics 2 skeleton 1 skipgrams 1')",
    "CREATE TRIGGER IF NOT EXISTS skeleton_idx_ai AFTER INSERT ON indexed_file BEGIN \
     INSERT INTO skeleton_idx(rowid, skeleton_path) \
     VALUES (new.id, new.skeleton_path); END",
    "CREATE TRIGGER IF NOT EXISTS skeleton_idx_au AFTER UPDATE ON indexed_file BEGIN \
     INSERT INTO skeleton_idx(skeleton_idx, rowid, skeleton_path) \
     VALUES('delete', old.id, old.skeleton_path); \
     INSERT INTO skeleton_idx(rowid, skeleton_path) \
     VALUES (new.id, new.skeleton_path); END",
    "CREATE TRIGGER IF NOT EXISTS skeleton_idx_ad AFTER DELETE ON indexed_file BEGIN \
     INSERT INTO skeleton_idx(skeleton_idx, rowid, skeleton_path) \
     VALUES('delete', old.id, old.skeleton_path); END",
    "CREATE TABLE IF NOT EXISTS vocabulary (word TEXT PRIMARY KEY, rank INTEGER NOT NULL) \
     WITHOUT ROWID",
];

/// The file-index schema version, stamped as `user_version`.
///
/// A database stamped otherwise is from a breaking change and gets purged,
/// never migrated — it is a cache, rebuilt by rescanning. Version 2 replaced the
/// `spellfix1` virtual table with a plain `vocabulary` table (ADR-0017), which
/// is also why the file is no longer the C++ engine's: see
/// `vicinae::indexer_service::DATABASE_FILE_NAME`.
pub const FILE_INDEX_SCHEMA_VERSION: i64 = 2;

/// Applies [`WRITER_SCHEMA`] and stamps [`FILE_INDEX_SCHEMA_VERSION`].
///
/// Idempotent — rerunning over an existing database changes nothing but the
/// stamp — so startup calls this after purging, never to migrate.
///
/// # Errors
///
/// Returns the first SQLite failure.
pub fn ensure_file_index_schema(db: &Database) -> Result<(), compass_sqlcipher_sys::Error> {
    for statement in WRITER_SCHEMA {
        db.execute(statement)?;
    }
    db.execute(&format!(
        "PRAGMA user_version = {FILE_INDEX_SCHEMA_VERSION}"
    ))
}

/// The stamped schema version of `db`, or 0 when it will not say.
///
/// Drives the purge decision: anything but [`FILE_INDEX_SCHEMA_VERSION`]
/// means a breaking change.
#[must_use]
pub fn file_index_user_version(db: &Database) -> i64 {
    db.query_one_text("PRAGMA user_version")
        .ok()
        .flatten()
        .and_then(|version| version.parse().ok())
        .unwrap_or(0)
}

/// Opens the file index at `path`, purging and recreating it when its stamp
/// is stale.
///
/// Ports the `main.cpp` startup: an unopenable database starts over, a stamp
/// mismatch means a breaking change and gets purged, and a fresh database
/// gets the schema. Never migrates.
///
/// # Errors
///
/// Returns the SQLite failure when the database cannot be opened or the
/// schema cannot be applied.
pub fn prepare_file_index_database(path: &Path) -> Result<(), compass_sqlcipher_sys::Error> {
    let version = Database::open(path, &[]).map(|db| file_index_user_version(&db));
    match version {
        Ok(FILE_INDEX_SCHEMA_VERSION) => return Ok(()),
        Ok(stale) => {
            tracing::info!(stale, "breaking file-index change, starting over");
        }
        Err(error) => {
            tracing::warn!(error = ?error, "file-index database could not be opened, starting over");
        }
    }
    for suffix in ["", "-wal", "-shm"] {
        let mut file = path.as_os_str().to_owned();
        file.push(suffix);
        if std::fs::remove_file(Path::new(&file)).is_ok() {
            tracing::info!(file = ?Path::new(&file), "removed stale file-index file");
        }
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        let _ = std::fs::create_dir_all(parent);
    }
    let db = Database::open(path, &[])?;
    ensure_file_index_schema(&db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_engine::SearchOptions;
    use crate::query_reader::IndexReader;
    use crate::sqlite_reader::SqliteReader;

    #[test]
    fn scan_discriminants_match_the_stored_numbers() {
        assert_eq!(ScanType::Full as i64, 0);
        assert_eq!(ScanType::Incremental as i64, 1);
        assert_eq!(ScanStatus::Pending as i64, 0);
        assert_eq!(ScanStatus::Started as i64, 1);
        assert_eq!(ScanStatus::Interrupted as i64, 2);
        assert_eq!(ScanStatus::Failed as i64, 3);
        assert_eq!(ScanStatus::Succeeded as i64, 4);
    }

    #[test]
    fn statuses_round_trip_with_pending_as_the_fallback() {
        for (number, status) in [
            (0, ScanStatus::Pending),
            (1, ScanStatus::Started),
            (2, ScanStatus::Interrupted),
            (3, ScanStatus::Failed),
            (4, ScanStatus::Succeeded),
        ] {
            assert_eq!(status_from_db(number), status);
        }
        assert_eq!(status_from_db(-1), ScanStatus::Pending);
        assert_eq!(status_from_db(99), ScanStatus::Pending);
    }

    #[test]
    fn subtree_ranges_cover_descendants_only() {
        assert_eq!(
            subtree_range(Path::new("/home/ada/docs")),
            ("/home/ada/docs/".to_owned(), "/home/ada/docs0".to_owned())
        );
    }

    #[test]
    fn skeleton_documents_join_token_skeletons() {
        assert_eq!(
            skeleton_document(Path::new("/home/ada/AnnualReport.txt")),
            "hm ad anl rprt txt"
        );
        assert_eq!(skeleton_document(Path::new("notes.txt")), "nts txt");
    }

    #[test]
    fn mime_names_come_from_the_extension() {
        assert_eq!(
            mime_name_for(Path::new("report.txt"), false).as_deref(),
            Some("text/plain")
        );
        assert_eq!(
            mime_name_for(Path::new("photo.jpg"), false).as_deref(),
            Some("image/jpeg")
        );
        assert_eq!(mime_name_for(Path::new("report.txt"), true), None);
        assert_eq!(
            mime_name_for(Path::new("file.this-extension-does-not-exist-xyz"), false),
            None
        );
    }

    #[test]
    fn categories_follow_the_file() {
        assert_eq!(
            indexed_category(Path::new("photo.jpg"), false),
            IndexedFileCategory::Image
        );
        assert_eq!(
            indexed_category(Path::new("photo.jpg"), true),
            IndexedFileCategory::Directory
        );
    }

    #[test]
    fn unix_seconds_are_signed() {
        assert_eq!(unix_seconds(UNIX_EPOCH), 0);
        assert_eq!(
            unix_seconds(UNIX_EPOCH + std::time::Duration::from_secs(60)),
            60
        );
        assert_eq!(
            unix_seconds(UNIX_EPOCH - std::time::Duration::from_secs(60)),
            -60
        );
    }

    /// Seeds a live database with the shared `WRITER_SCHEMA` above.
    fn live_writer() -> (tempfile::TempDir, SqliteWriter) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("index.db");
        {
            let setup = Database::open(&path, &[]).expect("open");
            for statement in WRITER_SCHEMA {
                setup.execute(statement).expect("schema");
            }
        }
        let writer = SqliteWriter::open(&path).expect("writer");
        (dir, writer)
    }

    fn live_reader(dir: &tempfile::TempDir) -> SqliteReader {
        SqliteReader::open(&dir.path().join("index.db")).expect("reader")
    }

    #[test]
    fn live_prepare_stamps_fresh_and_purges_stale() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("index.db");

        prepare_file_index_database(&path).expect("fresh prepare");
        let version =
            |path: &Path| file_index_user_version(&Database::open(path, &[]).expect("open"));
        assert_eq!(version(&path), FILE_INDEX_SCHEMA_VERSION);

        // Idempotent: a stamped database is left alone.
        prepare_file_index_database(&path).expect("second prepare");
        assert_eq!(version(&path), FILE_INDEX_SCHEMA_VERSION);

        // A breaking stamp purges everything, not just the version.
        let setup = Database::open(&path, &[]).expect("open");
        setup
            .execute("CREATE TABLE marker (id INT)")
            .expect("marker table");
        setup
            .execute("PRAGMA user_version = 99")
            .expect("stale stamp");
        drop(setup);
        prepare_file_index_database(&path).expect("purge prepare");
        assert_eq!(version(&path), FILE_INDEX_SCHEMA_VERSION);
        let check = Database::open(&path, &[]).expect("open");
        assert!(
            check.prepare("SELECT id FROM marker").is_err(),
            "stale tables are gone with the purge"
        );
    }

    fn touch(dir: &tempfile::TempDir, name: &str) -> PathBuf {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parents");
        }
        std::fs::write(&path, "x").expect("file");
        path
    }

    #[test]
    fn live_scan_lifecycle_round_trips() {
        let (_dir, mut writer) = live_writer();
        let record = writer
            .create_scan(Path::new("/home/ada"), ScanType::Full)
            .expect("scan");
        assert!(record.id > 0);
        assert_eq!(record.status, ScanStatus::Pending);
        assert_eq!(record.path, PathBuf::from("/home/ada"));
        assert_eq!(record.scan_type, ScanType::Full);

        assert!(writer.update_scan_status(record.id, ScanStatus::Started));
        assert!(writer.finalize_scan(record.id, ScanStatus::Succeeded, 7));
        assert!(writer.set_scan_error(record.id, "boom"));
    }

    #[test]
    fn live_indexed_files_are_searchable() {
        let (dir, mut writer) = live_writer();
        let report = touch(&dir, "report.txt");
        let photo = touch(&dir, "photo.jpg");
        writer.index_files(std::slice::from_ref(&report));
        writer.index_files(std::slice::from_ref(&photo));

        let reader = live_reader(&dir);
        let hits = reader.search_candidates("\"report\"", 10, &SearchOptions::default());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, report);
        assert_eq!(hits[0].category, IndexedFileCategory::Document);
        assert_eq!(hits[0].mime_type.as_deref(), Some("text/plain"));

        let photos = reader.search_candidates(
            "\"photo\"",
            10,
            &SearchOptions {
                category: Some(IndexedFileCategory::Image),
            },
        );
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].mime_type.as_deref(), Some("image/jpeg"));

        writer.rebuild_vocabulary();
        let suggestions = reader.vocabulary_suggestions("reprot", 20, true);
        assert!(
            suggestions
                .iter()
                .any(|suggestion| suggestion.word == "report")
        );
    }

    #[test]
    fn live_upsert_refreshes_instead_of_duplicating() {
        let (dir, mut writer) = live_writer();
        let report = touch(&dir, "report.txt");
        writer.index_files(std::slice::from_ref(&report));
        writer.index_files(std::slice::from_ref(&report));

        let reader = live_reader(&dir);
        let hits = reader.search_candidates("\"report\"", 10, &SearchOptions::default());
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn live_delete_removes_files_and_subtrees() {
        let (dir, mut writer) = live_writer();
        let keep = touch(&dir, "keep.txt");
        let nested = touch(&dir, "tree/nested.txt");
        writer.index_files(&[keep.clone(), nested.clone()]);

        writer.delete_indexed_files(std::slice::from_ref(&nested));
        let reader = live_reader(&dir);
        assert!(
            reader
                .search_candidates("\"nested\"", 10, &SearchOptions::default())
                .is_empty()
        );
        assert_eq!(
            reader
                .search_candidates("\"keep\"", 10, &SearchOptions::default())
                .len(),
            1
        );

        let tree = dir.path().join("tree");
        writer.delete_indexed_files(std::slice::from_ref(&tree));
        assert!(
            reader
                .search_candidates("\"tree\"", 10, &SearchOptions::default())
                .is_empty()
        );
        assert_eq!(
            reader
                .search_candidates("\"keep\"", 10, &SearchOptions::default())
                .len(),
            1
        );
    }

    #[test]
    fn live_delete_all_clears_the_index() {
        let (dir, mut writer) = live_writer();
        let report = touch(&dir, "report.txt");
        writer.index_files(std::slice::from_ref(&report));
        writer.delete_all_indexed_files();

        let reader = live_reader(&dir);
        assert!(
            reader
                .search_candidates("\"report\"", 10, &SearchOptions::default())
                .is_empty()
        );
    }

    #[test]
    fn live_events_modify_and_delete() {
        let (dir, mut writer) = live_writer();
        let report = touch(&dir, "report.txt");
        writer.index_events(&[FileEvent {
            event_type: FileEventType::Modify,
            path: report.clone(),
            event_time: SystemTime::now(),
            is_directory: false,
            size_bytes: Some(1),
        }]);
        let reader = live_reader(&dir);
        assert_eq!(
            reader
                .search_candidates("\"report\"", 10, &SearchOptions::default())
                .len(),
            1
        );

        writer.index_events(&[FileEvent {
            event_type: FileEventType::Delete,
            path: report,
            event_time: SystemTime::now(),
            is_directory: false,
            size_bytes: None,
        }]);
        assert!(
            reader
                .search_candidates("\"report\"", 10, &SearchOptions::default())
                .is_empty()
        );
    }

    #[test]
    fn live_small_database_needs_no_compaction() {
        let (_dir, writer) = live_writer();
        assert!(!writer.needs_compaction());
    }
}
