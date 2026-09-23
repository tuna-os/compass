//! Schema migrations, shared by every Compass database.
//!
//! Ports `MigrationManager` (`src/server/src/utils/migration-manager/`). The
//! C++ loads its migrations from `:database/...`, a Qt **resource** path, so
//! they are compiled into the binary rather than read from disk; the crates
//! that use this embed theirs with `include_str!` for the same reason, and
//! hand them here as a [`Migration`] slice.
//!
//! # The contract, which is on disk and cannot be changed
//!
//! A database already written by the C++ engine carries a `schema_migrations`
//! table with one row per applied migration:
//!
//! ```sql
//! CREATE TABLE IF NOT EXISTS schema_migrations (
//!   id TEXT PRIMARY KEY,
//!   applied_at INTEGER NOT NULL,
//!   version INTEGER,
//!   checksum TEXT NOT NULL
//! );
//! ```
//!
//! `id` is the file name, `version` the leading integer in it, and `checksum`
//! the **MD5** of the file content, hex encoded. MD5 is not a security choice
//! here and is not used as one: it is what is already in the column, and a port
//! that wrote SHA-256 there would make every existing row unreadable to the
//! other engine.
//!
//! # Two things this does that the C++ does not
//!
//! Both are [declared divergences](../../../docs/rust-engine/PARITY.md), not
//! accidents.
//!
//! **It returns errors.** `MigrationManager::runMigrations` catches every
//! exception, logs it, rolls back, and returns `void`. A failed migration is
//! therefore silent, and the next thing the user sees is every query failing
//! against a schema that was never created. [`run`] returns a `Result`.
//!
//! **It checks the checksum.** The C++ computes it, stores it, reads it back
//! into a struct field, and never compares it to anything — the column exists
//! to detect a migration edited after it was applied, and nothing performs that
//! detection. Here a mismatch is an error, because a stored value with no
//! reader is not a check.

<<<<<<< HEAD
pub mod db_writer;
pub mod query_engine;
>>>>>>> 948652a0f (feat: port query-engine scoring over compass-search)
pub mod query_policy;
pub mod vicinae;

use std::time::{SystemTime, UNIX_EPOCH};

use compass_sqlcipher_sys::Database;
use md5::{Digest as _, Md5};

/// A migration, embedded at build time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Migration {
    /// The file name, which is the `id` column.
    pub id: &'static str,
    /// The leading integer in the file name.
    pub version: i64,
    /// The SQL, verbatim.
    pub sql: &'static str,
}

/// The table the C++ engine creates to record what it has applied.
const SCHEMA_MIGRATIONS: &str = "\
CREATE TABLE IF NOT EXISTS schema_migrations (
  id TEXT PRIMARY KEY,
  applied_at INTEGER NOT NULL,
  version INTEGER,
  checksum TEXT NOT NULL
);";

/// Something wrong with the schema or with applying it.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The database refused something.
    #[error(transparent)]
    Database(#[from] compass_sqlcipher_sys::Error),

    /// The database records a migration this build does not have, or records
    /// them in a different order.
    ///
    /// Almost always means the database was written by a newer build. Applying
    /// anything on top would produce a schema neither build expects.
    #[error(
        "this database has migration {applied} at position {position}, but this build has \
         {expected} there. It was probably written by a newer Compass."
    )]
    Diverged {
        /// The `id` recorded in the database.
        applied: String,
        /// The `id` this build expects at that position.
        expected: String,
        /// Where they disagree, counting from zero.
        position: usize,
    },

    /// An applied migration's content no longer matches what was applied.
    ///
    /// The C++ engine stores this checksum and never compares it, so this error
    /// has no counterpart there — see the module docs.
    #[error(
        "migration {id} was applied with checksum {applied}, but this build's copy hashes to \
         {found}. The file was edited after it was applied, so the schema on disk is not the one \
         this build describes."
    )]
    ChecksumMismatch {
        /// Which migration.
        id: String,
        /// What the database recorded.
        applied: String,
        /// What this build's copy hashes to.
        found: String,
    },
}

