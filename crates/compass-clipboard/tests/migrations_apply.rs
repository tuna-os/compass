//! The migrations, run against real encrypted databases.
//!
//! The unit tests in `schema` check the table of migrations. These check what
//! happens when it meets SQLite: that the schema the C++ engine's SQL describes
//! actually comes into being, that running twice is a no-op, and that the two
//! refusals fire.

use compass_clipboard::schema::{self, Error, MIGRATIONS};
use compass_sqlcipher_sys::Database;

const KEY: &[u8] = &[0x21; 32];

fn scratch() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("clipboard.db");
    (dir, path)
}

fn tables(db: &Database) -> Vec<String> {
    let mut stmt = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .expect("prepare");
    let mut names = Vec::new();
    while stmt.step().expect("step") {
        if let Some(name) = stmt.column_text(0) {
            names.push(name);
        }
    }
    names
}

#[test]
fn a_fresh_database_gets_the_whole_schema() {
    let (_dir, path) = scratch();
    let db = Database::open(&path, KEY).expect("open");
    schema::run(&db).expect("migrations");

    let names = tables(&db);
    for expected in [
        "selection",
        "data_offer",
        "selection_fts",
        "schema_migrations",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "{expected} is missing; got {names:?}"
        );
    }

    // 002 replaces the porter-tokenized FTS table from 001 with a
    // fuzzy_trigram one. If only 001 ran, this query still works but the
    // tokenizer is wrong, so assert on the schema text rather than on the
    // table merely existing.
    let sql = db
        .query_one_text("SELECT sql FROM sqlite_master WHERE name = 'selection_fts'")
        .expect("query")
        .expect("a row");
    assert!(
        sql.contains("fuzzy_trigram"),
        "selection_fts was not migrated to the trigram tokenizer: {sql}"
    );
    assert!(
        !sql.contains("porter"),
        "selection_fts still has 001's porter tokenizer: {sql}"
    );
}

#[test]
fn every_migration_is_recorded_once() {
    let (_dir, path) = scratch();
    let db = Database::open(&path, KEY).expect("open");
    schema::run(&db).expect("migrations");

    let mut stmt = db
        .prepare("SELECT id, version, checksum, applied_at FROM schema_migrations ORDER BY version")
        .expect("prepare");
    let mut rows = Vec::new();
    while stmt.step().expect("step") {
        rows.push((
            stmt.column_text(0).unwrap_or_default(),
            stmt.column_int64(1),
            stmt.column_text(2).unwrap_or_default(),
            stmt.column_int64(3),
        ));
    }

    assert_eq!(rows.len(), MIGRATIONS.len());
    for (row, migration) in rows.iter().zip(MIGRATIONS) {
        assert_eq!(row.0, migration.id);
        assert_eq!(row.1, migration.version);
        assert_eq!(row.2, schema::checksum(migration.sql));
        assert!(row.3 > 0, "applied_at was not set");
    }
}

#[test]
fn running_twice_changes_nothing() {
    let (_dir, path) = scratch();
    let db = Database::open(&path, KEY).expect("open");
    schema::run(&db).expect("first run");

    let before = tables(&db);
    schema::run(&db).expect("second run must be a no-op, not an error");
    assert_eq!(tables(&db), before);

    let count = db
        .query_one_text("SELECT count(*) FROM schema_migrations")
        .expect("query")
        .expect("a row");
    assert_eq!(
        count,
        MIGRATIONS.len().to_string(),
        "the second run recorded the migrations again"
    );
}

#[test]
fn the_schema_survives_being_closed_and_reopened() {
    // Migrations run inside a transaction; if it were never committed the
    // schema would be present in this connection and gone from the file.
    let (_dir, path) = scratch();
    {
        let db = Database::open(&path, KEY).expect("open");
        schema::run(&db).expect("migrations");
    }
    let db = Database::open(&path, KEY).expect("reopen");
    assert!(tables(&db).iter().any(|n| n == "selection"));
}

