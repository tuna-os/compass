//! Recording a copy in history — the ingest path.
//!
//! Ports `ClipboardService::saveSelection` (`clipboard-service.cpp`) for the
//! shape the GNOME helper extension actually delivers: one `ClipboardChanged`
//! signal carries a single offer (bytes plus a MIME type plus a best-effort
//! source app), not the multi-offer `ClipboardSelection` the Wayland and Qt
//! backends build. The multi-offer machinery — sorting by MIME type, deduping,
//! dropping the raw-image offer, the monitoring flag — stays on the C++ side
//! until the daemon loop that needs it is ported; with one offer there is
//! nothing to sort or dedupe.
//!
//! What is ported, in order, mirroring the C++:
//!
//! 1. Ignore rules: an empty selection (the "clear" selection), whitespace-only
//!    text, and offers of unknown kind never reach the database.
//! 2. De-duplication: `md5hex(data)` is looked up first, and a hit is bubbled
//!    to the top of the history instead of inserted again. This is the same
//!    key the C++ uses (`QCryptographicHash::hash(preferredOffer.data, Md5)`),
//!    so a row written by either engine bubbles under the other.
//! 3. Fresh rows: one selection plus its single offer, the text indexed for
//!    search when the kind is text-like, and the payload written to
//!    `data_dir/offer_id` — encrypted with the clipboard key when one is
//!    supplied, verbatim otherwise. `EncryptionType` records which.
//!
//! # Deliberate approximations
//!
//! * `Utils::isTextMimeType` consults the shared-MIME database
//!   (`inherits("text/plain")`). There is no MIME database here, so
//!   [`classify`] treats `text/*` plus a short explicit list of text-inheriting
//!   `application/*` types as text. Anything else unrecognised is ignored,
//!   which is the safe direction: the C++ ignores unknown kinds too.
//! * Image previews read `"Image"`. The C++ decodes the image and reports its
//!   dimensions (`Image (WxH)`); there is no image decoder in this crate, so
//!   the fallback text is used unconditionally.
//! * URL parsing is a small hand-written `scheme://authority` split, not
//!   `QUrl::fromEncoded(StrictMode)`. Garbage that `QUrl` would reject parses
//!   as "not a URL" here too, and both sides agree on the well-formed cases
//!   the extension actually emits.
//! * A payload file that cannot be written is an error. The C++ logs and keeps
//!   the database row, leaving a history entry whose payload is missing; a new
//!   implementation should not reproduce a dangling row on purpose.
//!
//! The caller owns the subscription loop: read `ClipboardChanged` from
//! `compass-shell`, build an [`Incoming`], call [`ingest`]. IDs are
//! caller-supplied (pass UUIDs in production) so this module needs no RNG and
//! tests stay deterministic: `next_id` is called exactly twice per insert,
//! first for the selection, then for the offer, and never otherwise.

use std::path::{Path, PathBuf};

use md5::{Digest as _, Md5};

use crate::kind::OfferKind;
use crate::write::{self, NewOffer, NewSelection};

/// One observed copy: the body of a `ClipboardChanged` signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming<'a> {
    /// Raw bytes. For `text/*` this is UTF-8.
    pub data: &'a [u8],
    /// MIME type, possibly with parameters (`text/plain;charset=utf-8`).
    pub mime_type: &'a str,
    /// Best-effort application id of whoever copied; `None` when unknown.
    pub source_app: Option<&'a str>,
}

/// Why a copy was not recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum IgnoreReason {
    /// Nothing was copied (the "clear" selection).
    Empty,
    /// Text with nothing but whitespace in it.
    BlankText,
    /// The MIME type classifies as nothing history knows how to show.
    UnknownKind,
}

/// What [`ingest`] did with a copy.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Decision {
    /// The copy was ignored, and why.
    Ignored(IgnoreReason),
    /// An identical selection already existed and was moved to the top.
    BubbledUp,
    /// A new selection was recorded, with fresh ids.
    Inserted {
        /// The new row in `selection`.
        selection_id: String,
        /// The new row in `data_offer`, also the payload's filename.
        offer_id: String,
    },
}

