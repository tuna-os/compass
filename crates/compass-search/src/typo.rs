//! A one-slip fallback for queries the subsequence matcher cannot reach.
//!
//! nucleo matches a query as an ordered subsequence, which absorbs a dropped
//! character but not a transposed (`alacrtity`), doubled (`alacrittyy`) or
//! substituted one: each of those leaves a query character with nothing to
//! match, and the candidate vanishes. This measures the slip instead, as an
//! optimal-string-alignment distance from `strsim`, against every word of the
//! candidate.
//!
//! It is deliberately narrow. One edit, and only for queries of at least
//! [`MIN_TYPO_QUERY_CHARS`] characters: a shorter query is one edit away from
//! half the catalogue. Callers rank these hits after every real match, so a
//! fallback can add an answer but never displace one.

/// The shortest query, in characters, the fallback considers.
pub const MIN_TYPO_QUERY_CHARS: usize = 5;

/// The largest slip the fallback forgives, in edits.
pub const MAX_TYPO_EDITS: usize = 1;

/// Edits between `query` and the closest word of `candidate`, when that is a
/// forgivable slip.
///
/// The query is compared against each word's prefix of the query's own length,
/// one shorter and one longer, so a slip early in a long name is found while
/// the user is still typing it. Case-insensitive. `None` for a multi-word or
/// too-short query, or when nothing is within [`MAX_TYPO_EDITS`].
#[must_use]
pub fn typo_distance(candidate: &str, query: &str) -> Option<usize> {
    let query = query.trim().to_lowercase();
    let query_chars = query.chars().count();
    if query_chars < MIN_TYPO_QUERY_CHARS || query.contains(char::is_whitespace) {
        return None;
    }
    candidate
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .filter_map(|word| {
            let word = word.to_lowercase();
            (query_chars - 1..=query_chars + 1)
                .filter_map(|len| {
                    let end = word.char_indices().nth(len).map_or(word.len(), |(i, _)| i);
                    (word[..end].chars().count() == len)
                        .then(|| strsim::osa_distance(&word[..end], &query))
                })
                .min()
        })
        .min()
        .filter(|&edits| edits <= MAX_TYPO_EDITS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_of_single_slip_is_forgiven() {
        assert_eq!(typo_distance("Alacritty", "alacrtity"), Some(1));
        assert_eq!(typo_distance("Alacritty", "alacrittyy"), Some(1));
        assert_eq!(typo_distance("Blender", "blemder"), Some(1));
        assert_eq!(typo_distance("HandBrake", "handbrkae"), Some(1));
    }

    #[test]
    fn a_slip_in_a_later_word_or_a_partial_word_is_found() {
        assert_eq!(typo_distance("GNU Image Manipulation", "imgae"), Some(1));
        assert_eq!(typo_distance("Thunderbird", "thnuder"), Some(1));
    }

    #[test]
    fn two_slips_are_not_a_typo() {
        assert_eq!(typo_distance("Alacritty", "alcartity"), None);
    }

    #[test]
    fn short_and_multi_word_queries_are_left_to_the_matcher() {
        assert_eq!(typo_distance("Blender", "bldn"), None);
        assert_eq!(typo_distance("Visual Studio Code", "visual studo"), None);
    }

    #[test]
    fn an_exact_word_is_distance_zero() {
        assert_eq!(typo_distance("Files", "files"), Some(0));
    }

    #[test]
    fn multibyte_names_do_not_panic() {
        assert_eq!(typo_distance("Déjà Dup", "dejaa"), None);
        assert_eq!(typo_distance("Déjà Dup", "déjàà"), Some(1));
    }
}
