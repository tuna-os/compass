//! How the master key is actually stored in the login keyring.
//!
//! # Why this file is mostly constants and a comment
//!
//! The C++ engine reaches the keyring through **qtkeychain v0.14.0**, pinned in
//! `cmake/QtKeychain.cmake`, which on Linux goes through libsecret. Compass has
//! to find the *same* entry, and "the same entry" turns out to mean four
//! specific attributes and one non-obvious encoding. None of it is documented
//! anywhere; it was read out of qtkeychain's `libsecret.cpp` at that tag.
//!
//! Get it wrong and nothing errors: the lookup simply misses, which
//! `database-key.cpp` reports as the key being absent, which it treats as
//! fatal and tells the user to delete their database files over.
//!
//! # The contract
//!
//! ```text
//! attribute  user    = "vicinae-master-key"   (KEY_NAME)
//! attribute  server  = "vicinae"              (APP_ID)
//! attribute  type    = "base64"               (because the write is binary)
//! attribute  xdg:schema = "org.qt.keychain"   (added by libsecret itself)
//! secret     = base64 text of the 32 raw key bytes
//! ```
//!
//! **The secret is base64 text, not bytes.** `database-key.cpp` calls
//! `setBinaryData`, and qtkeychain's binary mode does `password.toBase64()`
//! before storing and `QByteArray::fromBase64` after reading. A port that
//! stored 32 raw bytes would write an entry the C++ engine reads as base64,
//! decodes to garbage, and rejects for having the wrong length — if it is
//! lucky. This is the single most likely way to get this wrong.
//!
//! **The lookup is two-step.** `LibSecretKeyring::findPassword` first searches
//! with `type="plaintext"`; only when that finds nothing does it retry with
//! `type="base64"` and decode. So an entry written as text shadows one written
//! as binary, and Compass should write `base64` to match what the C++ engine
//! writes rather than what it looks for first.
//!
//! # What is not here, and why
//!
//! The Secret Service client itself. This machine has no
//! `gnome-keyring-daemon`, no `libsecret` and no `secret-tool`, so a D-Bus
//! client written here could only be tested against a mock written here —
//! which would prove the two agree with each other and nothing about
//! interoperating with the real thing. That is the weak evidence this
//! project's test suite exists to avoid.
//!
//! The format above is the part that is hard to get right and easy to verify;
//! the D-Bus plumbing is mechanical once it is known. Doing the client belongs
//! with an environment that has a real keyring daemon to test against — the VM
//! tier already boots a full GNOME session and is the obvious home for it.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use crate::keys::{KEYCHAIN_SERVICE, MASTER_KEY_NAME, MASTER_KEY_SIZE};

/// libsecret schema qtkeychain registers. Written by libsecret as the
/// `xdg:schema` attribute.
pub const SCHEMA_NAME: &str = "org.qt.keychain";

/// Attribute holding qtkeychain's "key" — the entry name within a service.
pub const ATTR_USER: &str = "user";

/// Attribute holding qtkeychain's "service".
pub const ATTR_SERVER: &str = "server";

/// Attribute recording how the secret is encoded.
pub const ATTR_TYPE: &str = "type";

/// `type` value for an entry written with `setBinaryData`, whose secret is
/// base64. This is what the master key uses.
pub const TYPE_BINARY: &str = "base64";

/// `type` value for an entry written as text, whose secret is verbatim.
/// Searched first by qtkeychain's reader; Compass does not write it.
pub const TYPE_TEXT: &str = "plaintext";

/// The attributes identifying the master key entry, in libsecret's terms.
///
/// `xdg:schema` is deliberately absent: libsecret adds it on store from the
/// schema passed in, so a client supplying it explicitly is describing an
/// implementation detail of the library rather than of this contract.
pub fn master_key_attributes() -> [(&'static str, &'static str); 3] {
    [
        (ATTR_USER, MASTER_KEY_NAME),
        (ATTR_SERVER, KEYCHAIN_SERVICE),
        (ATTR_TYPE, TYPE_BINARY),
    ]
}

