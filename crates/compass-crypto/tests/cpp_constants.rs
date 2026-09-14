//! The key-derivation constants are read back out of the C++ sources.
//!
//! WHY THIS IS NOT PARANOIA
//!
//! `keys.rs` carries four strings copied from the C++ engine: the keyring
//! service, the keyring entry name, and two HKDF labels. Copied constants are
//! correct on the day they are copied and silently wrong afterwards, and the
//! failure mode here is not a red test — it is a user's clipboard history
//! decrypting to nothing, or `database-key.cpp`'s own words:
//!
//!   "A database is encrypted but its key is missing from the keychain. [...]
//!    otherwise the affected database files must be deleted to reset."
//!
//! So the C++ sources are parsed and compared. This is the same move as
//! `compass-shell`'s introspection check: a document that nobody reads drifts,
//! and the fix is to make something read it.
//!
//! WHAT IT CANNOT DO
//!
//! It reads text, not semantics. If someone changes `deriveKey`'s meaning
//! without changing its label the comparison still passes — that is what
//! `crypto-parity`'s cross-engine derivation is for. And if the C++ tree is
//! deleted at Phase 8, this test goes with it; by then the constants are
//! whatever Compass says they are, which is the point of the migration.

use std::path::{Path, PathBuf};

use compass_crypto::keys;

/// Repository root, from this crate's manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/compass-crypto sits two levels below the repository root")
        .to_path_buf()
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "cannot read {}: {err}. If the C++ tree has moved or been deleted, this test should \
             move or be deleted with it rather than be pointed somewhere else.",
            path.display()
        )
    })
}

/// Every string literal that follows `marker` in `text`.
///
/// Deliberately simple: it finds the marker, then takes what is between the
/// next pair of double quotes. A C++ parser would be a worse trade here — the
/// point is to notice a changed constant, and anything clever enough to be
/// fooled by a comment is also clever enough to have bugs of its own.
fn literals_after(text: &str, marker: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(marker) {
        rest = &rest[at + marker.len()..];
        let Some(open) = rest.find('"') else { break };
        let after_open = &rest[open + 1..];
        let Some(close) = after_open.find('"') else {
            break;
        };
        out.push(after_open[..close].to_owned());
        rest = &after_open[close + 1..];
    }
    out
}

#[test]
fn the_hkdf_labels_match_the_cpp_engine() {
    let source = read("src/server/src/internal/db/database-key.cpp");
    let labels = literals_after(&source, "Crypto::deriveKey(*master,");

    assert_eq!(
        labels.len(),
        2,
        "expected exactly two deriveKey calls in database-key.cpp, found {}: {labels:?}. A third \
         purpose means compass-crypto::keys needs a third subkey, not that this test should be \
         relaxed.",
        labels.len()
    );
    assert_eq!(
        labels[0],
        keys::DATABASE_LABEL,
        "the C++ engine derives its database key with {:?}; compass-crypto uses {:?}. These \
         produce different keys from the same master, so an existing encrypted database would \
         not open.",
        labels[0],
        keys::DATABASE_LABEL
    );
    assert_eq!(
        labels[1],
        keys::CLIPBOARD_LABEL,
        "the C++ engine derives its clipboard key with {:?}; compass-crypto uses {:?}. These \
         produce different keys from the same master, so an existing clipboard history would \
         not decrypt.",
        labels[1],
        keys::CLIPBOARD_LABEL
    );
}

#[test]
fn the_keyring_entry_name_matches_the_cpp_engine() {
    let source = read("src/server/src/internal/db/database-key.cpp");
    let names = literals_after(&source, "KEY_NAME =");

    assert_eq!(
        names.len(),
        1,
        "expected one KEY_NAME definition in database-key.cpp, found {names:?}"
    );
    assert_eq!(
        names[0],
        keys::MASTER_KEY_NAME,
        "the C++ engine stores its master key under {:?} and compass-crypto looks for {:?}. \
         Looking in the wrong place is indistinguishable from the key being absent, which \
         database-key.cpp treats as fatal and unrecoverable.",
        names[0],
        keys::MASTER_KEY_NAME
    );
}

