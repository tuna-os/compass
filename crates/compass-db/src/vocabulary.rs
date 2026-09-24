//! Typo suggestions over the index's vocabulary, in Rust.
//!
//! This replaces the vendored `spellfix1` SQLite extension (ADR-0017). The
//! vocabulary is a plain table of filename tokens and how often each occurs;
//! candidates come from an `fst` Levenshtein automaton and are measured with
//! optimal-string-alignment distance from `strsim`, so a transposition counts
//! as one edit, as a person would count it.
//!
//! Distances are reported in spellfix's units — [`EDIT_COST`] per edit — so
//! the correction policy in [`crate::query_policy`], which was tuned against
//! them, reads them unchanged: `MAX_CORRECTION_DISTANCE` of 120 still means
//! "one edit". What is deliberately *not* reproduced is spellfix's phonetic
//! candidate hash and its per-character-class substitution costs. The file
//! search quality suite (`tests/query_quality.rs`) is the bar, not spellfix's
//! exact numbers.

use fst::automaton::Levenshtein;
use fst::{Automaton, IntoStreamer, Map, Streamer};

use crate::query_policy::VocabularySuggestion;

/// The distance one edit costs, in the units the correction policy expects.
pub const EDIT_COST: i32 = 100;

/// Edits past which a word is not worth returning at all.
const MAX_EDITS: usize = 2;

/// The index's vocabulary, ready to be asked for near misses.
///
/// Candidates come from an `fst` Levenshtein automaton, so a lookup visits the
/// words within reach instead of every word; each candidate is then measured
/// with optimal-string-alignment distance, which counts a transposition as the
/// single slip it is (Levenshtein counts it as two).
#[derive(Default)]
pub struct Vocabulary {
    words: Map<Vec<u8>>,
}

impl Vocabulary {
    /// Builds from `(word, rank)` pairs. Words are expected lowercase, as the
    /// writer stores them; a repeated word keeps its highest rank.
    #[must_use]
    pub fn new(words: impl IntoIterator<Item = (String, i64)>) -> Self {
        let mut sorted: Vec<(String, i64)> = words.into_iter().collect();
        sorted.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        sorted.dedup_by(|later, earlier| later.0 == earlier.0);
        let words = Map::from_iter(
            sorted
                .into_iter()
                .map(|(word, rank)| (word, u64::try_from(rank.max(0)).unwrap_or(0))),
        )
        .unwrap_or_default();
        Self { words }
    }

    /// How many distinct words it holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.words.len()
    }

    /// Whether it holds no words.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }
}

/// The words in `vocabulary` closest to `word`, at most `top` of them.
///
/// In `prefix` mode a vocabulary word only has to *start* with something close
/// to `word`: `moderna` suggests `modernisation`, the way a person still typing
/// expects.
///
/// Ordered by distance, then by rank (more familiar first), then shorter, then
/// alphabetically, so the result is deterministic.
#[must_use]
pub fn suggest(
    vocabulary: &Vocabulary,
    word: &str,
    top: usize,
    prefix: bool,
) -> Vec<VocabularySuggestion> {
    let query = word.to_lowercase();
    let query_len = query.chars().count();
    if query_len == 0 || top == 0 {
        return Vec::new();
    }
    let Ok(automaton) = Levenshtein::new(&query, u32::try_from(MAX_EDITS).unwrap_or(2)) else {
        // Past the automaton's state budget — only for absurdly long words,
        // which no filename token near them is going to rescue.
        return Vec::new();
    };

    let mut found = Vec::new();
    let mut push = |candidate: &[u8], rank: u64| {
        let Ok(candidate) = std::str::from_utf8(candidate) else {
            return;
        };
        let Some(edits) = edits(candidate, &query, query_len, prefix) else {
            return;
        };
        let distance = i32::try_from(edits)
            .unwrap_or(i32::MAX)
            .saturating_mul(EDIT_COST);
        found.push(VocabularySuggestion {
            word: candidate.to_owned(),
            distance,
            score: distance,
            rank: i64::try_from(rank).unwrap_or(i64::MAX),
        });
    };
    if prefix {
        let mut stream = vocabulary
            .words
            .search(automaton.starts_with())
            .into_stream();
        while let Some((candidate, rank)) = stream.next() {
            push(candidate, rank);
        }
    } else {
        let mut stream = vocabulary.words.search(automaton).into_stream();
        while let Some((candidate, rank)) = stream.next() {
            push(candidate, rank);
        }
    }

    found.sort_by(|a, b| {
        a.distance
            .cmp(&b.distance)
            .then(b.rank.cmp(&a.rank))
            .then(a.word.chars().count().cmp(&b.word.chars().count()))
            .then_with(|| a.word.cmp(&b.word))
    });
    found.truncate(top);
    found
}

