//! How the file index ranks what it found.
//!
//! Ports the scoring half of `src/file-indexer/src/file-indexer-query-engine.cpp`
//! — the multipliers applied to a fuzzy score, the tie-breaks between equally
//! scored paths, and how a spelling-correction plan is scored across its words.
//!
//! The fuzzy match itself is [`compass_search`]'s, and the scores it produces
//! are passed in rather than computed here: what this module holds is
//! everything the engine does *around* that number, which is where the
//! decisions are.

use crate::query_policy::CorrectionPlan;
use crate::vocabulary::{basename, dirname, file_extension, is_skeleton_vowel};

/// How many candidates one thread scores before another is started.
pub const SCORING_BATCH_SIZE: usize = 500;

/// The lowest fuzzy quality a skeleton match may have.
pub const SKELETON_MIN_QUALITY: i32 = 40;

/// How many rows the index will hand back to be scored.
pub const CANDIDATE_LIMIT: usize = 10_000;

/// Whether the query is one word.
///
/// Several of the multipliers apply only to a single-word query, because a
/// substring of a multi-word query is not a meaningful thing to look for: the
/// words are matched separately and may be far apart in the path.
#[must_use]
pub fn is_single_token_query(query: &str) -> bool {
    !query.is_empty() && !query.chars().any(char::is_whitespace)
}

/// Whether a word looks like an abbreviation rather than a word.
///
/// Three to five characters with at most one vowel. `src`, `pkg`, `cfg` and
/// `dst` pass; `file` and `report` do not. The bounds are both needed: two
/// characters is too little to tell, and six is long enough that a vowel-poor
/// spelling is more likely a real word than an initialism.
#[must_use]
pub fn is_abbreviation_like_word(word: &str) -> bool {
    let len = word.len();
    if !(3..=5).contains(&len) {
        return false;
    }
    let vowels = word
        .bytes()
        .filter(|b| is_skeleton_vowel(b.to_ascii_lowercase()))
        .count();
    vowels <= 1
}

/// Whether *any* word of the query looks like an abbreviation.
#[must_use]
pub fn is_abbreviation_like_query(query: &str) -> bool {
    crate::query_policy::split_query_words(query)
        .iter()
        .any(|word| is_abbreviation_like_word(word))
}

/// Where in a string the query turned up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SubstringMatch {
    /// Not at all.
    None,
    /// Inside a word.
    Inner,
    /// At the start of a word.
    TokenStart,
}

/// Find the best place the query occurs in the text, case-insensitively.
///
/// A match at the very start of the text, or immediately after anything that
/// is not a letter or digit, counts as the start of a token — that is what
/// separates `report` in `report-2024.pdf` from `report` in `finalreport.pdf`.
/// The scan keeps looking after an inner match, because a later occurrence may
/// still be at a token start, and the best one is what the caller wants.
#[must_use]
pub fn substring_match(text: &str, query: &str) -> SubstringMatch {
    // The empty check is load-bearing: `find("")` succeeds at offset 0, which
    // would make every text a token-start match. The C++ also guards a query
    // longer than the text; that one is left out here because the loop's own
    // bound already covers it, and a line no mutation can reach is worse than
    // no line.
    if query.is_empty() {
        return SubstringMatch::None;
    }
    let lower_text = text.to_lowercase();
    let lower_query = query.to_lowercase();
    let bytes = lower_text.as_bytes();
    let mut has_inner = false;
    let mut from = 0usize;

    while from <= lower_text.len().saturating_sub(lower_query.len()) {
        let Some(offset) = lower_text[from..].find(&lower_query) else {
            break;
        };
        let at = from + offset;
        if at == 0 || !bytes[at - 1].is_ascii_alphanumeric() {
            return SubstringMatch::TokenStart;
        }
        has_inner = true;
        from = at + 1;
    }

    if has_inner {
        SubstringMatch::Inner
    } else {
        SubstringMatch::None
    }
}

/// How much a substring match is worth.
///
/// Only for a single-word query. A token-start match is worth half as much
/// again; an inner one barely anything — 1.05 is a tie-break rather than a
/// ranking, because a query buried inside a longer word is weak evidence and
/// treating it as strong would put `finalreport.pdf` above `report.pdf`.
#[must_use]
pub fn substring_match_multiplier(path: &str, query: &str) -> f64 {
    if !is_single_token_query(query) {
        return 1.0;
    }
    let best = substring_match(basename(path), query).max(substring_match(dirname(path), query));
    match best {
        SubstringMatch::TokenStart => 1.5,
        SubstringMatch::Inner => 1.05,
        SubstringMatch::None => 1.0,
    }
}

/// How much a file is worth on its own, before any query.
///
/// Every one of these is a file somebody's editor or compiler left behind. The
/// multipliers push them down rather than removing them: an editor swap file
/// *is* sometimes what you are looking for, right after a crash, and a search
/// that cannot find it at all is worse than one that ranks it last.
#[must_use]
pub fn file_relevance_multiplier(path: &str) -> f64 {
    let ext = file_extension(path);

    // Emacs autosave: `#file#`. Tested on the whole path, so it only catches
    // one whose *last* character is a hash — which is what the name looks
    // like.
    if path.starts_with('#') && path.ends_with('#') {
        return 0.1;
    }
    if ext.starts_with('#') {
        return 0.1;
    }
    if ext == "o" {
        return 0.5;
    }
    if path.ends_with('~') {
        return 0.3;
    }
    if ext == "swp" || ext == "swo" || ext == "swm" {
        return 0.5;
    }
    1.0
}