/// The MD5 of `sql`, hex encoded — the form the `checksum` column holds.
#[must_use]
pub fn checksum(sql: &str) -> String {
    let digest = Md5::digest(sql.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(out, "{byte:02x}").expect("writing to a String cannot fail");
    }
    out
}

/// What `db` has already applied, in version order.
fn applied(db: &Database) -> Result<Vec<(String, String)>, Error> {
    let mut stmt = db.prepare("SELECT id, checksum FROM schema_migrations ORDER BY version")?;
    let mut rows = Vec::new();
    while stmt.step()? {
        rows.push((
            stmt.column_text(0).unwrap_or_default(),
            stmt.column_text(1).unwrap_or_default(),
        ));
    }
    Ok(rows)
}

/// Bring `db` up to the schema `migrations` describes.
///
/// Idempotent: applying twice is a no-op, which is what makes it safe to call
/// on every open, as the C++ constructors do.
///
/// # Errors
///
/// Returns [`Error::Diverged`] if the database records migrations this build
/// does not have, [`Error::ChecksumMismatch`] if an applied migration's content
/// has changed, and [`Error::Database`] if SQLite refuses. On any of them
/// nothing is committed.
pub fn run(db: &Database, migrations: &[Migration]) -> Result<(), Error> {
    db.execute(SCHEMA_MIGRATIONS)?;

    let already = applied(db)?;

    // Compare position by position. A database with more migrations than this
    // build has is newer than this build, which is a refusal rather than
    // something to migrate "up" to.
    for (position, (id, recorded)) in already.iter().enumerate() {
        let Some(ours) = migrations.get(position) else {
            return Err(Error::Diverged {
                applied: id.clone(),
                expected: "nothing — this build has no migration at that position".to_owned(),
                position,
            });
        };
        if ours.id != id {
            return Err(Error::Diverged {
                applied: id.clone(),
                expected: ours.id.to_owned(),
                position,
            });
        }
        let found = checksum(ours.sql);
        if &found != recorded {
            return Err(Error::ChecksumMismatch {
                id: id.clone(),
                applied: recorded.clone(),
                found,
            });
        }
    }

    if already.len() == migrations.len() {
        return Ok(());
    }

    // One transaction for the whole run, as the C++ does: a half-migrated
    // schema is worse than an unmigrated one.
    let tx = db.transaction()?;
    let now = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
    .unwrap_or(i64::MAX);

    for migration in &migrations[already.len()..] {
        db.execute(migration.sql)?;

        let mut stmt = db.prepare(
            "INSERT INTO schema_migrations (id, applied_at, version, checksum) \
             VALUES (:id, :applied_at, :version, :checksum)",
        )?;
        stmt.bind_text(":id", migration.id)?;
        stmt.bind_int64(":applied_at", now)?;
        stmt.bind_int64(":version", migration.version)?;
        stmt.bind_text(":checksum", &checksum(migration.sql))?;
        stmt.step()?;
    }

    tx.commit()?;
    Ok(())
}

