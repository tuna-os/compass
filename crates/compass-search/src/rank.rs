//! Ranking a list of [`FuzzySearchable`] items against a query.
//!
//! Port of `fuzzy::fuzzyFilter` / `fuzzy::FuzzyScorer` (`fuzzy-searchable.hpp`,
//! `weighted-fuzzy-scorer.hpp`).

use crate::matcher::Matcher;
use crate::query::Query;
use crate::searchable::{FuzzySearchable, Match, WeightedField, score_weighted_with};

/// Options controlling which fuzzy matches enter a ranking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RankOptions {
    /// Minimum accepted quality of the worst-matched query word.
    ///
    /// A value above 100 intentionally rejects every non-empty-query match.
    /// Empty queries are never filtered by this threshold.
    pub min_quality: u32,
}

impl Default for RankOptions {
    fn default() -> Self {
        Self {
            min_quality: crate::MIN_QUALITY,
        }
    }
}

/// An item paired with its score and its position in the input slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scored<T> {
    /// The scored value.
    pub item: T,
    /// Field-weighted score in `0..=100`.
    pub score: u32,
    /// Quality of the worst-matched query word, in `0..=100`.
    pub quality: u32,
    /// Raw weighted score; the tiebreak between items whose `score` is equal
    /// (notably both capped at 100). See [`Match::weighted`].
    pub weighted: u32,
    /// Index of the item in the input slice; the ranking tiebreak.
    pub index: usize,
}

/// An item ranked by its fuzzy score plus a caller-provided bias.
///
/// Launch history is the first consumer, but the type deliberately does not
/// know what the bias means. Keeping the combination and its deterministic
/// tiebreaks here prevents every caller from growing a subtly different copy
/// of the launcher ranking rule.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BiasedScored<T> {
    /// The scored value.
    pub item: T,
    /// Combined score: `match_score + bias`.
    pub score: f64,
    /// The field-weighted fuzzy score in `0..=100`, before the bias.
    pub match_score: u32,
    /// Quality of the worst-matched query word, in `0..=100`.
    pub quality: u32,
    /// Raw weighted matcher score; the tiebreak between equal combined scores.
    pub weighted: u32,
    /// The caller-provided boost added to `match_score`.
    pub bias: f64,
    /// Index of the item in the input slice; the final ranking tiebreak.
    pub index: usize,
}

/// Ranks `items` against `query`, best first.
///
/// Non-matching items (those failing the default
/// [`MIN_QUALITY`](crate::MIN_QUALITY) gate) are dropped. An empty query keeps
/// every item, in input order, with score 0 — matching the C++ `fuzzyFilter`.
///
/// The order is **total and deterministic**: descending score, then ascending
/// input index. Equal-scoring items therefore keep their input order and two
/// runs over the same input always produce the same sequence.
pub fn rank<'a, T: FuzzySearchable>(query: &str, items: &'a [T]) -> Vec<Scored<&'a T>> {
    rank_with_options(query, items, RankOptions::default())
}

/// [`rank`] with an explicit quality threshold.
pub fn rank_with_options<'a, T: FuzzySearchable>(
    query: &str,
    items: &'a [T],
    options: RankOptions,
) -> Vec<Scored<&'a T>> {
    let parsed = Query::new(query);
    rank_with_query_and_options(&parsed, items, options)
}

/// [`rank`], but with a query parsed once and reused across calls.
pub fn rank_with_query<'a, T: FuzzySearchable>(
    query: &Query,
    items: &'a [T],
) -> Vec<Scored<&'a T>> {
    rank_with_query_and_options(query, items, RankOptions::default())
}

/// [`rank_with_options`], but with a query parsed once and reused across calls.
pub fn rank_with_query_and_options<'a, T: FuzzySearchable>(
    query: &Query,
    items: &'a [T],
    options: RankOptions,
) -> Vec<Scored<&'a T>> {
    let scored = rank_indices_with_query_and_options(query, items, options);
    scored
        .into_iter()
        .map(|s| Scored {
            item: &items[s.index],
            score: s.score,
            quality: s.quality,
            weighted: s.weighted,
            index: s.index,
        })
        .collect()
}

