//! Turning a filename into the words it can be found by.
//!
//! A port of `file_indexer::vocab`
//! (`src/file-indexer/src/file-indexer/vocabulary.hpp`).
//!
//! # This decides what is findable at all
//!
//! A file only turns up in a search if one of these tokens matches what was
//! typed. Split too coarsely and `AnnualReport2024.pdf` is findable only by
//! its whole name; too finely and the index fills with fragments that match
//! everything. Drop the wrong thing as junk and a file is simply not there,
//! with nothing anywhere saying why — which is why the junk rules are narrow
//! and specific rather than clever.

/// The shortest token worth indexing.
///
/// Two letters match far too much to be worth the row.
pub const MIN_TOKEN_LENGTH: usize = 3;

/// The longest token worth indexing.
///
/// Past this it is a hash, a base64 blob or a concatenation — never something
/// a person will type.
pub const MAX_TOKEN_LENGTH: usize = 24;

/// The length at which an all-hex token is treated as a content hash.
pub const MIN_HEX_JUNK_LENGTH: usize = 12;

/// Whether `token` is noise rather than a word.
///
/// Two rules only: too long, or long enough and entirely hexadecimal. The
/// second catches the content hashes that fill a build directory or a git
/// object store, and the twelve-character floor is what keeps it from eating
/// real words — `deface`, `facade` and `added` are all hex-shaped.
#[must_use]
pub fn is_junk_token(token: &str) -> bool {
    // Lengths are in bytes, as `std::string::size()` is. For a non-ASCII name
    // that makes the limit stricter than a character count would, which is the
    // C++'s behaviour and not worth diverging from: the limit exists to reject
    // blobs, and a 24-byte Japanese filename is eight characters.
    if token.len() > MAX_TOKEN_LENGTH {
        return true;
    }
    token.len() >= MIN_HEX_JUNK_LENGTH
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The vowels the skeleton drops.
#[must_use]
pub const fn is_skeleton_vowel(byte: u8) -> bool {
    matches!(byte, b'a' | b'e' | b'i' | b'o' | b'u')
}

/// A token reduced to its consonant skeleton.
///
/// Lowercased, then vowels and doubled letters removed — except at the very
/// start, where the first character is always kept whatever it is. That is
/// what makes `apple` and `aple` and `appel` share a skeleton, so a typo still
/// finds the file, while `apple` and `ripple` do not collide on their first
/// letter.
#[must_use]
pub fn skeletonize_token(token: &str) -> String {
    let mut skeleton = String::with_capacity(token.len());
    let mut last = 0u8;

    for byte in token.bytes() {
        let byte = byte.to_ascii_lowercase();

        if !skeleton.is_empty() {
            if is_skeleton_vowel(byte) {
                continue;
            }
            if byte == last {
                continue;
            }
        }

        skeleton.push(byte as char);
        last = byte;
    }

    skeleton
}

/// Everything after the last `/`.
#[must_use]
pub fn basename(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}

/// Everything after the last `.`.
///
/// The whole string when there is no dot, as the C++ returns — so a file with
/// no extension reports its own name as one. Reproduced because the callers
/// treat it as a hint rather than a fact.
#[must_use]
pub fn file_extension(path: &str) -> &str {
    path.rsplit_once('.')
        .map_or(path, |(_, extension)| extension)
}

/// Everything before the last `/`.
#[must_use]
pub fn dirname(path: &str) -> &str {
    path.rsplit_once('/')
        .map_or(path, |(directory, _)| directory)
}

/// Whether a byte can be part of a token.
///
/// Alphanumeric, or anything outside ASCII — so a name in Greek or Japanese is
/// one token rather than being split at every byte.
fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte >= 0x80
}

/// The words `name` can be found by.
///
/// # Where it splits
///
/// At anything that is not a token byte, and at two kinds of case boundary:
/// after a lowercase letter or digit (`annualReport` becomes `annual`,
/// `report`) and at the end of an acronym (`XMLParser` becomes `xml`,
/// `parser`). Without the second, an acronym swallows the word after it.
///
/// # What it drops
///
/// Tokens shorter than [`MIN_TOKEN_LENGTH`], tokens that are all digits — a
/// year or a serial number matches too many files to be useful — and anything
/// [`is_junk_token`] rejects.
#[must_use]
pub fn tokenize_filename(name: &str) -> Vec<String> {
    let bytes = name.as_bytes();
    let mut tokens: Vec<String> = Vec::new();
    // Accumulated as bytes, not as `char`s: the C++ builds a `std::string`
    // byte by byte, and pushing a UTF-8 continuation byte as a `char` would
    // turn one accented letter into two wrong ones.
    let mut current: Vec<u8> = Vec::new();

    let flush = |current: &mut Vec<u8>, tokens: &mut Vec<String>| {
        let token = String::from_utf8_lossy(current).into_owned();
        let all_digits = !token.is_empty() && token.bytes().all(|b| b.is_ascii_digit());
        if token.len() >= MIN_TOKEN_LENGTH && !all_digits && !is_junk_token(&token) {
            tokens.push(token);
        }
        current.clear();
    };

    for index in 0..bytes.len() {
        let byte = bytes[index];

        if !is_token_byte(byte) {
            flush(&mut current, &mut tokens);
            continue;
        }

        if byte.is_ascii_uppercase() && !current.is_empty() {
            let previous = bytes[index - 1];
            let camel_boundary = previous.is_ascii_lowercase() || previous.is_ascii_digit();
            let acronym_end = previous.is_ascii_uppercase()
                && index + 1 < bytes.len()
                && bytes[index + 1].is_ascii_lowercase();

            if camel_boundary || acronym_end {
                flush(&mut current, &mut tokens);
            }
        }

        current.push(byte.to_ascii_lowercase());
    }

    flush(&mut current, &mut tokens);

    tokens
}
