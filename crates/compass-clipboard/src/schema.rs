//! The clipboard schema, and applying it.
//!
//! Ports `MigrationManager` (`src/server/src/utils/migration-manager/`) for the
//! `clipboard` namespace. The C++ loads its migrations from `:database/...`,
//! a Qt **resource** path, so they are already compiled into the binary rather
//! than read from disk; [`MIGRATIONS`] embeds the same files with `include_str!`
//! for the same reason.
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
//! exception, logs it, rolls back, and returns `void`; `ClipboardDatabase::
//! runMigrations` returns `void` too. A failed migration is therefore silent,
//! and the next thing the user sees is every query failing against a schema
//! that was never created. [`run`] returns a `Result`.
//!
//! **It checks the checksum.** The C++ computes it, stores it, reads it back
//! into a struct field, and never compares it to anything — the column exists to
//! detect a migration edited after it was applied, and nothing performs that
//! detection. Here a mismatch is an error, because a stored value with no
//! reader is not a check.

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

/// Every clipboard migration, in the order they must be applied.
///
/// The versions come from the file names, exactly as the C++ regex
/// `(\d+)_.*\.sql` extracts them. They are written out here rather than parsed
/// at runtime because there is no directory to scan: the list is fixed at
/// build time, and a new migration is a source change either way.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        id: "001_init.sql",
        version: 1,
        sql: include_str!("../../../src/server/database/clipboard/migrations/001_init.sql"),
    },
    Migration {
        id: "002_trigram_fts.sql",
        version: 2,
        sql: include_str!("../../../src/server/database/clipboard/migrations/002_trigram_fts.sql"),
    },
];

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

/// Bring `db` up to the current schema.
///
/// Idempotent: applying twice is a no-op, which is what makes it safe to call
/// on every open, as `ClipboardDatabase`'s constructor path does.
///
/// # Errors
///
/// Returns [`Error::Diverged`] if the database records migrations this build
/// does not have, [`Error::ChecksumMismatch`] if an applied migration's content
/// has changed, and [`Error::Database`] if SQLite refuses. On any of them
/// nothing is committed.
pub fn run(db: &Database) -> Result<(), Error> {
    db.execute(SCHEMA_MIGRATIONS)?;

    let already = applied(db)?;

    // Compare position by position. A database with more migrations than this
    // build has is newer than this build, which is a refusal rather than
    // something to migrate "up" to.
    for (position, (id, recorded)) in already.iter().enumerate() {
        let Some(ours) = MIGRATIONS.get(position) else {
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

    if already.len() == MIGRATIONS.len() {
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

    for migration in &MIGRATIONS[already.len()..] {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_versions_match_the_file_names() {
        // The C++ parses these out with `(\d+)_.*\.sql`; this crate writes them
        // down. The two must not drift, and a typo here would apply migrations
        // in the wrong order.
        for migration in MIGRATIONS {
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
    }

    #[test]
    fn the_migrations_are_in_ascending_version_order() {
        let versions: Vec<i64> = MIGRATIONS.iter().map(|m| m.version).collect();
        let mut sorted = versions.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            versions, sorted,
            "MIGRATIONS is out of order or has a duplicate"
        );
    }

    #[test]
    fn the_embedded_content_hashes_to_what_the_cpp_engine_recorded() {
        // These are `md5sum` over the files in
        // src/server/database/clipboard/migrations/. Pinning them here is what
        // turns the runtime checksum check from a tautology into a check: the
        // runtime comparison is embedded-content against database-content, and
        // both sides would move together if a file were edited. This one does
        // not move, so editing a migration fails here, at development time,
        // rather than on a user's machine.
        assert_eq!(
            checksum(MIGRATIONS[0].sql),
            "9a7178012077f4e05be300eb9b73ae83",
            "001_init.sql changed. An applied migration cannot be edited -- add a new one."
        );
        assert_eq!(
            checksum(MIGRATIONS[1].sql),
            "1046384a016297b859067e4cd482898d",
            "002_trigram_fts.sql changed. An applied migration cannot be edited -- add a new one."
        );
    }

    #[test]
    fn the_checksum_is_md5_hex() {
        // A known vector, so a swapped hash function is caught by something
        // other than the pins above moving together with the code.
        assert_eq!(checksum(""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(checksum("abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(checksum("abc").len(), 32);
    }
}