/// Ranks `items` by fuzzy match quality plus a caller-provided bias.
///
/// The bias is added to the normalized fuzzy score and may be negative. It
/// must be finite. Non-matches are still filtered before the callback can
/// influence ordering, so a large bias cannot resurrect an unrelated item.
/// For an empty query every item is retained and ranked by bias alone.
///
/// Ordering is deterministic: combined score descending, raw weighted score
/// descending, then input index ascending.
pub fn rank_with_bias<'a, T, B>(query: &str, items: &'a [T], bias: B) -> Vec<BiasedScored<&'a T>>
where
    T: FuzzySearchable,
    B: Fn(&T) -> f64,
{
    let parsed = Query::new(query);
    rank_with_query_and_bias(&parsed, items, bias)
}

/// [`rank_with_bias`], with a query parsed once and reused across calls.
pub fn rank_with_query_and_bias<'a, T, B>(
    query: &Query,
    items: &'a [T],
    bias: B,
) -> Vec<BiasedScored<&'a T>>
where
    T: FuzzySearchable,
    B: Fn(&T) -> f64,
{
    let mut out: Vec<_> = rank_indices_with_query(query, items)
        .into_iter()
        .map(|matched| {
            let item = &items[matched.index];
            let item_bias = bias(item);
            assert!(
                item_bias.is_finite(),
                "ranking bias must be finite, got {item_bias} for input index {}",
                matched.index
            );
            BiasedScored {
                item,
                score: f64::from(matched.score) + item_bias,
                match_score: matched.score,
                quality: matched.quality,
                weighted: matched.weighted,
                bias: item_bias,
                index: matched.index,
            }
        })
        .collect();

    out.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(b.weighted.cmp(&a.weighted))
            .then(a.index.cmp(&b.index))
    });
    out
}

/// [`rank`], returning indices into `items` instead of borrows.
pub fn rank_indices<T: FuzzySearchable>(query: &str, items: &[T]) -> Vec<Scored<usize>> {
    rank_indices_with_options(query, items, RankOptions::default())
}

/// [`rank_indices`] with an explicit quality threshold.
pub fn rank_indices_with_options<T: FuzzySearchable>(
    query: &str,
    items: &[T],
    options: RankOptions,
) -> Vec<Scored<usize>> {
    let parsed = Query::new(query);
    rank_indices_with_query_and_options(&parsed, items, options)
}

/// [`rank_indices`], with a pre-parsed query.
pub fn rank_indices_with_query<T: FuzzySearchable>(
    query: &Query,
    items: &[T],
) -> Vec<Scored<usize>> {
    rank_indices_with_query_and_options(query, items, RankOptions::default())
}

/// [`rank_indices_with_options`], with a pre-parsed query.
pub fn rank_indices_with_query_and_options<T: FuzzySearchable>(
    query: &Query,
    items: &[T],
    options: RankOptions,
) -> Vec<Scored<usize>> {
    let mut out: Vec<Scored<usize>> = Vec::with_capacity(items.len());

    if query.is_empty() {
        out.extend(items.iter().enumerate().map(|(index, _)| Scored {
            item: index,
            score: 0,
            quality: 0,
            weighted: 0,
            index,
        }));
        return out;
    }

    Matcher::with_thread_local(|matcher| {
        let mut fields: Vec<WeightedField<'_>> = Vec::new();
        for (index, item) in items.iter().enumerate() {
            fields.clear();
            item.fuzzy_fields(&mut fields);
            let Match {
                score,
                quality,
                weighted,
            } = score_weighted_with(matcher, &fields, query);
            // `quality == 0` can mean either an incoherent fuzzy match or no
            // match at all. `weighted` distinguishes them, which matters when
            // a caller deliberately lowers the quality threshold to zero.
            if weighted > 0 && quality >= options.min_quality {
                out.push(Scored {
                    item: index,
                    score,
                    quality,
                    weighted,
                    index,
                });
            }
        }
    });

    // Total order: normalized score descending, then raw weighted score
    // descending (the normalized score saturates at 100, so it alone leaves
    // many ties), then input index ascending. No two entries compare equal, so
    // the order is deterministic even with an unstable sort.
    out.sort_unstable_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(b.weighted.cmp(&a.weighted))
            .then(a.index.cmp(&b.index))
    });
    out
}