/// Errors from recording a copy.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// SQLite refused, including inside the write helpers.
    #[error(transparent)]
    Database(#[from] compass_sqlcipher_sys::Error),

    /// A write helper refused.
    #[error(transparent)]
    Write(#[from] write::Error),

    /// The payload could not be encrypted.
    #[error(transparent)]
    Encrypt(#[from] compass_crypto::EncryptError),

    /// The payload directory or file could not be written.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

type Result<T> = std::result::Result<T, Error>;

/// How many characters of a text preview history shows.
///
/// `QString::mid(0, 50)` in `getOfferTextPreview`, counted the same way
/// [`write::truncate_for_index`] counts: UTF-16 code units.
pub const PREVIEW_LEN: usize = 50;

/// Lowercase hex MD5, as `QCryptographicHash::hash(...).toHex()` produces.
fn md5hex(data: &[u8]) -> String {
    let digest = Md5::digest(data);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(out, "{byte:02x}").expect("writing to a String cannot fail");
    }
    out
}

/// The MIME type without parameters or padding: `text/plain;charset=utf-8`
/// becomes `text/plain`.
fn base_mime(mime: &str) -> &str {
    mime.split(';').next().unwrap_or(mime).trim()
}

/// Whether a MIME type counts as text.
///
/// Approximation of `Utils::isTextMimeType` (`QMimeDatabase`, inherits
/// `text/plain`): every `text/*` type, plus the `application/*` types that the
/// shared-MIME database marks as text-inheriting and that plausibly appear on
/// a clipboard. Anything missing here degrades to `UnknownKind` and is
/// ignored, matching what the C++ does with kinds it cannot show.
fn is_text_mime(mime: &str) -> bool {
    const TEXT_INHERITING_APPLICATION: &[&str] = &[
        "application/json",
        "application/xml",
        "application/javascript",
        "application/ecmascript",
        "application/x-javascript",
        "application/x-www-form-urlencoded",
        "application/rtf",
    ];
    if mime.starts_with("text/") {
        return true;
    }
    if let Some(subtype) = mime.strip_prefix("application/") {
        return TEXT_INHERITING_APPLICATION.contains(&mime)
            || subtype.ends_with("+json")
            || subtype.ends_with("+xml");
    }
    false
}

/// The URI scheme, lowercased: `HTTPS://x` gives `https`. Empty when there is
/// no scheme. This is parsing, not validation — enough to tell `file` from
/// `https` from "just words".
fn scheme_of(text: &str) -> &str {
    let end = text
        .find(':')
        .filter(|colon| *colon > 0 && text[..*colon].chars().all(is_scheme_char));
    match end {
        Some(colon) => &text[..colon],
        None => "",
    }
}

fn is_scheme_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.')
}

/// Whether a `text/uri-list` line names a local file, mirroring
/// `QUrl(uri).isLocalFile()` for the shapes that occur there: `file://`
/// URIs and bare paths (no scheme at all) are local; anything with another
/// scheme is not.
fn is_local_file_uri(line: &str) -> bool {
    let scheme = scheme_of(line).to_ascii_lowercase();
    if scheme.is_empty() || scheme == "file" {
        if scheme == "file" {
            let after = &line[line.find(':').map_or(0, |i| i + 1)..];
            let authority = after.strip_prefix("//").unwrap_or(after);
            let host = authority.split(['/', '?', '#']).next().unwrap_or("");
            return host.is_empty() || host == "localhost";
        }
        return true;
    }
    false
}

/// What kind of thing a single offer holds — the single-offer restriction of
/// `ClipboardService::getKind`.
#[must_use]
pub fn classify(mime_type: &str, data: &[u8]) -> OfferKind {
    let mime = base_mime(mime_type);
    if mime == "text/uri-list" {
        let text = String::from_utf8_lossy(data);
        let mut lines = text
            .split("\r\n")
            .filter(|line| !line.is_empty())
            .peekable();
        if lines.peek().is_none() {
            return OfferKind::Text;
        }
        return if lines.all(is_local_file_uri) {
            OfferKind::File
        } else {
            OfferKind::Text
        };
    }
    if mime.starts_with("image/") {
        return OfferKind::Image;
    }
    if mime == "text/html" {
        return OfferKind::Text;
    }
    if is_text_mime(mime) {
        let Ok(text) = std::str::from_utf8(data) else {
            return OfferKind::Text;
        };
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.contains([' ', '\t', '\n', '\r']) {
            return OfferKind::Text;
        }
        let scheme = scheme_of(trimmed).to_ascii_lowercase();
        if scheme.is_empty() {
            return OfferKind::Text;
        }
        if scheme == "file" && is_local_file_uri(trimmed) {
            return OfferKind::File;
        }
        return OfferKind::Link;
    }
    OfferKind::Unknown
}

/// The host of an `http(s)` URL, so history can fetch a favicon without
/// parsing. `None` when there is nothing worth storing.
fn url_host(data: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(data).ok()?.trim();
    if !scheme_of(text).to_ascii_lowercase().starts_with("http") {
        return None;
    }
    let after_scheme = text.split(':').nth(1)?.strip_prefix("//")?;
    let authority = after_scheme.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = if let Some(bracketed) = host.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or("")
    } else {
        host.split(':').next().unwrap_or(host)
    };
    if host.is_empty() {
        None
    } else {
        Some(host.to_owned())
    }
}

