//! The `vicinae` database's schema.
//!
//! The same three files the C++ engine applies, embedded rather than read from
//! disk because the C++ loads them from `:database/...`, a Qt resource path.
//! It lives here rather than in one of its readers because the `vicinae`
//! database is shared: local storage, the OAuth token store, the root item
//! manager and the calculator history are all tables in it, and each of them
//! has to apply the same list.

use compass_sqlcipher_sys::Database;

use crate::{Error, Migration};

/// Every `vicinae` migration, in the order they must be applied.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        id: "001_init.sql",
        version: 1,
        sql: include_str!("../../../src/server/database/vicinae/migrations/001_init.sql"),
    },
    Migration {
        id: "002_add_recent_files.sql",
        version: 2,
        sql: include_str!(
            "../../../src/server/database/vicinae/migrations/002_add_recent_files.sql"
        ),
    },
    Migration {
        id: "003_add_oauth_token_store.sql",
        version: 3,
        sql: include_str!(
            "../../../src/server/database/vicinae/migrations/003_add_oauth_token_store.sql"
        ),
    },
];

/// Bring `db` up to the current `vicinae` schema.
///
/// # Errors
///
/// See [`crate::run`].
pub fn run(db: &Database) -> Result<(), Error> {
    crate::run(db, MIGRATIONS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checksum;

    #[test]
    fn the_migration_list_is_well_formed() {
        crate::assert_well_formed(MIGRATIONS);
    }

    #[test]
    fn the_embedded_content_hashes_to_what_the_cpp_engine_recorded() {
        // `md5sum` over src/server/database/vicinae/migrations/. Pinned so that
        // editing an applied migration fails here, at development time, rather
        // than against a user's existing database -- where the runtime check
        // would compare two copies that had moved together.
        for (migration, expected) in MIGRATIONS.iter().zip([
            "64461b5a228f582d8dd6c5c22beff07d",
            "0d5f3e3d43e9151cf8a76f8855358763",
            "c738bb86d0c90b465b5b3ef72ba8f7cc",
        ]) {
            assert_eq!(
                checksum(migration.sql),
                expected,
                "{} changed. An applied migration cannot be edited -- add a new one.",
                migration.id
            );
        }
    }

    #[test]
    fn the_storage_table_is_created_with_the_key_the_cpp_uses() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let db = Database::open(&dir.path().join("vicinae.db"), &[]).expect("an unencrypted db");
        run(&db).expect("the migrations apply");

        // The primary key is (namespace_id, key): two extensions may use the
        // same key, and `set` relies on the conflict target being exactly this.
        db.execute(
            "INSERT INTO storage_data_item (namespace_id, value_type, key, value) \
             VALUES ('a:data', 1, 'k', 'v')",
        )
        .expect("a first row");
        db.execute(
            "INSERT INTO storage_data_item (namespace_id, value_type, key, value) \
             VALUES ('b:data', 1, 'k', 'v')",
        )
        .expect("the same key in another namespace");
        db.execute(
            "INSERT INTO storage_data_item (namespace_id, value_type, key, value) \
             VALUES ('a:data', 1, 'k', 'w')",
        )
        .expect_err("the same key in the same namespace conflicts");
    }
}
