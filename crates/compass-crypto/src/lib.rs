//! Clipboard-at-rest crypto: the Rust side of `vicinae::crypto`.
//!
//! Port of `src/lib/crypto` (`aes-gcm.cpp`, `kdf.cpp`). The C++ header comment
//! explains what it is for: encrypting things that live outside SQLCipher, of
//! which the clipboard history is the one that matters.
//!
//! # The wire format is the contract
//!
//! Phase 3's gate asks that the clipboard store be "readable and writable by
//! both engines interchangeably". That makes the on-disk layout, not the API,
//! the thing that has to match. Read out of `aes-gcm.cpp` rather than assumed:
//!
//! ```text
//! [ iv (12 bytes) | ciphertext (n bytes) | tag (16 bytes) ]
//! ```
//!
//! AES-256-GCM, no associated data, IV from the system CSPRNG, tag appended.
//! `deriveKey` is HKDF-SHA256 with an empty salt and the label as `info`.
//!
//! `crates/compass-testkit` diffs this against the real C++ implementation on
//! every PR — see [`docs/rust-engine/PLAN.md`] §8.4a for why that harness
//! cross-decrypts instead of comparing ciphertexts.
//!
//! # Why RustCrypto and not the `openssl` crate
//!
//! This deserves stating because the C++ side chose the other way, on purpose.
//! Its CMakeLists says it uses the platform backend "so that we can benefit
//! from system security updates to these critical libraries", which is a good
//! argument — for a binary installed from a distro package.
//!
//! It transfers only partly to us. The shipping artefact is a Flatpak, and a
//! Flatpak's OpenSSL comes from its runtime, on the runtime's update cadence,
//! not the host's. So the choice is between two vendored-ish supply chains
//! rather than between "the distro's" and "ours", and the pure-Rust one costs
//! no C toolchain in the build and keeps `unsafe_code = "forbid"` meaningful
//! across the code we write.
//!
//! This is reversible, and the parity harness is what makes it safely so: swap
//! the backend and cross-decryption against the C++ engine still has to pass,
//! so a change of implementation cannot quietly change the format.

pub mod keys;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;

/// Size of an AES-256 key, in bytes. `Crypto::AES256GCM::KEY_SIZE`.
pub const KEY_SIZE: usize = 32;

/// Size of a derived subkey, in bytes. `Crypto::SUBKEY_SIZE`.
pub const SUBKEY_SIZE: usize = 32;

/// GCM initialisation vector length used by the format, in bytes.
///
/// Not configurable: it is what the C++ writer emits and the C++ reader
/// expects, so changing it breaks every clipboard row already on disk.
pub const IV_SIZE: usize = 12;

/// GCM authentication tag length used by the format, in bytes.
pub const TAG_SIZE: usize = 16;

/// Why a decryption did not produce plaintext.
///
/// The variants mirror C++ `Crypto::AES256GCM::DecryptError` one for one, and
/// the parity harness asserts the *specific* variant rather than "it failed" —
/// a decrypt that ignored the GCM tag would still "fail" on plenty of inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DecryptError {
    /// The key was not [`KEY_SIZE`] bytes.
    #[error("key must be {KEY_SIZE} bytes")]
    InvalidKeySize,
    /// The buffer cannot even hold an IV and a tag, so it is not a blob this
    /// format ever produced.
    #[error("encrypted buffer is shorter than the {IV_SIZE}-byte IV plus {TAG_SIZE}-byte tag")]
    DataTooShort,
    /// The tag did not verify: wrong key, or the data was altered.
    #[error("authentication failed: wrong key, or the ciphertext was modified")]
    AuthFailed,
}

/// Why an encryption failed.
///
/// One variant, because `Crypto::AES256GCM::EncryptError` also has one. That
/// is arguably a flaw in the C++ API — a wrong-sized key and a failed cipher
/// are different mistakes, and `encrypt` cannot say which — but the two error
/// sets are compared by name in the parity harness, so inventing a more
/// precise variant here would show up as a divergence rather than as an
/// improvement. Fixing it means changing both sides together.
///
/// Note the asymmetry this leaves in the C++ API, faithfully reproduced:
/// `decrypt` *can* report [`DecryptError::InvalidKeySize`], `encrypt` cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EncryptError {
    /// The key was the wrong size, the system CSPRNG was unavailable, or the
    /// cipher refused. `aes-gcm.cpp` folds all three into this one value.
    #[error("encryption failed: bad key size, no system randomness, or the cipher refused")]
    CipherError,
}