/// Checks that hold for any migration list, so every caller gets them.
///
/// Called from each crate's own tests with its own list: a version that does
/// not match its file name applies migrations in the wrong order, and a
/// duplicate applies one twice.
///
/// # Panics
///
/// With a message naming the offending migration.
pub fn assert_well_formed(migrations: &[Migration]) {
    assert!(
        !migrations.is_empty(),
        "an empty migration list would make every check here vacuous"
    );

    for migration in migrations {
        let leading: String = migration
            .id
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        assert_eq!(
            leading.parse::<i64>().ok(),
            Some(migration.version),
            "{} does not declare the version its name says",
            migration.id
        );
    }

    let versions: Vec<i64> = migrations.iter().map(|m| m.version).collect();
    let mut sorted = versions.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        versions, sorted,
        "the migration list is out of order or has a duplicate"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checksum_is_md5_hex() {
        // A known vector, so a swapped hash function is caught by something
        // other than a crate's pinned checksums moving together with its files.
        assert_eq!(checksum(""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(checksum("abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(checksum("abc").len(), 32);
    }

    const ONE: &[Migration] = &[Migration {
        id: "001_init.sql",
        version: 1,
        sql: "CREATE TABLE t (a TEXT);",
    }];

    const TWO: &[Migration] = &[
        ONE[0],
        Migration {
            id: "002_more.sql",
            version: 2,
            sql: "CREATE TABLE u (b TEXT);",
        },
    ];

    fn open() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let db = Database::open(&dir.path().join("test.db"), &[]).expect("an unencrypted database");
        (dir, db)
    }

    #[test]
    fn running_twice_applies_nothing_the_second_time() {
        let (_dir, db) = open();
        run(&db, TWO).expect("first run");
        run(&db, TWO).expect("second run");

        let mut stmt = db
            .prepare("SELECT count(*) FROM schema_migrations")
            .expect("count");
        assert!(stmt.step().expect("a row"));
        assert_eq!(stmt.column_int64(0), 2, "a migration was applied twice");
    }

    #[test]
    fn a_database_from_a_newer_build_is_refused_rather_than_migrated() {
        let (_dir, db) = open();
        run(&db, TWO).expect("applying two migrations");

        match run(&db, ONE) {
            Err(Error::Diverged { position, .. }) => assert_eq!(position, 1),
            other => panic!("a newer database must be refused, got {other:?}"),
        }
    }

    #[test]
    fn an_edited_migration_is_refused() {
        let (_dir, db) = open();
        run(&db, ONE).expect("applying one migration");

        const EDITED: &[Migration] = &[Migration {
            id: "001_init.sql",
            version: 1,
            sql: "CREATE TABLE t (a TEXT, b TEXT);",
        }];
        match run(&db, EDITED) {
            Err(Error::ChecksumMismatch { id, .. }) => assert_eq!(id, "001_init.sql"),
            other => panic!("an edited migration must be refused, got {other:?}"),
        }
    }

    #[test]
    fn a_later_migration_applies_on_top_of_an_earlier_one() {
        let (_dir, db) = open();
        run(&db, ONE).expect("applying one migration");
        run(&db, TWO).expect("applying the second on top");

        db.execute("INSERT INTO u (b) VALUES ('x')")
            .expect("the second migration's table exists");
    }

    #[test]
    fn a_failing_migration_commits_nothing() {
        let (_dir, db) = open();
        const BROKEN: &[Migration] = &[
            ONE[0],
            Migration {
                id: "002_broken.sql",
                version: 2,
                sql: "CREATE TABLE ;",
            },
        ];
        run(&db, BROKEN).expect_err("invalid SQL is refused");

        // Not even the first migration's row survives: the C++ runs the whole
        // set in one transaction, and a half-migrated schema is worse than an
        // unmigrated one.
        let mut stmt = db
            .prepare("SELECT count(*) FROM schema_migrations")
            .expect("count");
        assert!(stmt.step().expect("a row"));
        assert_eq!(stmt.column_int64(0), 0, "a failed run left rows behind");
    }

    #[test]
    fn well_formedness_rejects_what_it_should() {
        assert_well_formed(TWO);

        let out_of_order = &[TWO[1], TWO[0]];
        assert!(
            std::panic::catch_unwind(|| assert_well_formed(out_of_order)).is_err(),
            "an out-of-order list must be rejected"
        );

        let mislabelled = &[Migration {
            id: "007_init.sql",
            version: 1,
            sql: "",
        }];
        assert!(
            std::panic::catch_unwind(|| assert_well_formed(mislabelled)).is_err(),
            "a version that disagrees with the file name must be rejected"
        );
    }
}
