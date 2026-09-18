//! The version comparison that decides whether an update is an update.
//!
//! A port of `vicinae::Semver` (`src/server/src/services/update/semver.hpp`).
//!
//! # It is not semver
//!
//! Despite the name it parses a dotted run of decimal integers and nothing
//! else — no pre-release suffix, no build metadata, no letters anywhere. A tag
//! like `v1.2.3-rc1` does not parse, which is how release candidates are kept
//! from being offered as updates without anyone having to filter them. That
//! is load-bearing, so it is pinned rather than "fixed" into real semver.

use std::cmp::Ordering;

/// A dotted run of integers, as a release tag carries it.
#[derive(Debug, Clone, Default)]
pub struct Semver {
    /// The components, most significant first.
    pub components: Vec<u32>,
}

impl Semver {
    /// Parse `text`, which may carry one leading `v`.
    ///
    /// `None` for anything that is not digits and dots: an empty string, a
    /// leading or trailing dot, two dots in a row, or any other character.
    ///
    /// # A divergence: a component too large to hold is refused
    ///
    /// The C++ accumulates into an `unsigned` with `current * 10 + digit` and
    /// no overflow check, so a component past 2^32 wraps silently — and
    /// `4294967296.0.0` compares equal to `0.0.0`, which would make a new
    /// release look like no release. This returns `None` instead, so such a
    /// tag is "not a release tag" rather than a wrong answer.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        // The C++ returns early on an empty string; that is redundant, because
        // an empty run leaves `has_digit` false and the check at the end
        // refuses it anyway. One rule is one rule that can be tested.
        let text = text.strip_prefix('v').unwrap_or(text);

        let mut components = Vec::with_capacity(3);
        let mut current: u32 = 0;
        let mut has_digit = false;

        for character in text.chars() {
            match character {
                '0'..='9' => {
                    let digit = u32::from(character as u8 - b'0');
                    current = current.checked_mul(10)?.checked_add(digit)?;
                    has_digit = true;
                }
                '.' => {
                    if !has_digit {
                        return None;
                    }
                    components.push(current);
                    current = 0;
                    has_digit = false;
                }
                _ => return None,
            }
        }

        if !has_digit {
            return None;
        }
        components.push(current);

        Some(Self { components })
    }
}

impl Semver {
    /// The components with trailing zeroes removed.
    ///
    /// `1.0` and `1.0.0` are the same version, so they must also be equal and
    /// hash alike; this is the form both agree on.
    fn significant(&self) -> &[u32] {
        let mut end = self.components.len();
        while end > 0 && self.components[end - 1] == 0 {
            end -= 1;
        }
        &self.components[..end]
    }
}

impl PartialEq for Semver {
    /// Equality *is* the comparison, as the C++ `operator==` defines it in
    /// terms of `<=>`. Comparing the component lists structurally instead
    /// would make `1.0` and `1.0.0` unequal but neither greater, which is a
    /// contradiction a sort can act on.
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Semver {}

impl std::hash::Hash for Semver {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.significant().hash(state);
    }
}

impl Ord for Semver {
    /// Compare positionally, treating a missing component as zero.
    ///
    /// So `1.0` and `1.0.0` are equal, and `1.0.1` is greater than both. A
    /// comparison that ordered by length first would offer `1.0.0` as an
    /// update over `1.0`.
    fn cmp(&self, other: &Self) -> Ordering {
        let count = self.components.len().max(other.components.len());
        for index in 0..count {
            let left = self.components.get(index).copied().unwrap_or(0);
            let right = other.components.get(index).copied().unwrap_or(0);
            match left.cmp(&right) {
                Ordering::Equal => {}
                ordering => return ordering,
            }
        }
        Ordering::Equal
    }
}

impl PartialOrd for Semver {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