/// A fresh 256-bit key from the system CSPRNG.
///
/// Fallible, like C++ `generateKey`, whose comment is worth keeping: "nullopt
/// when the system random generator is unavailable; never returns a weak key".
/// A key generator that silently substituted a fallback source on failure
/// would be the worst possible bug in this file, so the failure is in the
/// signature.
pub fn generate_key() -> Result<[u8; KEY_SIZE], NoRandomness> {
    let mut out = [0u8; KEY_SIZE];
    getrandom::fill(&mut out).map_err(|_| NoRandomness)?;
    Ok(out)
}

/// The system CSPRNG could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the system random number generator is unavailable")]
pub struct NoRandomness;

/// Derives a purpose-specific subkey from `master` via HKDF-SHA256.
///
/// Distinct labels give independent keys and the same `(master, label)` is
/// deterministic — the one part of this module that can be compared to the C++
/// implementation byte for byte, and the harness does.
///
/// The salt is empty, matching `kdf.cpp`, which sets `info` and leaves the salt
/// unset.
pub fn derive_key(master: &[u8], label: &str) -> [u8; SUBKEY_SIZE] {
    let hkdf = Hkdf::<Sha256>::new(None, master);
    let mut out = [0u8; SUBKEY_SIZE];
    hkdf.expand(label.as_bytes(), &mut out)
        .expect("32 bytes is far below HKDF-SHA256's 255*32 output limit");
    out
}

/// Encrypts `data`, returning `[iv | ciphertext | tag]`.
pub fn encrypt(data: &[u8], key: &[u8]) -> Result<Vec<u8>, EncryptError> {
    if key.len() != KEY_SIZE {
        return Err(EncryptError::CipherError);
    }
    let key = Key::<Aes256Gcm>::try_from(key).map_err(|_| EncryptError::CipherError)?;
    let cipher = Aes256Gcm::new(&key);

    let mut iv = [0u8; IV_SIZE];
    getrandom::fill(&mut iv).map_err(|_| EncryptError::CipherError)?;
    let nonce = Nonce::try_from(iv.as_slice()).map_err(|_| EncryptError::CipherError)?;

    let sealed = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: data,
                aad: &[],
            },
        )
        .map_err(|_| EncryptError::CipherError)?;

    // `sealed` is ciphertext||tag, so prefixing the nonce gives exactly the
    // C++ layout with no further arithmetic.
    let mut out = Vec::with_capacity(IV_SIZE + sealed.len());
    out.extend_from_slice(&iv);
    out.extend_from_slice(&sealed);
    Ok(out)
}

