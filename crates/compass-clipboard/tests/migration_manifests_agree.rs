//! Every migration file on disk is in `MIGRATIONS`, in order, and nothing else is.
//!
//! WHY THIS IS A REAL RISK AND NOT A HYPOTHETICAL
//!
//! `MIGRATIONS` lists its `.sql` files by hand and `include_str!`s each one. So
//! adding `004_something.sql` beside the others and forgetting the list is not a
//! build error: the file is simply never applied, and every database this engine
//! opens -- including one upstream Vicinae created, which is what the schema
//! compat promise covers -- stays on the old schema without a word.
//!
//! Until the C++ engine was removed (ADR-0021) this compared the list with the
//! Qt resource manifest that engine compiled in. The directory is now the only
//! other place the set is written down, so the list is compared with that.

use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<name> sits two levels below the root")
        .to_path_buf()
}

/// The `.sql` files in a migrations directory, sorted: the order the C++
/// engine applied them in, and the order their version prefixes give.
fn directory_migrations(relative: &str) -> Vec<String> {
    let dir = repo().join(relative);
    let mut out: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.ends_with(".sql"))
        .collect();
    out.sort();

    assert!(
        !out.is_empty(),
        "no .sql files in {} -- the directory moved and this test would now \
         pass by finding nothing",
        dir.display()
    );
    out
}

fn assert_agree(what: &str, dir: &str, rust: &[compass_db::Migration]) {
    let on_disk = directory_migrations(dir);
    let from_rust: Vec<String> = rust.iter().map(|m| m.id.to_owned()).collect();

    assert_eq!(
        on_disk, from_rust,
        "the {what} migrations disagree.\n  \
         {dir} holds: {on_disk:?}\n  \
         Rust MIGRATIONS declares: {from_rust:?}\n\
         A file missing from the list is never applied, and a database opened \
         by this engine would silently stay on the old schema."
    );
}

#[test]
fn the_clipboard_manifests_agree() {
    assert_agree(
        "clipboard",
        "crates/compass-clipboard/migrations",
        compass_clipboard::schema::MIGRATIONS,
    );
}

#[test]
fn the_vicinae_manifests_agree() {
    assert_agree(
        "vicinae",
        "crates/compass-db/migrations/vicinae",
        compass_db::vicinae::MIGRATIONS,
    );
}

/// The versions must be 1..=n with no gaps, in the order the list gives them.
///
/// Separate from the agreement above because the list can match the directory
/// and still be wrong — a skipped version would apply cleanly here and leave a
/// database upstream's `MigrationManager` would try to re-migrate.
#[test]
fn the_versions_are_dense_and_ordered() {
    for (what, migrations) in [
        ("clipboard", compass_clipboard::schema::MIGRATIONS),
        ("vicinae", compass_db::vicinae::MIGRATIONS),
    ] {
        for (index, migration) in migrations.iter().enumerate() {
            let expected = i64::try_from(index + 1).expect("few migrations");
            assert_eq!(
                migration.version, expected,
                "{what} migration {} is version {}, expected {expected}. \
                 Versions are applied in sequence and compared against what a \
                 database has already seen, so a gap or a reorder is a \
                 divergence even when both manifests agree on the files.",
                migration.id, migration.version
            );
        }
    }
}