/// Apply the substring bonus to a fuzzy score.
///
/// The bonus is applied to the *remaining headroom* — `score + (100 - score) *
/// bonus` — rather than multiplying the score. A candidate already near 100
/// therefore gains almost nothing and one at 40 gains a lot, so the bonus
/// re-orders the middle of the list without letting a weak match overtake a
/// strong one. It also cannot push anything past 100.
#[must_use]
pub fn boosted_score(score: f64, multiplier: f64) -> f64 {
    let bonus = multiplier - 1.0;
    score + (100.0 - score) * bonus
}

/// Score one candidate.
///
/// Below the quality floor the candidate scores zero and is dropped — the
/// floor is on the *quality* the matcher reports, not on the score, so a short
/// path matching weakly is cut for matching weakly rather than for being
/// short.
#[must_use]
pub fn score_candidate(
    path: &str,
    fuzzy_score: f64,
    quality: i32,
    min_quality: i32,
    query: &str,
) -> i32 {
    if quality < min_quality {
        return 0;
    }
    let boosted = boosted_score(fuzzy_score, substring_match_multiplier(path, query));
    (boosted * file_relevance_multiplier(path)) as i32
}

/// Score one word of a correction plan against a candidate.
///
/// The raw fuzzy score is normalised against what the word scores *against
/// itself*, so a long word and a short one are comparable — without it a
/// six-letter word would always outscore a three-letter one on the same
/// quality of match.
///
/// A match in the path is discounted to 70% of one in the filename, and the
/// better of the two is taken. The result is capped at 100, because the
/// normalisation can exceed it when a word matches better than it matches
/// itself.
#[must_use]
pub fn plan_word_score(filename_score: f64, path_score: f64, self_score: f64) -> i32 {
    if self_score <= 0.0 {
        return 0;
    }
    let on_path = path_score * 0.7;
    let best = filename_score.max(on_path);
    ((best * 100.0 / self_score) as i32).min(100)
}

/// Score a whole correction plan.
///
/// Every word must contribute something: one word scoring zero drops the whole
/// plan. That is what makes a plan an *and* — a correction that finds files
/// matching two of its three words is not a reading of the query, it is a
/// different query.
///
/// The surviving scores are weighted by how much each choice is trusted and
/// then **averaged**, not summed, so a three-word plan is not worth three
/// times a one-word plan.
#[must_use]
pub fn plan_score(plan: &CorrectionPlan, word_scores: &[i32]) -> i32 {
    if plan.choices.len() != word_scores.len() {
        return 0;
    }
    let mut total = 0i32;
    let mut count = 0i32;
    for (choice, score) in plan.choices.iter().zip(word_scores) {
        let weighted = (f64::from(*score) * choice.weight) as i32;
        if weighted <= 0 {
            return 0;
        }
        total += weighted;
        count += 1;
    }
    if count == 0 {
        return 0;
    }
    total / count
}

/// A candidate, as the sort sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedCandidate {
    /// Its path.
    pub path: String,
    /// Its score.
    pub score: i32,
    /// Whether it is a directory.
    pub is_directory: bool,
}

/// Order the scored candidates.
///
/// Four keys, in order, and each one exists because the one before it ties:
///
/// 1. Score, descending.
/// 2. Path **length**, ascending. Between two equally good matches the shorter
///    path is the more likely one — `~/notes.md` over
///    `~/archive/2019/old/notes.md`.
/// 3. Files before directories. A directory matching as well as a file inside
///    it is usually not what was meant, because the file is the thing you open.
/// 4. The path itself, so the order is total and two runs agree.
pub fn sort_candidates(candidates: &mut [RankedCandidate]) {
    candidates.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.path.len().cmp(&b.path.len()))
            .then_with(|| a.is_directory.cmp(&b.is_directory))
            .then_with(|| a.path.cmp(&b.path))
    });
}

/// Drop the candidates that scored nothing.
///
/// They are scored and then removed rather than skipped, because the scoring
/// runs in parallel over a fixed-size buffer: leaving a hole would mean
/// synchronising the writers.
#[must_use]
pub fn keep_scoring(candidates: Vec<RankedCandidate>) -> Vec<RankedCandidate> {
    candidates.into_iter().filter(|c| c.score > 0).collect()
}

/// How many threads score a batch of candidates.
///
/// One thread per [`SCORING_BATCH_SIZE`] candidates, capped at the number of
/// cores. Below one batch the work is done inline instead — starting a thread
/// to score 200 paths costs more than scoring them.
#[must_use]
pub fn scoring_thread_count(candidate_count: usize, cores: usize) -> usize {
    let wanted = candidate_count.div_ceil(SCORING_BATCH_SIZE);
    wanted.min(cores.max(1))
}

/// Whether a batch is scored in parallel at all.
#[must_use]
pub const fn scores_in_parallel(candidate_count: usize) -> bool {
    candidate_count >= SCORING_BATCH_SIZE
}