/// Decrypts a `[iv | ciphertext | tag]` buffer.
pub fn decrypt(encrypted: &[u8], key: &[u8]) -> Result<Vec<u8>, DecryptError> {
    if key.len() != KEY_SIZE {
        return Err(DecryptError::InvalidKeySize);
    }
    if encrypted.len() < IV_SIZE + TAG_SIZE {
        return Err(DecryptError::DataTooShort);
    }

    let (iv, rest) = encrypted.split_at(IV_SIZE);
    let key = Key::<Aes256Gcm>::try_from(key).map_err(|_| DecryptError::InvalidKeySize)?;
    let cipher = Aes256Gcm::new(&key);
    let nonce = Nonce::try_from(iv).map_err(|_| DecryptError::DataTooShort)?;

    cipher
        .decrypt(
            &nonce,
            Payload {
                msg: rest,
                aad: &[],
            },
        )
        .map_err(|_| DecryptError::AuthFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The C++ error set has a `CipherError` variant for decryption that this
    /// one does not. `aes-gcm` reports every decryption failure as one opaque
    /// error by design — distinguishing "the cipher broke" from "the tag did
    /// not verify" is a padding-oracle-shaped invitation — so mapping them all
    /// to `AuthFailed` is deliberate, not an oversight.
    ///
    /// It is recorded as a test rather than only a comment because the parity
    /// harness compares error *names* with the C++ probe, and this is the one
    /// place the two sets legitimately differ.
    #[test]
    fn the_error_sets_differ_in_exactly_one_documented_way() {
        assert_eq!(
            decrypt(&[0u8; IV_SIZE + TAG_SIZE], &[0u8; KEY_SIZE]),
            Err(DecryptError::AuthFailed),
            "a well-formed but bogus blob must be AuthFailed, which is what the C++ side also \
             reports for it"
        );
    }

    #[test]
    fn round_trips() {
        let key = generate_key().expect("system randomness");
        for plaintext in [b"".as_slice(), b"x", b"hello clipboard", &[0xff; 5000]] {
            let blob = encrypt(plaintext, &key).expect("encrypt");
            assert_eq!(blob.len(), IV_SIZE + plaintext.len() + TAG_SIZE);
            assert_eq!(decrypt(&blob, &key).expect("decrypt"), plaintext);
        }
    }

    #[test]
    fn a_flipped_bit_anywhere_fails_authentication() {
        let key = generate_key().expect("system randomness");
        let blob = encrypt(b"hello clipboard", &key).expect("encrypt");

        // Every byte, including the IV and the tag: GCM authenticates the
        // nonce as well as the ciphertext, so there is no prefix an attacker
        // can edit freely.
        for index in 0..blob.len() {
            let mut tampered = blob.clone();
            tampered[index] ^= 0x01;
            assert_eq!(
                decrypt(&tampered, &key),
                Err(DecryptError::AuthFailed),
                "flipping a bit at offset {index} of {} was not detected",
                blob.len()
            );
        }
    }

    #[test]
    fn a_different_key_fails_authentication() {
        let blob = encrypt(b"secret", &generate_key().expect("rng")).expect("encrypt");
        assert_eq!(
            decrypt(&blob, &generate_key().expect("rng")),
            Err(DecryptError::AuthFailed)
        );
    }

    #[test]
    fn short_buffers_are_rejected_before_the_cipher_sees_them() {
        let key = generate_key().expect("system randomness");
        for len in 0..(IV_SIZE + TAG_SIZE) {
            assert_eq!(
                decrypt(&vec![0u8; len], &key),
                Err(DecryptError::DataTooShort),
                "a {len}-byte buffer should be DataTooShort"
            );
        }
        // One byte more is long enough to be a zero-length message, so it gets
        // as far as the tag check and fails there instead. This pins the
        // boundary rather than leaving it to whichever branch happens to run.
        assert_eq!(
            decrypt(&[0u8; IV_SIZE + TAG_SIZE], &key),
            Err(DecryptError::AuthFailed)
        );
    }

    #[test]
    fn wrong_key_sizes_are_rejected_by_both_directions() {
        for len in [0, 16, 31, 33, 64] {
            let key = vec![0u8; len];
            assert_eq!(encrypt(b"x", &key), Err(EncryptError::CipherError));
            assert_eq!(
                decrypt(&[0u8; 64], &key),
                Err(DecryptError::InvalidKeySize),
                "a {len}-byte key must be rejected before the length check on the blob"
            );
        }
    }

    #[test]
    fn derivation_is_deterministic_and_label_separated() {
        let master = [7u8; 32];
        assert_eq!(
            derive_key(&master, "clipboard"),
            derive_key(&master, "clipboard")
        );
        assert_ne!(
            derive_key(&master, "clipboard"),
            derive_key(&master, "clipboard2")
        );
        assert_ne!(
            derive_key(&master, "clipboard"),
            derive_key(&[8u8; 32], "clipboard")
        );
        assert_ne!(
            derive_key(&master, ""),
            derive_key(&master, "clipboard"),
            "an empty label must still be a distinct domain, not a pass-through"
        );
    }

    /// RFC 5869 test case 1, so the KDF is anchored to the standard and not
    /// only to whatever the C++ side happens to do. If both implementations
    /// were wrong in the same way, the cross-engine diff would be green and
    /// this would not.
    ///
    /// The RFC's vector uses a salt; ours does not, so this exercises
    /// `Hkdf::new` with the same inputs the RFC gives rather than
    /// [`derive_key`] itself.
    #[test]
    fn hkdf_matches_rfc_5869_case_1() {
        let ikm = [0x0bu8; 22];
        let salt: Vec<u8> = (0x00u8..=0x0c).collect();
        let info: Vec<u8> = (0xf0u8..=0xf9).collect();

        let hkdf = Hkdf::<Sha256>::new(Some(&salt), &ikm);
        let mut okm = [0u8; 42];
        hkdf.expand(&info, &mut okm).expect("42 bytes");

        assert_eq!(
            okm.as_slice(),
            [
                0x3c, 0xb2, 0x5f, 0x25, 0xfa, 0xac, 0xd5, 0x7a, 0x90, 0x43, 0x4f, 0x64, 0xd0, 0x36,
                0x2f, 0x2a, 0x2d, 0x2d, 0x0a, 0x90, 0xcf, 0x1a, 0x5a, 0x4c, 0x5d, 0xb0, 0x2d, 0x56,
                0xec, 0xc4, 0xc5, 0xbf, 0x34, 0x00, 0x72, 0x08, 0xd5, 0xb8, 0x87, 0x18, 0x58, 0x65
            ]
            .as_slice()
        );
    }
}
