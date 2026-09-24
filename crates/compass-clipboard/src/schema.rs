//! The clipboard schema, and applying it.
//!
//! The migration mechanism itself lives in [`compass_db`] -- the
//! `schema_migrations` table, the MD5 checksums, the one-transaction run and
//! the two divergences from the C++ `MigrationManager` are documented there,
//! because the `vicinae` namespace needs exactly the same thing. What is
//! specific to the clipboard is the list, and the pins that keep it honest.
//!
//! The C++ loads these from `:database/...`, a Qt **resource** path, so they
//! are already compiled into the binary rather than read from disk;
//! [`MIGRATIONS`] embeds the same files with `include_str!` for the same
//! reason.

pub use compass_db::{Error, Migration, checksum, run as run_migrations};

use compass_sqlcipher_sys::rusqlite::Connection;

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

/// Bring `db` up to the current clipboard schema.
///
/// Idempotent, so it is safe to call on every open, as `ClipboardDatabase`'s
/// constructor path does.
///
/// # Errors
///
/// See [`compass_db::run`].
pub fn run(db: &Connection) -> Result<(), Error> {
    run_migrations(db, MIGRATIONS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_migration_list_is_well_formed() {
        // Versions that match the file names, in ascending order, no
        // duplicates. `compass_db` owns the rule; this list has to obey it.
        compass_db::assert_well_formed(MIGRATIONS);
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
}
