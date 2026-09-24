//! `slugify`: a title turned into something that can be a directory name, a
//! command name, or a key.
//!
//! The [`slug`] crate, which transliterates to ASCII (`Café` → `cafe`,
//! `日本` → `ri-ben`) where the C++ `slugify` only strips accents and drops
//! everything else, leaving an empty name for a non-Latin title. See
//! PARITY.md.

/// Slugify `input`, joining words with `-`.
#[must_use]
pub fn slugify(input: &str) -> String {
    slug::slugify(input)
}