#[test]
fn the_keyring_service_matches_the_cpp_engine() {
    let source = read("src/server/src/vicinae.hpp");
    let ids = literals_after(&source, "APP_ID =");

    assert_eq!(
        ids.len(),
        1,
        "expected one APP_ID definition in vicinae.hpp, found {ids:?}"
    );
    assert_eq!(
        ids[0],
        keys::KEYCHAIN_SERVICE,
        "the C++ engine opens the keyring as service {:?}; compass-crypto uses {:?}",
        ids[0],
        keys::KEYCHAIN_SERVICE
    );
}

#[test]
fn the_master_key_size_matches_the_cpp_engine() {
    // db::KEY_SIZE is Crypto::AES256GCM::KEY_SIZE; assert the source of truth
    // rather than the alias, since that is what `readKey` compares against.
    let source = read("src/lib/crypto/include/crypto/aes-gcm.hpp");
    let declared = source
        .split("KEY_SIZE =")
        .nth(1)
        .and_then(|rest| rest.split(';').next())
        .map(str::trim)
        .and_then(|value| value.parse::<usize>().ok())
        .expect("aes-gcm.hpp should declare `KEY_SIZE = <n>;`");

    assert_eq!(
        declared,
        keys::MASTER_KEY_SIZE,
        "the C++ engine's key is {declared} bytes and compass-crypto expects {}",
        keys::MASTER_KEY_SIZE
    );
}

/// The extractor is shown to work and to fail, on text with the shapes the
/// real sources have — a comment mentioning the marker, and a second call.
///
/// Without this, every assertion above could be passing because
/// `literals_after` returns something that happens to match, or because a
/// silent parse failure yields an empty vector that some future edit compares
/// leniently.
#[test]
fn control_the_extractor_finds_what_is_there_and_only_that() {
    let sample = concat!(
        "// Crypto::deriveKey(*master, \"not-a-real-call\") in a comment\n",
        "auto databaseKey = Crypto::deriveKey(*master, \"first-label\");\n",
        "auto clipboardKey = Crypto::deriveKey(*master, \"second-label\");\n"
    );
    assert_eq!(
        literals_after(sample, "Crypto::deriveKey(*master,"),
        ["not-a-real-call", "first-label", "second-label"],
        "the extractor is deliberately literal and does see commented-out calls; the real test \
         asserts a COUNT of two, which is what would catch a comment like this appearing in \
         database-key.cpp"
    );

    assert!(
        literals_after(sample, "NoSuchMarker").is_empty(),
        "a marker that is not present must yield nothing, not a stray literal"
    );
    assert!(
        literals_after("KEY_NAME = ", "KEY_NAME =").is_empty(),
        "a marker with no following literal must yield nothing rather than panicking"
    );
}

/// The keyring storage format is a property of **qtkeychain v0.14.0**.
///
/// `keyring.rs` documents four attributes and one non-obvious encoding — the
/// secret is base64, not raw bytes — all read out of that tag's
/// `libsecret.cpp`. None of it is in qtkeychain's documentation, and none of
/// it is guaranteed across versions: the schema, the attribute names, the
/// `type` values and the base64 wrapping are all internal choices.
///
/// This machine has no keyring daemon, so the format cannot be re-verified
/// here on demand. What *can* be checked, offline and in a second, is that the
/// version whose source was read is still the version being built. If the pin
/// moves, someone has to go and look again.
#[test]
fn the_qtkeychain_pin_still_matches_the_version_the_format_was_read_from() {
    /// The tag whose `libsecret.cpp` was read to write `keyring.rs`.
    const VERIFIED_AGAINST: &str = "v0.14.0";

    let cmake = read("cmake/QtKeychain.cmake");

    // GIT_TAG's argument is a bare token in CMake, so take the next
    // whitespace-delimited word and strip any quotes rather than assuming
    // either form. An earlier version of this test asked `literals_after`
    // whether the tag was quoted, which cannot work: that helper finds the
    // next quote ANYWHERE in the file, and this one has several further down.
    let tag = cmake
        .split("GIT_TAG")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .map(|token| token.trim_matches('"'))
        .expect("cmake/QtKeychain.cmake should declare a GIT_TAG");

    assert_eq!(
        tag, VERIFIED_AGAINST,
        "qtkeychain is pinned to {tag}, but compass-crypto::keyring's storage format was read \
         out of {VERIFIED_AGAINST}'s libsecret.cpp. The attribute names, the `type` values and \
         the base64 wrapping of the secret are all internal to qtkeychain and can change \
         between releases. Re-read libsecret.cpp at {tag} and confirm — or correct — the \
         contract in keyring.rs before moving this constant."
    );
}
