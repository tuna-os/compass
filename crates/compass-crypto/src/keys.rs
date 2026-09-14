//! Master key and the per-purpose subkeys derived from it.
//!
//! Port of `src/server/src/internal/db/database-key.cpp`.
//!
//! # Why these constants matter more than the code around them
//!
//! One master key lives in the login keyring. Everything the engine encrypts
//! is derived from it by HKDF with a purpose label: the SQLCipher database
//! under one label, the clipboard history under another. The labels and the
//! keyring entry name are therefore a **data format**, not an implementation
//! detail.
//!
//! Get a label wrong and the Rust engine derives a different key from the same
//! master, so an existing clipboard history decrypts to nothing. Get the
//! keyring entry name wrong and it does not find the master key at all, and
//! `database-key.cpp` is explicit about what happens then:
//!
//! > A database is encrypted but its key is missing from the keychain. \[…\]
//! > otherwise the affected database files must be deleted to reset.
//!
//! That is a user losing their clipboard history to a typo. So these constants
//! are not retyped and trusted — `tests/cpp_constants.rs` reads them back out
//! of the C++ sources and fails if either side moves.
//!
//! # What is deliberately not here yet
//!
//! Reading the master key from the keyring. That needs a Secret Service
//! backend and a running daemon to test against, and it is a separable
//! problem: the derivation below is where the irreversible mistake lives, and
//! it is testable today against the real C++ implementation. The keyring
//! lookup is the next increment, not this one.

use crate::{KEY_SIZE, SUBKEY_SIZE, derive_key};

/// Keyring service name. C++ `Omnicast::APP_ID` in `vicinae.hpp`.
pub const KEYCHAIN_SERVICE: &str = "vicinae";

/// Keyring entry holding the master key. C++ `KEY_NAME` in `database-key.cpp`.
pub const MASTER_KEY_NAME: &str = "vicinae-master-key";

/// HKDF label for the SQLCipher database key.
pub const DATABASE_LABEL: &str = "vicinae-db";

/// HKDF label for the clipboard history key.
pub const CLIPBOARD_LABEL: &str = "vicinae-clipboard";

/// The subkeys a master key expands to.
///
/// Both are always derived together, as `prepareEncryption` does, because the
/// interesting property is that one master gives exactly these two and that
/// they differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedKeys {
    /// Key for the SQLCipher database.
    pub database: [u8; SUBKEY_SIZE],
    /// Key for the clipboard history.
    pub clipboard: [u8; SUBKEY_SIZE],
}

/// Expands a master key into its per-purpose subkeys.
///
/// The master key is [`KEY_SIZE`] bytes in practice, but HKDF accepts any
/// input length and the C++ signature takes a span, so this does not impose a
/// length it does not need. [`MASTER_KEY_SIZE`] is what the keyring entry is
/// checked against on read.
pub fn derive_all(master: &[u8]) -> DerivedKeys {
    DerivedKeys {
        database: derive_key(master, DATABASE_LABEL),
        clipboard: derive_key(master, CLIPBOARD_LABEL),
    }
}

/// Size the keyring entry must be, matching C++ `db::KEY_SIZE`.
///
/// `readKey` rejects a stored blob of any other length rather than deriving
/// from it, and so should we: a short entry is a corrupted keyring, not a
/// weaker key to carry on with.
pub const MASTER_KEY_SIZE: usize = KEY_SIZE;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_subkeys_differ() {
        let keys = derive_all(&[7u8; MASTER_KEY_SIZE]);
        assert_ne!(
            keys.database, keys.clipboard,
            "both purposes derived the same key, which defeats the point of \
             labelling them: a clipboard compromise would hand over the database too"
        );
    }

    #[test]
    fn derivation_is_deterministic() {
        let master = [3u8; MASTER_KEY_SIZE];
        assert_eq!(derive_all(&master), derive_all(&master));
    }

    #[test]
    fn a_different_master_gives_different_subkeys() {
        let a = derive_all(&[1u8; MASTER_KEY_SIZE]);
        let b = derive_all(&[2u8; MASTER_KEY_SIZE]);
        assert_ne!(a.database, b.database);
        assert_ne!(a.clipboard, b.clipboard);
    }

    /// The subkeys are exactly `derive_key` with those labels, so a future
    /// refactor cannot quietly add a salt or a second expansion round on one
    /// path only.
    #[test]
    fn subkeys_are_plain_hkdf_with_the_documented_labels() {
        let master = [9u8; MASTER_KEY_SIZE];
        let keys = derive_all(&master);
        assert_eq!(keys.database, derive_key(&master, DATABASE_LABEL));
        assert_eq!(keys.clipboard, derive_key(&master, CLIPBOARD_LABEL));
    }
}
