//! The key-derivation constants are upstream Vicinae's, pinned.
//!
//! WHY THIS IS NOT PARANOIA
//!
//! `keys.rs` carries four strings and a size that decide where the master key
//! lives and what it expands to. A database or clipboard history upstream
//! Vicinae encrypted opens here only if every one of them matches, and the
//! failure mode is not a red test: it is a user's history decrypting to
//! nothing, or in upstream `database-key.cpp`'s own words:
//!
//!   "A database is encrypted but its key is missing from the keychain. [...]
//!    otherwise the affected database files must be deleted to reset."
//!
//! Until the C++ engine was removed (ADR-0021) these were parsed out of the
//! in-tree sources. They are now written down here, as they stand in upstream
//! v0.29.0 (`vicinaehq/vicinae@c3415a3`, the release `scripts/bench` pins):
//! `APP_ID` in `src/server/src/vicinae.hpp`, `KEY_NAME` and the two
//! `deriveKey` labels in `src/server/src/internal/db/database-key.cpp`, and
//! `KEY_SIZE` in `src/lib/crypto/include/crypto/aes-gcm.hpp`. They are data
//! formats, not branding: renaming the product must not rename them.
//!
//! WHAT IT CANNOT DO
//!
//! It pins text, not semantics. A change to what `derive_key` does with a label
//! is `crypto-parity`'s to catch, by cross-decrypting with upstream's crypto.

use compass_crypto::keys;

#[test]
fn the_hkdf_labels_are_upstreams() {
    assert_eq!(
        keys::DATABASE_LABEL,
        "vicinae-db",
        "a different label derives a different key from the same master, so a database \
         upstream encrypted would not open"
    );
    assert_eq!(
        keys::CLIPBOARD_LABEL,
        "vicinae-clipboard",
        "a different label derives a different key from the same master, so a clipboard \
         history upstream encrypted would not decrypt"
    );
}

#[test]
fn the_keyring_entry_is_where_upstream_stores_it() {
    assert_eq!(
        keys::KEYCHAIN_SERVICE,
        "vicinae",
        "upstream opens the keyring as this service"
    );
    assert_eq!(
        keys::MASTER_KEY_NAME,
        "vicinae-master-key",
        "looking in the wrong place is indistinguishable from the key being absent, which \
         upstream treats as fatal and unrecoverable"
    );
}

#[test]
fn the_master_key_size_is_upstreams() {
    assert_eq!(
        keys::MASTER_KEY_SIZE,
        32,
        "upstream's AES-256-GCM key is 32 bytes"
    );
}
