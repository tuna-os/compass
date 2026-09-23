//! Compile SQLCipher, the `fuzzy_trigram` tokenizer, and the `spellfix1`
//! virtual table from `vendor/`.
//!
//! The flags mirror `vendor/sqlcipher/CMakeLists.txt` exactly, because the C++
//! engine and this crate have to produce and read the same files. A flag that
//! differs here is not a build difference, it is a format difference, and
//! nothing would report it until a user's history failed to open.

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
    let sqlcipher = vendor.join("sqlcipher/sqlite3.c");
    let tokenizer = vendor.join("fuzzy-trigram/register.c");
    let spellfix = vendor.join("spellfix/register.c");

    for file in [&sqlcipher, &tokenizer, &spellfix] {
        assert!(
            file.exists(),
            "{} is missing. This crate builds the vendored C that defines the clipboard file \
             format; it cannot fall back to a system SQLite, because a system SQLite is not \
             SQLCipher and does not have this tokenizer.",
            file.display()
        );
        println!("cargo:rerun-if-changed={}", file.display());
    }

    // SQLCipher. Flags from vendor/sqlcipher/CMakeLists.txt.
    let mut build = cc::Build::new();
    build
        .file(&sqlcipher)
        .include(&vendor)
        .warnings(false)
        .define("SQLITE_ENABLE_FTS5", None)
        .define("SQLITE_TEMP_STORE", "2")
        .define("SQLITE_HAS_CODEC", None)
        .define("SQLITE_EXTRA_INIT", "sqlcipher_extra_init")
        .define("SQLITE_EXTRA_SHUTDOWN", "sqlcipher_extra_shutdown");

    // The crypto provider is chosen per platform, and choosing wrong writes a
    // database the other engine cannot read (ADR-0014).
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos") | Ok("ios") => {
            build.define("SQLCIPHER_CRYPTO_CC", None);
            println!("cargo:rustc-link-lib=framework=Security");
            println!("cargo:rustc-link-lib=framework=CoreFoundation");
        }
        Ok("windows") => {
            build
                .file(vendor.join("sqlcipher/crypto_cng.c"))
                .define("SQLCIPHER_CRYPTO_CUSTOM", "sqlcipher_cng_setup");
            println!("cargo:rustc-link-lib=bcrypt");
            println!("cargo:rustc-link-lib=version");
        }
        _ => {
            build.define("SQLCIPHER_CRYPTO_OPENSSL", None);
            println!("cargo:rustc-link-lib=crypto");
        }
    }
    build.compile("compass_sqlcipher");

    // The tokenizer, statically linked the same way the C++ engine links it.
    // `register.c` includes `sqlite3.h` (via `fuzzy-trigram.h`), which lives
    // in the SQLCipher amalgamation directory — not on the default search
    // path, so without this include nothing that links this crate compiles
    // on a fresh checkout.
    cc::Build::new()
        .file(&tokenizer)
        .include(&vendor)
        .include(vendor.join("sqlcipher"))
        .include(vendor.join("fuzzy-trigram"))
        .warnings(false)
        .define("SQLITE_CORE", "1")
        .compile("compass_fuzzy_trigram");

    // The spellfix1 vocabulary, statically linked the same way: the file
    // indexer keeps its typo-correction vocabulary in `spellfix_vocab`, and
    // without this module every access to that table fails — including the
    // reads in `compass-db`'s `SqliteReader`.
    cc::Build::new()
        .file(&spellfix)
        .include(&vendor)
        .include(vendor.join("sqlcipher"))
        .include(vendor.join("spellfix"))
        .warnings(false)
        .define("SQLITE_CORE", "1")
        .compile("compass_spellfix");
}
