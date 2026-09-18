//! Which clipboard offers are kept, and which of them are read.
//!
//! Ports `src/data-control-server/src/selection.cpp` and
//! `src/data-control-server/src/wayland/mime.hpp` — the part of the
//! `wlr-data-control` helper that decides, given the MIME types a Wayland
//! client is offering, which of them belong in a clipboard entry and which
//! need their bytes fetched over a pipe.
//!
//! The selection arrives as a list of MIME types, and reading each one costs a
//! round trip through a file descriptor, so the filtering is not cosmetic: it
//! is what stops a screenshot tool that offers six encodings of the same image
//! from being copied six times.

use std::collections::BTreeSet;

/// The MIME type a client sets to say its selection is a password.
pub const PASSWORD_HINT_MIME_TYPE: &str = "x-kde-passwordManagerHint";

/// The MIME type this project sets to say a selection must not be recorded.
pub const CONCEALED_MIME_TYPE: &str = "vicinae/concealed";

/// Types that are kept in the entry but whose bytes are never requested.
///
/// They carry their meaning entirely in their presence.
pub const FLAG_MIMES: &[&str] = &[PASSWORD_HINT_MIME_TYPE, CONCEALED_MIME_TYPE];

/// Types kept for their data despite not matching one of the accepted prefixes.
pub const DATA_MIMES: &[&str] = &["x-special/gnome-copied-files"];

/// Types dropped outright.
///
/// `application/x-qt-image` is a hint that an image is available, and the
/// filter already picks exactly one image encoding, so the hint is noise.
pub const IGNORED_MIMES: &[&str] = &["application/x-qt-image"];

/// Image encodings in the order they are preferred, best first.
pub const PREFERRED_IMAGE_TYPES: &[&str] = &[
    "image/gif",
    "image/png",
    "image/jpeg",
    "image/jpg",
    "image/webp",
];

/// The largest primary selection that is recorded, in bytes.
///
/// The primary selection changes on every drag of the mouse, so a large one is
/// dropped rather than stored.
pub const MAX_PRIMARY_SIZE: usize = 1 << 20;

/// Whether a type is kept for its data.
pub fn is_data_mime(mime: &str) -> bool {
    if IGNORED_MIMES.contains(&mime) {
        return false;
    }
    mime.starts_with("text/")
        || mime.starts_with("image/")
        || mime.starts_with("application/")
        || DATA_MIMES.contains(&mime)
}

/// Whether a type is kept for its presence alone.
pub fn is_flag_mime(mime: &str) -> bool {
    FLAG_MIMES.contains(&mime)
}

/// Where an image encoding sits in the preference order.
///
/// The C++ computes this as the negated distance from the start of the
/// preference list, and an unknown encoding lands at the *end* of the list —
/// so every unknown encoding scores the same, and worse than any known one.
fn image_priority(mime: &str) -> isize {
    let index = PREFERRED_IMAGE_TYPES
        .iter()
        .position(|candidate| *candidate == mime)
        .unwrap_or(PREFERRED_IMAGE_TYPES.len());
    -(index as isize)
}

/// Pick the MIME types worth keeping from everything a client offered.
///
/// Three things happen, in this order:
///
/// 1. Anything that is neither a flag nor data is dropped.
/// 2. At most one image encoding survives, the best-ranked one offered. A
///    later encoding must be **strictly** better to displace an earlier one,
///    so two equally-ranked encodings — which includes any two the preference
///    list does not name — keep the first offered.
/// 3. `text/plain` is dropped when `text/plain;charset=utf-8` is also there,
///    because the two hold the same text and only one of them says how it is
///    encoded.
pub fn filter_mimes(offer_mimes: &[String]) -> BTreeSet<String> {
    let mut filtered: BTreeSet<String> = BTreeSet::new();
    let mut saved_image: Option<String> = None;

    for mime in offer_mimes
        .iter()
        .filter(|m| is_flag_mime(m) || is_data_mime(m))
    {
        if mime.starts_with("image/") {
            if let Some(saved) = &saved_image {
                if image_priority(mime) <= image_priority(saved) {
                    continue;
                }
                filtered.remove(saved);
            }
            saved_image = Some(mime.clone());
        }
        filtered.insert(mime.clone());
    }

    if filtered.contains("text/plain") && filtered.contains("text/plain;charset=utf-8") {
        filtered.remove("text/plain");
    }

    filtered
}

/// One MIME type and the bytes behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    /// What the bytes are.
    pub mime_type: String,
    /// The bytes, empty for a flag type.
    pub data: Vec<u8>,
}

/// A clipboard entry: everything kept from one selection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// The kept types, in lexicographic order.
    ///
    /// The C++ collects them into a `std::set<std::string>`, so the client's
    /// own ordering is lost — `offers[0]` is the alphabetically first type,
    /// not the client's first choice.
    pub offers: Vec<Offer>,
}

/// Read the bytes for each kept type.
///
/// A flag type is recorded with no data and, more to the point, is never read:
/// asking a client for the bytes behind a password hint would be a pipe read
/// for something that was never meant to have contents.
pub fn build_selection(
    filtered: &BTreeSet<String>,
    mut receive: impl FnMut(&str) -> Vec<u8>,
) -> Selection {
    let mut selection = Selection::default();
    for mime in filtered {
        let data = if is_flag_mime(mime) {
            Vec::new()
        } else {
            receive(mime)
        };
        selection.offers.push(Offer {
            mime_type: mime.clone(),
            data,
        });
    }
    selection
}

/// Build the entry for a primary selection — the middle-click buffer.
///
/// This is far stricter than the clipboard proper. A concealed selection is
/// dropped whole; only plain text is kept, UTF-8 first; and a selection over
/// [`MAX_PRIMARY_SIZE`] is dropped rather than truncated, because the primary
/// selection changes on every drag and a partial record of one is worse than
/// none.
pub fn build_primary_selection(
    mimes: &[String],
    mut receive: impl FnMut(&str) -> Vec<u8>,
) -> Selection {
    let selection = Selection::default();

    if mimes.iter().any(|m| m == CONCEALED_MIME_TYPE) {
        return selection;
    }

    for candidate in ["text/plain;charset=utf-8", "text/plain"] {
        if !mimes.iter().any(|m| m == candidate) {
            continue;
        }
        let raw = receive(candidate);
        if raw.len() > MAX_PRIMARY_SIZE {
            return selection;
        }
        return Selection {
            offers: vec![Offer {
                mime_type: candidate.to_owned(),
                data: raw,
            }],
        };
    }

    selection
}