fn edits(candidate: &str, query: &str, query_len: usize, prefix: bool) -> Option<usize> {
    let candidate_len = candidate.chars().count();
    if !prefix {
        if candidate_len.abs_diff(query_len) > MAX_EDITS {
            return None;
        }
        return Some(strsim::osa_distance(candidate, query)).filter(|&e| e <= MAX_EDITS);
    }
    if candidate_len + MAX_EDITS < query_len {
        return None;
    }
    let shortest = query_len.saturating_sub(MAX_EDITS).max(1);
    let longest = (query_len + MAX_EDITS).min(candidate_len);
    (shortest..=longest)
        .map(|len| {
            let end = candidate
                .char_indices()
                .nth(len)
                .map_or(candidate.len(), |(i, _)| i);
            strsim::osa_distance(&candidate[..end], query)
        })
        .min()
        .filter(|&e| e <= MAX_EDITS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab(words: &[(&str, i64)]) -> Vocabulary {
        Vocabulary::new(words.iter().map(|(w, r)| ((*w).to_owned(), *r)))
    }

    fn words(found: &[VocabularySuggestion]) -> Vec<&str> {
        found.iter().map(|s| s.word.as_str()).collect()
    }

    #[test]
    fn a_transposition_is_one_edit() {
        let found = suggest(&vocab(&[("budget", 1)]), "budgte", 20, false);
        assert_eq!(found[0].word, "budget");
        assert_eq!(found[0].distance, EDIT_COST);
    }

    #[test]
    fn an_exact_word_comes_back_at_distance_zero_with_its_rank() {
        // The policy's known-word veto depends on seeing this row.
        let found = suggest(&vocab(&[("vrs", 3), ("vers", 1)]), "vrs", 20, false);
        assert_eq!(found[0].word, "vrs");
        assert_eq!(found[0].distance, 0);
        assert_eq!(found[0].rank, 3);
        assert!(words(&found).contains(&"vers"));
    }

    #[test]
    fn prefix_mode_finds_a_longer_word_being_typed() {
        let v = vocab(&[("modernisation", 1), ("modem", 1)]);
        assert_eq!(words(&suggest(&v, "moderna", 20, true)), ["modernisation"]);
        assert!(suggest(&v, "moderna", 20, false).is_empty());
    }

    #[test]
    fn closer_beats_more_familiar_and_familiar_breaks_ties() {
        let v = vocab(&[("frostwire", 1), ("firstparty", 50), ("frostwise", 9)]);
        let found = suggest(&v, "frstwire", 20, false);
        assert_eq!(found[0].word, "frostwire");
        let v = vocab(&[("reports", 1), ("reporta", 7)]);
        assert_eq!(suggest(&v, "reportx", 20, false)[0].word, "reporta");
    }

    #[test]
    fn far_words_and_empty_queries_return_nothing() {
        let v = vocab(&[("budget", 1)]);
        assert!(suggest(&v, "zzqqxxw", 20, false).is_empty());
        assert!(suggest(&v, "", 20, true).is_empty());
        assert!(suggest(&v, "budget", 0, false).is_empty());
    }

    #[test]
    fn top_limits_the_list() {
        let v = vocab(&[("sercom0", 1), ("sercom1", 1), ("sercom2", 1)]);
        assert_eq!(suggest(&v, "sercom", 2, true).len(), 2);
    }

    #[test]
    fn multibyte_words_do_not_panic() {
        let v = vocab(&[("répondre", 1)]);
        assert_eq!(suggest(&v, "répodnre", 20, false)[0].word, "répondre");
        assert!(!suggest(&v, "rép", 20, true).is_empty());
    }
}

#[cfg(test)]
mod scale {
    use super::*;

    /// A large home directory's filename vocabulary is tens of thousands of
    /// tokens. The fallback runs a full scan per lookup, twice per query word
    /// (prefix and exact), so it has to stay well inside a keystroke's budget.
    #[test]
    fn a_hundred_thousand_word_vocabulary_answers_in_milliseconds() {
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let vocabulary = Vocabulary::new((0..100_000).map(|_| {
            let len = 3 + (next() % 10) as usize;
            let word: String = (0..len)
                .map(|_| char::from(b'a' + (next() % 26) as u8))
                .collect();
            (word, (next() % 50) as i64)
        }));
        let start = std::time::Instant::now();
        for query in ["budgte", "mayonaise", "serach", "moderna"] {
            let _ = suggest(&vocabulary, query, 20, true);
            let _ = suggest(&vocabulary, query, 20, false);
        }
        let per_lookup = start.elapsed() / 8;
        eprintln!("per lookup over 100k words: {per_lookup:?}");
        // Debug build, shared CI runner: a generous ceiling that still catches
        // an accidental quadratic.
        assert!(
            per_lookup < std::time::Duration::from_millis(100),
            "{per_lookup:?}"
        );
    }
}
