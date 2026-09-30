//! What a clipboard entry is, and whether it is encrypted — as stored.
//!
//! Both of these are written to SQLite as plain integers (`selection.kind`,
//! `data_offer.kind`, `data_offer.encryption_type` in
//! `001_init.sql`), so their numeric values are a **storage format**, not an
//! implementation detail. Renumbering them does not break a build; it silently
//! reinterprets every row already on disk — an image becomes a link, an
//! encrypted offer becomes a plaintext one. The discriminants below are
//! therefore written out explicitly rather than left implicit, and the tests
//! below pin them to the values upstream Vicinae's C++ header assigns.

/// Whether an offer's payload on disk is encrypted.
///
/// Mirrors `ClipboardEncryptionType` in
/// `src/server/src/services/clipboard/clipboard-db.hpp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum EncryptionType {
    /// Stored as written.
    #[default]
    None = 0,
    /// Encrypted with the locally derived clipboard key — see
    /// [`compass_crypto`](../../compass_crypto/index.html).
    Local = 1,
}

/// What kind of thing a clipboard entry holds.
///
/// Mirrors `ClipboardOfferKind`. The C++ enum ends with a `Count` member whose
/// trailing comment reads `/* a link ot a file */`; despite that, nothing
/// classifies anything as `Count`, and the one place that switches on the enum
/// (`clipboard-history-view-host.cpp`) falls it through to the same label as
/// `Unknown`. It is an enumerator-count sentinel that acquired a misleading
/// comment. It is kept here because it occupies value `5` — dropping it would
/// let a future member take that value and change what an existing row means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum OfferKind {
    /// Not classified.
    #[default]
    Unknown = 0,
    /// Plain text.
    Text = 1,
    /// A URL.
    Link = 2,
    /// An image.
    Image = 3,
    /// A file.
    File = 4,
    /// Not a kind: the C++ enum's trailing count sentinel, reserved so nothing
    /// else claims value `5`. Reads back as `Unknown` everywhere it is shown.
    Count = 5,
}

/// A stored integer that does not name any known variant.
///
/// Reading one is not a reason to drop the row: an entry written by a newer
/// build is still an entry, and the caller decides whether to show it as
/// unknown or to refuse. Making that the caller's decision is why this is an
/// error rather than a silent fallback to `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{value} does not name a {enum_name}")]
pub struct UnknownDiscriminant {
    /// The integer read from the database.
    pub value: i64,
    /// Which enum it failed to name.
    pub enum_name: &'static str,
}

impl EncryptionType {
    /// The integer written to `data_offer.encryption_type`.
    #[must_use]
    pub const fn to_stored(self) -> i64 {
        self as i64
    }

    /// Read back a stored integer.
    ///
    /// # Errors
    ///
    /// Returns [`UnknownDiscriminant`] when the value names no variant.
    pub const fn from_stored(value: i64) -> Result<Self, UnknownDiscriminant> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::Local),
            _ => Err(UnknownDiscriminant {
                value,
                enum_name: "ClipboardEncryptionType",
            }),
        }
    }
}

impl OfferKind {
    /// The integer written to `selection.kind` and `data_offer.kind`.
    #[must_use]
    pub const fn to_stored(self) -> i64 {
        self as i64
    }

    /// Read back a stored integer.
    ///
    /// # Errors
    ///
    /// Returns [`UnknownDiscriminant`] when the value names no variant.
    pub const fn from_stored(value: i64) -> Result<Self, UnknownDiscriminant> {
        match value {
            0 => Ok(Self::Unknown),
            1 => Ok(Self::Text),
            2 => Ok(Self::Link),
            3 => Ok(Self::Image),
            4 => Ok(Self::File),
            5 => Ok(Self::Count),
            _ => Err(UnknownDiscriminant {
                value,
                enum_name: "ClipboardOfferKind",
            }),
        }
    }

    /// Every variant, in storage order.
    pub const ALL: [Self; 6] = [
        Self::Unknown,
        Self::Text,
        Self::Link,
        Self::Image,
        Self::File,
        Self::Count,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_integer_round_trips() {
        for kind in OfferKind::ALL {
            assert_eq!(OfferKind::from_stored(kind.to_stored()), Ok(kind));
        }
        for enc in [EncryptionType::None, EncryptionType::Local] {
            assert_eq!(EncryptionType::from_stored(enc.to_stored()), Ok(enc));
        }
    }

    #[test]
    fn the_discriminants_are_the_ones_the_cpp_enum_assigns() {
        // A round trip is satisfied by *any* self-consistent numbering, so it
        // cannot catch a renumbering that shifts both directions together.
        // These are the values from clipboard-db.hpp, written out.
        assert_eq!(OfferKind::Unknown.to_stored(), 0);
        assert_eq!(OfferKind::Text.to_stored(), 1);
        assert_eq!(OfferKind::Link.to_stored(), 2);
        assert_eq!(OfferKind::Image.to_stored(), 3);
        assert_eq!(OfferKind::File.to_stored(), 4);
        assert_eq!(OfferKind::Count.to_stored(), 5);

        assert_eq!(EncryptionType::None.to_stored(), 0);
        assert_eq!(EncryptionType::Local.to_stored(), 1);
    }

    #[test]
    fn unencrypted_is_the_default_because_zero_is_what_an_old_row_holds() {
        assert_eq!(EncryptionType::default(), EncryptionType::None);
        assert_eq!(OfferKind::default(), OfferKind::Unknown);
    }

    #[test]
    fn an_unrecognised_value_is_reported_rather_than_guessed() {
        let err = OfferKind::from_stored(6).unwrap_err();
        assert_eq!(err.value, 6);
        assert_eq!(err.enum_name, "ClipboardOfferKind");
        assert!(err.to_string().contains("ClipboardOfferKind"));

        assert!(OfferKind::from_stored(-1).is_err());
        assert!(EncryptionType::from_stored(2).is_err());
    }

    #[test]
    fn no_two_variants_share_a_value() {
        // The failure this guards against is a duplicated discriminant, which
        // compiles fine for a non-C-like use and makes `from_stored`
        // unrecoverably ambiguous.
        let mut seen = std::collections::HashSet::new();
        for kind in OfferKind::ALL {
            assert!(seen.insert(kind.to_stored()), "{kind:?} reuses a value");
        }
        assert_eq!(seen.len(), OfferKind::ALL.len());
    }
}
