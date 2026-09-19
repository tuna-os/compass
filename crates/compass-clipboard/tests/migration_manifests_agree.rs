//! The two engines must apply the SAME migrations to the same database.
//!
//! WHY THIS IS A REAL RISK AND NOT A HYPOTHETICAL
//!
//! Both engines read the same `.sql` files, but they *enumerate* them
//! differently and neither knows about the other's list:
//!
//!   * C++ compiles them in as Qt resources, listed by hand in
//!     `src/server/database/{clipboard,vicinae}/migrations.qrc`.
//!   * Rust `include_str!`s them, listed by hand in `MIGRATIONS`.
//!
//! So adding `004_something.sql` means editing two files in two languages, and
//! nothing fails if you edit one. The result would not be a build error — it
//! would be **two engines applying different schemas to the same SQLite file**,
//! which is precisely what §8.1's schema-compat item is about:
//!
//!     Schema compat: open the same SQLite file with both engines, in both
//!     orders; assert no corruption and no lost rows.
//!
//! That check needs both binaries and a VM image. THIS one needs neither: a
//! divergence between the two manifests is visible from the source alone, and
//! catching it here is cheaper and earlier than catching it in a VM.
//!
//! It does not replace schema compat. It rules out the most likely way the two
//! could disagree, on every PR, in milliseconds.

use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<name> sits two levels below the root")
        .to_path_buf()
}

/// The migration filenames a `.qrc` declares, in declaration order.
///
/// Parsed rather than pattern-matched loosely: the order matters as much as
/// the set, because migrations are applied in sequence and a reordering would
/// be as wrong as an omission while passing any set-comparison.
fn qrc_migrations(relative: &str) -> Vec<String> {
    let path = repo().join(relative);
    let xml = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

    let mut out = Vec::new();
    for line in xml.lines() {
        let Some(rest) = line.split_once("<file>") else {
            continue;
        };
        let Some((entry, _)) = rest.1.split_once("</file>") else {
            continue;
        };
        // The qrc lists `migrations/001_init.sql`; MIGRATIONS ids are the bare
        // filename. Compare the part that identifies the migration.
        let name = entry.rsplit('/').next().unwrap_or(entry);
        out.push(name.to_owned());
    }

    assert!(
        !out.is_empty(),
        "parsed no <file> entries out of {} — the qrc format changed and this \
         test would now pass by finding nothing",
        path.display()
    );
    out
}

fn assert_agree(what: &str, qrc_path: &str, rust: &[compass_db::Migration]) {
    let from_qrc = qrc_migrations(qrc_path);
    let from_rust: Vec<String> = rust.iter().map(|m| m.id.to_owned()).collect();

    assert_eq!(
        from_qrc, from_rust,
        "the {what} migration manifests disagree.\n  \
         {qrc_path} declares: {from_qrc:?}\n  \
         Rust MIGRATIONS declares: {from_rust:?}\n\
         Both engines open the same SQLite file. Applying different migration \
         sets to it is the schema divergence §8.1 exists to prevent, and it \
         would not show up as a build failure in either language."
    );
}

#[test]
fn the_clipboard_manifests_agree() {
    assert_agree(
        "clipboard",
        "src/server/database/clipboard/migrations.qrc",
        compass_clipboard::schema::MIGRATIONS,
    );
}

#[test]
fn the_vicinae_manifests_agree() {
    assert_agree(
        "vicinae",
        "src/server/database/vicinae/migrations.qrc",
        compass_db::vicinae::MIGRATIONS,
    );
}

/// The versions must be 1..=n with no gaps, in the order the list gives them.
///
/// Separate from the agreement above because two manifests can match each
/// other and still both be wrong — a skipped version would apply cleanly here
/// and leave a database the C++ `MigrationManager` would try to re-migrate.
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