#[test]
fn an_edited_migration_is_refused() {
    let (_dir, path) = scratch();
    let db = Database::open(&path, KEY).expect("open");
    schema::run(&db).expect("migrations");

    // Rewrite a recorded checksum to stand for "the file changed after it was
    // applied". The C++ engine stores this column and never reads it, so this
    // is the divergence made visible.
    let mut stmt = db
        .prepare("UPDATE schema_migrations SET checksum = :c WHERE id = :id")
        .expect("prepare");
    stmt.bind_text(":c", "00000000000000000000000000000000")
        .expect("bind");
    stmt.bind_text(":id", MIGRATIONS[0].id).expect("bind");
    stmt.step().expect("update");

    let err = schema::run(&db).expect_err("a changed migration must be refused");
    match err {
        Error::ChecksumMismatch { id, applied, found } => {
            assert_eq!(id, MIGRATIONS[0].id);
            assert_eq!(applied, "00000000000000000000000000000000");
            assert_eq!(found, schema::checksum(MIGRATIONS[0].sql));
        }
        other => panic!("expected ChecksumMismatch, got {other:?}"),
    }
}

#[test]
fn a_database_from_a_newer_build_is_refused() {
    let (_dir, path) = scratch();
    let db = Database::open(&path, KEY).expect("open");
    schema::run(&db).expect("migrations");

    // A migration this build has never heard of, as a newer Compass would
    // leave behind. Applying ours on top would produce a schema neither build
    // expects, so the answer is to refuse rather than to migrate.
    let mut stmt = db
        .prepare(
            "INSERT INTO schema_migrations (id, applied_at, version, checksum) \
             VALUES ('003_from_the_future.sql', 1, 3, 'deadbeef')",
        )
        .expect("prepare");
    stmt.step().expect("insert");

    let err = schema::run(&db).expect_err("a newer database must be refused");
    match err {
        Error::Diverged {
            applied, position, ..
        } => {
            assert_eq!(applied, "003_from_the_future.sql");
            assert_eq!(position, MIGRATIONS.len());
        }
        other => panic!("expected Diverged, got {other:?}"),
    }
}

#[test]
fn the_migrated_schema_accepts_what_the_clipboard_writes() {
    // The point of the schema is that the engine's own inserts work against
    // it. A migration that created the wrong columns would pass every test
    // above and fail here.
    let (_dir, path) = scratch();
    let db = Database::open(&path, KEY).expect("open");
    schema::run(&db).expect("migrations");

    db.execute(
        "INSERT INTO selection (id, hash_md5, preferred_mime_type, offer_count, created_at, \
         updated_at, kind) VALUES ('s1', 'abc', 'text/plain', 1, 100, 100, 1)",
    )
    .expect("insert a selection");
    db.execute(
        "INSERT INTO data_offer (id, selection_id, mime_type, text_preview, content_hash_md5, \
         size, encryption_type, kind) \
         VALUES ('o1', 's1', 'text/plain', 'hello', 'abc', 5, 0, 1)",
    )
    .expect("insert an offer");
    db.execute("INSERT INTO selection_fts (selection_id, content) VALUES ('s1', 'hello world')")
        .expect("index the content");

    let hit = db
        .query_one_text(
            "SELECT selection_id FROM selection_fts WHERE selection_fts MATCH '\"wor\"'",
        )
        .expect("query")
        .expect("a row");
    assert_eq!(hit, "s1");

    // The foreign key with ON DELETE CASCADE, which PRAGMA foreign_keys = ON
    // makes live. Without the pragma the offer would survive its selection.
    db.execute("DELETE FROM selection WHERE id = 's1'")
        .expect("delete");
    let offers = db
        .query_one_text("SELECT count(*) FROM data_offer")
        .expect("query")
        .expect("a row");
    assert_eq!(
        offers, "0",
        "deleting a selection must cascade to its offers"
    );
}
