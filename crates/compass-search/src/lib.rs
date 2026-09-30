//! Fuzzy search for compass: a matcher, a weighted-field searchable trait, and
//! a deterministic ranker.
//!
//! This is a Rust port of the C++ `src/lib/fuzzy` library. The *semantics* are
//! ported — case- and diacritic-insensitive matching, cross-script
//! transliteration, per-field weights, a quality gate for filtering, and a
//! stable ranking order. The scoring *algorithm* is [`nucleo_matcher`]'s rather
//! than the C++ side's fzf-v2 port, so absolute score values differ between the
//! two implementations; only relative order is a contract.

#![deny(missing_docs)]

mod coherence;
mod highlight;
mod matcher;
mod query;
mod rank;
mod searchable;
mod translit;
mod typo;

pub use coherence::is_coherent;
pub use highlight::term_ranges;
pub use matcher::{MatchResult, Matcher};
pub use query::{Query, Variant, Word};
pub use rank::{
    BiasedScored, RankOptions, Scored, rank, rank_indices, rank_indices_sequential,
    rank_indices_with_options, rank_indices_with_query, rank_indices_with_query_and_options,
    rank_with_bias, rank_with_options, rank_with_query, rank_with_query_and_bias,
    rank_with_query_and_options,
};
pub use searchable::{
    FRECENCY_WEIGHT, FuzzySearchable, MIN_QUALITY, Match, WeightedField, frecency, score_item,
    score_weighted, score_weighted_with,
};
pub use translit::{TranslitScheme, needs_transliteration, transliterate, transliterate_char};
pub use typo::{MAX_TYPO_EDITS, MIN_TYPO_QUERY_CHARS, typo_distance};
