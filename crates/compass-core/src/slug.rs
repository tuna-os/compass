//! `slugify`: a title turned into something that can be a directory name, a
//! command name, or a key.
//!
//! A port of `slugify` (`src/server/src/utils/utils.cpp`).
//!
//! # Why the decomposition step matters
//!
//! The C++ normalises to NFD *before* it strips anything that is not
//! `[a-z0-9<sep>]`. That order is the whole trick: `é` decomposes into `e`
//! plus a combining acute, the strip removes the accent and keeps the `e`, so
//! `Café` slugs to `cafe` rather than `caf`. Stripping first would silently
//! eat every accented letter, which for a non-English title is most of them.

use unicode_normalization::UnicodeNormalization;

/// The separator `slugify` uses when none is given.
pub const DEFAULT_SEPARATOR: &str = "-";

/// Slugify `input` with [`DEFAULT_SEPARATOR`].
#[must_use]
pub fn slugify(input: &str) -> String {
    slugify_with(input, DEFAULT_SEPARATOR)
}

/// Slugify `input`, joining words with `separator`.
///
/// An empty `input` slugs to an empty string, as it does in the C++.
#[must_use]
pub fn slugify_with(input: &str, separator: &str) -> String {
    // Lowercase, then decompose, so the strip below sees base letters.
    let decomposed: String = input.to_lowercase().nfd().collect();

    // `[\s_]+` -> separator. The C++ regex collapses the run itself; here one
    // separator is written per whitespace character and the collapse pass at
    // the end merges them, which reaches the same string with one rule rather
    // than two. The C++'s early return on an empty input is likewise dropped:
    // an empty input falls through every step below unchanged.
    let mut out = String::with_capacity(decomposed.len());
    for ch in decomposed.chars() {
        if ch.is_whitespace() || ch == '_' {
            out.push_str(separator);
        } else {
            out.push(ch);
        }
    }

    // Remove everything that is not `[a-z0-9<separator>]`.
    let sep_chars: Vec<char> = separator.chars().collect();
    let kept: String = out
        .chars()
        .filter(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || sep_chars.contains(ch))
        .collect();

    if separator.is_empty() {
        return kept;
    }

    // Trim leading and trailing separators, then collapse runs of them.
    let trimmed = trim_repeated(&kept, separator);
    collapse_repeated(trimmed, separator)
}

/// Strip whole leading and trailing repetitions of `separator`.
fn trim_repeated<'a>(input: &'a str, separator: &str) -> &'a str {
    let mut slice = input;
    while let Some(rest) = slice.strip_prefix(separator) {
        slice = rest;
    }
    while let Some(rest) = slice.strip_suffix(separator) {
        slice = rest;
    }
    slice
}

/// Collapse two or more consecutive `separator`s into one.
fn collapse_repeated(input: &str, separator: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(idx) = rest.find(separator) {
        out.push_str(&rest[..idx]);
        out.push_str(separator);
        rest = &rest[idx + separator.len()..];
        while let Some(next) = rest.strip_prefix(separator) {
            rest = next;
        }
    }
    out.push_str(rest);
    out
}
