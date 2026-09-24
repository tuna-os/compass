//! Compile the `fuzzy_trigram` tokenizer from `vendor/`.
//!
//! SQLCipher itself comes from `libsqlite3-sys`'s `bundled-sqlcipher` build,
//! through `rusqlite`. The tokenizer is compiled here against *that* build's
//! headers (`DEP_SQLITE3_INCLUDE`, exported by `libsqlite3-sys`), not the
//! copy in `vendor/sqlcipher` the C++ engine uses, so that the structures it
//! hands SQLite match the SQLite it is linked with.
//!
//! `spellfix1` is no longer built: the file index's typo vocabulary is a plain
//! table with suggestions computed in Rust (`compass_db::vocabulary`, ADR-0017).

use std::path::{Path, PathBuf};

fn vendor_dir() -> PathBuf {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets this"));
    manifest
        .parent()
        .and_then(Path::parent)
        .expect("crates/compass-sqlcipher-sys sits two levels below the repository root")
        .join("vendor")
}

fn main() {
    let vendor = vendor_dir();
    let tokenizer = vendor.join("fuzzy-trigram/register.c");
    assert!(
        tokenizer.exists(),
        "{} is missing. Every clipboard history declares this tokenizer on its FTS table, \
         and without it no query against that table runs.",
        tokenizer.display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        vendor.join("fuzzy-trigram").display()
    );

    let sqlite_include = std::env::var("DEP_SQLITE3_INCLUDE").expect(
        "libsqlite3-sys exports DEP_SQLITE3_INCLUDE when it builds its bundled SQLCipher; \
         without it the tokenizer would compile against headers of some other SQLite",
    );

    // Statically linked the same way the C++ engine links it. `register.c`
    // includes `sqlite3.h` (via `fuzzy-trigram.h`).
    cc::Build::new()
        .file(&tokenizer)
        .include(&sqlite_include)
        .include(vendor.join("fuzzy-trigram"))
        .warnings(false)
        .define("SQLITE_CORE", "1")
        .compile("compass_fuzzy_trigram");
}