/// Why a stored secret could not be turned back into a master key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    /// The stored text is not valid base64.
    #[error(
        "the keyring entry is not valid base64; it was not written by a binary-mode qtkeychain job"
    )]
    NotBase64,
    /// It decoded, but not to a key-sized blob.
    ///
    /// `readKey` rejects this rather than padding or truncating, and so does
    /// this: a short entry is a corrupted keyring, not a weaker key to carry
    /// on with.
    #[error("the keyring entry decodes to {found} bytes, not {MASTER_KEY_SIZE}")]
    WrongSize {
        /// What the entry actually decoded to.
        found: usize,
    },
}

/// Encodes a master key the way qtkeychain's binary mode does.
pub fn encode_secret(raw: &[u8]) -> String {
    STANDARD.encode(raw)
}

/// Decodes a stored secret and checks it is a master key.
pub fn master_key_from_secret(text: &str) -> Result<[u8; MASTER_KEY_SIZE], SecretError> {
    let bytes = STANDARD
        .decode(text.trim())
        .map_err(|_| SecretError::NotBase64)?;
    <[u8; MASTER_KEY_SIZE]>::try_from(bytes.as_slice())
        .map_err(|_| SecretError::WrongSize { found: bytes.len() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_master_key_round_trips_through_the_stored_form() {
        let key = crate::generate_key().expect("system randomness");
        let stored = encode_secret(&key);
        assert_eq!(master_key_from_secret(&stored), Ok(key));
    }

    /// Qt's `toBase64()` default is the standard alphabet with padding and no
    /// line breaks. Pinned against a hand-checked vector so a change of engine
    /// configuration — URL-safe alphabet, or padding dropped — is caught.
    #[test]
    fn the_encoding_is_standard_padded_base64() {
        assert_eq!(
            encode_secret(&[0u8; MASTER_KEY_SIZE]),
            "A".to_owned() + &"A".repeat(42) + "="
        );
        assert_eq!(encode_secret(b"\xff\xfe\xfd"), "//79");
        assert_eq!(
            encode_secret(b"\xff\xfe"),
            "//4=",
            "padding must be present; QByteArray::toBase64 emits it by default"
        );
    }

    #[test]
    fn a_non_base64_entry_is_rejected() {
        assert_eq!(
            master_key_from_secret("not base64 !!"),
            Err(SecretError::NotBase64)
        );
    }

    #[test]
    fn a_wrong_sized_entry_is_rejected_rather_than_adjusted() {
        let short = encode_secret(&[1u8; 16]);
        assert_eq!(
            master_key_from_secret(&short),
            Err(SecretError::WrongSize { found: 16 })
        );
        let long = encode_secret(&[1u8; 64]);
        assert_eq!(
            master_key_from_secret(&long),
            Err(SecretError::WrongSize { found: 64 })
        );
    }

    /// The trap this module exists for: raw bytes are not the stored form.
    ///
    /// If someone "simplifies" `encode_secret` to hand over the key directly,
    /// the entry it writes is one the C++ engine cannot read. This asserts the
    /// two forms are different, so that simplification fails a test instead of
    /// a user's migration.
    #[test]
    fn the_stored_form_is_not_the_raw_bytes() {
        let key = [0x41u8; MASTER_KEY_SIZE]; // 'A' repeated, valid UTF-8
        let stored = encode_secret(&key);
        assert_ne!(
            stored.as_bytes(),
            &key[..],
            "the secret must be base64 of the key, never the key itself: qtkeychain's binary \
             mode base64-decodes whatever it reads, so raw bytes come back as garbage"
        );
        assert_eq!(master_key_from_secret(&stored), Ok(key));
    }

    #[test]
    fn the_attributes_are_the_ones_qtkeychain_searches_for() {
        let attrs = master_key_attributes();
        assert_eq!(attrs[0], ("user", "vicinae-master-key"));
        assert_eq!(attrs[1], ("server", "vicinae"));
        assert_eq!(
            attrs[2],
            ("type", "base64"),
            "the master key is written with setBinaryData, so its type attribute is base64; \
             writing plaintext would shadow the real entry in qtkeychain's two-step lookup"
        );
    }
}