/// Truncate to [`PREVIEW_LEN`] UTF-16 code units without splitting a
/// surrogate pair — the same rule as [`write::truncate_for_index`], at a
/// different limit.
fn truncate_preview(content: &str) -> &str {
    let mut units = 0usize;
    for (offset, ch) in content.char_indices() {
        let width = ch.len_utf16();
        if units + width > PREVIEW_LEN {
            return &content[..offset];
        }
        units += width;
    }
    content
}

/// The list-view preview — the single-offer restriction of
/// `ClipboardService::getOfferTextPreview`.
///
/// `simplified()` (trim, collapse inner whitespace runs) is reproduced with
/// `split_whitespace`, which agrees on everything the extension emits.
#[must_use]
pub fn preview(kind: OfferKind, data: &[u8]) -> String {
    match kind {
        OfferKind::Text | OfferKind::Link | OfferKind::File => {
            let text = String::from_utf8_lossy(data);
            let simplified: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
            truncate_preview(&simplified).to_owned()
        }
        // No image decoder in this crate; the C++ falls back to this same
        // text when its decoder cannot read the image.
        OfferKind::Image => "Image".to_owned(),
        OfferKind::Unknown | OfferKind::Count => "Unknown".to_owned(),
    }
}

/// Record one observed copy in history.
///
/// See the module docs for the ported semantics. `clipboard_key` is the
/// `vicinae-clipboard` subkey (`compass_crypto::derive_key(master,
/// CLIPBOARD_LABEL)`); `None` stores the payload verbatim with
/// `EncryptionType::None`, exactly like a C++ service started without a
/// keyring. `next_id` is called twice per insert — selection first, offer
/// second — and never otherwise.
///
/// # Errors
///
/// Returns [`Error::Database`] if the store refuses, [`Error::Encrypt`] if
/// the payload cannot be sealed, and [`Error::Io`] if the payload file cannot
/// be written (in which case the row is already committed; see the module
/// docs).
pub fn ingest(
    db: &compass_sqlcipher_sys::Database,
    data_dir: &Path,
    copy: &Incoming<'_>,
    clipboard_key: Option<&[u8; compass_crypto::KEY_SIZE]>,
    next_id: &mut dyn FnMut() -> String,
) -> Result<Decision> {
    // The "clear" selection: every offer empty. With a single offer that is
    // just an empty payload.
    if copy.data.is_empty() {
        return Ok(Decision::Ignored(IgnoreReason::Empty));
    }

    let kind = classify(copy.mime_type, copy.data);
    if kind == OfferKind::Unknown || kind == OfferKind::Count {
        return Ok(Decision::Ignored(IgnoreReason::UnknownKind));
    }

    // `preferredOfferIt->data.trimmed().isEmpty()` in `saveSelection`.
    if kind == OfferKind::Text && String::from_utf8_lossy(copy.data).trim().is_empty() {
        return Ok(Decision::Ignored(IgnoreReason::BlankText));
    }

    // Same key the C++ bubbles and inserts under, so rows written by either
    // engine deduplicate against each other.
    let hash = md5hex(copy.data);
    if write::bubble_up(db, &hash)? {
        return Ok(Decision::BubbledUp);
    }

    let selection_id = next_id();
    let offer_id = next_id();
    let text = String::from_utf8_lossy(copy.data);
    let text_preview = preview(kind, copy.data);
    let host = if kind == OfferKind::Link {
        url_host(copy.data)
    } else {
        None
    };
    let encryption = if clipboard_key.is_some() {
        crate::kind::EncryptionType::Local
    } else {
        crate::kind::EncryptionType::None
    };

    let tx = db.transaction()?;
    write::insert_selection(
        db,
        &NewSelection {
            id: &selection_id,
            offer_count: 1,
            hash: &hash,
            preferred_mime_type: copy.mime_type,
            kind,
            source: copy.source_app,
        },
    )?;
    // "Index all offers, including empty ones" — empties cannot reach here,
    // but the shape is the same: every text-like offer is indexed.
    if matches!(kind, OfferKind::Text | OfferKind::Link) {
        write::index_content(db, &selection_id, &text)?;
    }
    write::insert_offer(
        db,
        &NewOffer {
            id: &offer_id,
            selection_id: &selection_id,
            mime_type: copy.mime_type,
            text_preview: &text_preview,
            md5sum: &hash,
            encryption,
            kind,
            size: i64::try_from(copy.data.len()).unwrap_or(i64::MAX),
            url_host: host.as_deref(),
        },
    )?;
    tx.commit()?;

    let payload: Vec<u8> = match clipboard_key {
        Some(key) => compass_crypto::encrypt(copy.data, key)?,
        None => copy.data.to_vec(),
    };
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(data_dir.join(&offer_id), payload)?;

    Ok(Decision::Inserted {
        selection_id,
        offer_id,
    })
}

/// The payload file for an offer, mirroring `m_dataDir / offerId`.
#[must_use]
pub fn payload_path(data_dir: &Path, offer_id: &str) -> PathBuf {
    data_dir.join(offer_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameters_do_not_change_the_classification() {
        assert_eq!(
            classify("text/plain;charset=utf-8", b"hello"),
            OfferKind::Text
        );
    }

    #[test]
    fn plain_words_are_text_not_links() {
        assert_eq!(classify("text/plain", b"just some words"), OfferKind::Text);
        assert_eq!(classify("text/plain", b"hello"), OfferKind::Text);
    }

    #[test]
    fn urls_are_links_and_local_files_are_files() {
        assert_eq!(
            classify("text/plain", b"https://example.com/x"),
            OfferKind::Link
        );
        assert_eq!(
            classify("text/plain", b"file:///etc/hosts"),
            OfferKind::File
        );
    }

    #[test]
    fn uri_lists_follow_their_entries() {
        assert_eq!(
            classify("text/uri-list", b"file:///home/a\r\nfile:///home/b\r\n"),
            OfferKind::File
        );
        assert_eq!(
            classify("text/uri-list", b"https://example.com/x\r\n"),
            OfferKind::Text
        );
    }

    #[test]
    fn images_html_and_unknown() {
        assert_eq!(classify("image/png", &[0x89, 0x50]), OfferKind::Image);
        assert_eq!(classify("text/html", b"<b>hi</b>"), OfferKind::Text);
        assert_eq!(
            classify("application/octet-stream", b"\x00\x01"),
            OfferKind::Unknown
        );
        assert_eq!(classify("", b"data"), OfferKind::Unknown);
    }

    #[test]
    fn preview_collapses_whitespace_and_truncates() {
        assert_eq!(preview(OfferKind::Text, b"  a   b\n c "), "a b c");
        let long = "w".repeat(PREVIEW_LEN + 10);
        assert_eq!(preview(OfferKind::Text, long.as_bytes()).len(), PREVIEW_LEN);
        assert_eq!(preview(OfferKind::Image, &[0x89]), "Image");
    }

    #[test]
    fn host_extraction_keeps_it_simple() {
        assert_eq!(
            url_host(b"https://example.com:8080/x?q=1"),
            Some("example.com".to_owned())
        );
        assert_eq!(
            url_host(b"https://user@example.com/x"),
            Some("example.com".to_owned())
        );
        assert_eq!(url_host(b"not a url"), None);
        assert_eq!(url_host(b"file:///etc/hosts"), None);
    }
}
