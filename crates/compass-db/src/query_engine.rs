//! Scoring and ranking file-index candidates.
//!
//! Ports the scoring half of `file-indexer-query-engine.cpp`: substring
//! bonuses, filename/dirname weighting through compass-search (the crate that
//! ports the weighted fuzzy scorer), relevance penalties, the sort order,
//! parallel batch scoring, and the correction/skeleton confidence math. The
//! database orchestration (`query`, `queryWithCorrections`) lives in
//! [`crate::query_reader`], over the read trait.
//!
//! The fuzzy engine underneath is nucleo, not the C++'s fzf: absolute scores
//! differ, but every threshold here compares gated or normalized values, and
//! the boost, penalty, and ordering math is the port's own.

use std::path::{Path, PathBuf};
use std::thread;

use compass_search::{Matcher, Query, WeightedField, score_weighted};

use crate::query_policy::{CorrectionPlan, split_query_words};

/// Below this many candidates, scoring stays on one thread.
pub const SCORING_BATCH_SIZE: usize = 500;

/// The quality floor for skeleton-merged candidates.
pub const SKELETON_MIN_QUALITY: u32 = 40;

/// How many candidates one database query may return.
pub const CANDIDATE_LIMIT: usize = 10_000;

/// How many spellfix suggestions each word fetches per prefix mode.
pub const SUGGESTION_FETCH_COUNT: i32 = 20;

/// How many corrections each query word keeps.
pub const MAX_CORRECTIONS_PER_WORD: usize = 3;

/// Shorter words are never corrected.
pub const MIN_CORRECTABLE_WORD_LENGTH: usize = 3;

/// How many correction plans are searched.
pub const MAX_CORRECTION_PLANS: usize = 4;

/// Below this strict confidence, skeleton candidates merge in.
pub const SKELETON_CONFIDENCE_THRESHOLD: f64 = 0.5;

/// At this correction confidence, the correction's results win outright.
pub const CORRECTION_CONFIDENCE_THRESHOLD: f64 = 0.7;

/// At this confidence a full page of correction results still wins.
pub const CORRECTION_FULL_PAGE_CONFIDENCE_THRESHOLD: f64 = 0.5;

/// What kind of file a candidate is.
///
/// Stored in SQLite by number: the discriminants are the C++ enum order, so
/// do not reorder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexedFileCategory {
    /// Anything else.
    Other = 0,
    /// A directory.
    Directory = 1,
    /// An image.
    Image = 2,
    /// A video.
    Video = 3,
    /// Audio.
    Audio = 4,
    /// A document.
    Document = 5,
    /// An archive.
    Archive = 6,
    /// An application.
    Application = 7,
}

/// One path the database query returned, before scoring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchCandidate {
    /// What was found.
    pub path: PathBuf,
    /// What kind of file it is.
    pub category: IndexedFileCategory,
    /// Its MIME type, when known.
    pub mime_type: Option<String>,
}

/// Narrows a database query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SearchOptions {
    /// Only this category.
    pub category: Option<IndexedFileCategory>,
}

/// One ranked file-index hit.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexerFileResult {
    /// What matched.
    pub path: PathBuf,
    /// The score that ranked it.
    pub rank: f64,
    /// What kind of file it is.
    pub category: IndexedFileCategory,
    /// Its MIME type, when known.
    pub mime_type: Option<String>,
}

/// A candidate with its score.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoredCandidate {
    /// What was scored.
    pub candidate: SearchCandidate,
    /// The score, 0 when cut.
    pub score: i32,
}

/// Why skeleton candidates did or did not merge.
#[derive(Debug, Clone, PartialEq)]
pub struct SkeletonMergeDecision {
    /// Whether to merge skeleton candidates into the ranking.
    pub should_merge: bool,
    /// Machine-readable reason: `no-strict-candidates`, `strict-confident`,
    /// `low-strict-confidence`, `sparse-abbreviation-query`.
    pub reason: &'static str,
    /// The best strict path, when there was one.
    pub best_path: Option<PathBuf>,
    /// Its score.
    pub best_score: i32,
    /// Its score over 100.
    pub confidence: f64,
}

/// How the query sits inside the text: not at all, inside a token, or opening
/// one. Ordered worst to best.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SubstringMatch {
    /// No match.
    None,
    /// Matched, but mid-token.
    Inner,
    /// Matched at a token start.
    TokenStart,
}

/// Whether a byte counts toward an abbreviation's consonant skeleton.
#[must_use]
pub fn is_skeleton_vowel(byte: u8) -> bool {
    matches!(byte, b'a' | b'e' | b'i' | b'o' | b'u')
}

/// The file name: everything after the last `/`, or the whole path.
#[must_use]
pub fn basename_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The directory: everything before the last `/`, or the whole path.
#[must_use]
pub fn dirname_of(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(dir, _)| dir)
}

/// The extension: everything after the last `.`, or the whole path.
#[must_use]
pub fn file_extension_of(path: &str) -> &str {
    path.rsplit_once('.').map_or(path, |(_, ext)| ext)
}

/// Finds the query in the text, case-insensitively, reporting the best kind
/// of hit: a token start beats an inner match, and scanning continues past
/// inner matches looking for one.
#[must_use]
pub fn substring_match(text: &str, query: &str) -> SubstringMatch {
    if query.is_empty() || query.len() > text.len() {
        return SubstringMatch::None;
    }
    let text = text.to_ascii_lowercase();
    let query = query.to_ascii_lowercase();
    let (text, query) = (text.as_bytes(), query.as_bytes());

    let mut inner = false;
    let mut rest = 0;
    while rest + query.len() <= text.len() {
        let Some(offset) = text[rest..]
            .windows(query.len())
            .position(|window| window == query)
        else {
            break;
        };
        let absolute = rest + offset;
        if absolute == 0 || !text[absolute - 1].is_ascii_alphanumeric() {
            return SubstringMatch::TokenStart;
        }
        inner = true;
        rest += offset + 1;
    }

    if inner {
        SubstringMatch::Inner
    } else {
        SubstringMatch::None
    }
}

/// Whether the query is one token: non-empty with no whitespace.
#[must_use]
pub fn is_single_token_query(query: &str) -> bool {
    !query.is_empty() && !query.bytes().any(|byte| byte.is_ascii_whitespace())
}

/// Whether a word looks like an abbreviation: 3 to 5 bytes with at most one
/// vowel.
#[must_use]
pub fn is_abbreviation_like_word(word: &str) -> bool {
    if word.len() < 3 || word.len() > 5 {
        return false;
    }
    let vowels = word
        .bytes()
        .filter(|byte| is_skeleton_vowel(byte.to_ascii_lowercase()))
        .count();
    vowels <= 1
}

/// Whether any query word looks like an abbreviation.
#[must_use]
pub fn is_abbreviation_like_query(query: &str) -> bool {
    let words = split_query_words(query);
    !words.is_empty() && words.iter().any(|word| is_abbreviation_like_word(word))
}

/// Boosts single-token queries that literally contain the text: 1.5 at a
/// token start, 1.05 inside a token, 1.0 otherwise — the better of the file
/// name and the directory.
#[must_use]
pub fn substring_match_multiplier(candidate: &SearchCandidate, query: &str) -> f64 {
    if !is_single_token_query(query) {
        return 1.0;
    }
    let path = candidate.path.to_string_lossy();
    let best =
        substring_match(basename_of(&path), query).max(substring_match(dirname_of(&path), query));
    match best {
        SubstringMatch::TokenStart => 1.5,
        SubstringMatch::Inner => 1.05,
        SubstringMatch::None => 1.0,
    }
}

/// Penalizes backup, object, and editor-swap files, in the C++'s order:
/// `#...#`, then `#`-extensions, then `.o`, then trailing `~`, then swaps.
#[must_use]
pub fn file_relevance_multiplier(candidate: &SearchCandidate) -> f64 {
    let path = candidate.path.to_string_lossy();
    let extension = file_extension_of(&path);

    if path.starts_with('#') && path.ends_with('#') {
        return 0.1;
    }
    if extension.starts_with('#') {
        return 0.1;
    }
    if extension == "o" {
        return 0.5;
    }
    if path.ends_with('~') {
        return 0.3;
    }
    if extension == "swp" || extension == "swo" || extension == "swm" {
        return 0.5;
    }

    1.0
}

/// Scores a candidate against a fuzzy query: filename at full weight,
/// directory at 0.7, gated by `min_quality`, boosted by a literal substring
/// hit, penalized by file kind.
#[must_use]
pub fn score_candidate(candidate: &SearchCandidate, query: &Query, min_quality: u32) -> i32 {
    let path = candidate.path.to_string_lossy();
    let matched = score_weighted(
        &[
            WeightedField::new(basename_of(&path), 1.0),
            WeightedField::new(dirname_of(&path), 0.7),
        ],
        query,
    );
    if matched.quality < min_quality {
        return 0;
    }
    let bonus = substring_match_multiplier(candidate, query.text()) - 1.0;
    let boosted = f64::from(matched.score) + (100.0 - f64::from(matched.score)) * bonus;
    (boosted * file_relevance_multiplier(candidate)) as i32
}

/// Scores a candidate against a correction plan: each term's best field match
/// normalized against its perfect self-match, weighted by the choice, averaged
/// — or 0 the moment any term misses entirely.
#[must_use]
pub fn score_candidate_with_plan(candidate: &SearchCandidate, plan: &CorrectionPlan) -> i32 {
    Matcher::with_thread_local(|matcher| {
        let path = candidate.path.to_string_lossy();
        let filename = basename_of(&path);

        let word_score = |matcher: &mut Matcher, word: &str| -> u32 {
            let themselves = matcher.score(word, word).unwrap_or(0);
            if themselves == 0 {
                return 0;
            }
            let on_filename = matcher.score(filename, word).unwrap_or(0);
            let on_path = (f64::from(matcher.score(&path, word).unwrap_or(0)) * 0.7) as u64;
            let best = u64::from(on_filename).max(on_path);
            (best * 100 / u64::from(themselves)).min(100) as u32
        };

        let mut total = 0i64;
        let mut count = 0i64;
        for choice in &plan.choices {
            let best = (f64::from(word_score(matcher, &choice.term)) * choice.weight) as i32;
            if best <= 0 {
                return 0;
            }
            total += i64::from(best);
            count += 1;
        }
        if count == 0 {
            0
        } else {
            (total / count) as i32
        }
    })
}

/// Orders ranked candidates: score first, shorter paths before longer ones,
/// files before directories, then byte order.
pub fn sort_scored(ranked: &mut [ScoredCandidate], is_directory: impl Fn(&Path) -> bool) {
    ranked.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(
                a.candidate
                    .path
                    .as_os_str()
                    .len()
                    .cmp(&b.candidate.path.as_os_str().len()),
            )
            .then({
                // Directories sort as 0, files as 1, descending — so files come
                // first when everything else ties.
                let dir_a = u8::from(!is_directory(&a.candidate.path));
                let dir_b = u8::from(!is_directory(&b.candidate.path));
                dir_b.cmp(&dir_a)
            })
            .then(
                a.candidate
                    .path
                    .as_os_str()
                    .as_encoded_bytes()
                    .cmp(b.candidate.path.as_os_str().as_encoded_bytes()),
            )
    });
}

/// Scores every candidate, cutting zeroes, most significant first.
///
/// Batches of [`SCORING_BATCH_SIZE`] per thread past the threshold, on scoped
/// threads; the sort after the join is single-threaded either way, so the
/// order never depends on the thread count.
#[must_use]
pub fn score_candidates(
    candidates: Vec<SearchCandidate>,
    scorer: impl Fn(&SearchCandidate) -> i32 + Sync + Send,
    is_directory: impl Fn(&Path) -> bool,
) -> Vec<ScoredCandidate> {
    if candidates.len() < SCORING_BATCH_SIZE {
        let mut ranked: Vec<ScoredCandidate> = candidates
            .into_iter()
            .map(|candidate| {
                let score = scorer(&candidate);
                ScoredCandidate { candidate, score }
            })
            .filter(|scored| scored.score > 0)
            .collect();
        sort_scored(&mut ranked, is_directory);
        return ranked;
    }

    let thread_count = candidates.len().div_ceil(SCORING_BATCH_SIZE).min(
        thread::available_parallelism()
            .map_or(1, std::num::NonZero::get)
            .max(1),
    );
    let chunk_size = candidates.len().div_ceil(thread_count);
    tracing::debug!(
        count = candidates.len(),
        threads = thread_count,
        "scoring candidates"
    );

    let mut scores = vec![0i32; candidates.len()];
    // Shared by reference: the scorer runs on many threads but belongs to none.
    let scorer = &scorer;
    thread::scope(|scope| {
        for (chunk, slots) in candidates
            .chunks(chunk_size)
            .zip(scores.chunks_mut(chunk_size))
        {
            scope.spawn(move || {
                for (candidate, slot) in chunk.iter().zip(slots.iter_mut()) {
                    *slot = scorer(candidate);
                }
            });
        }
    });

    let cut = scores.iter().filter(|score| **score <= 0).count();
    let mut ranked: Vec<ScoredCandidate> = candidates
        .into_iter()
        .zip(scores)
        .map(|(candidate, score)| ScoredCandidate { candidate, score })
        .collect();
    sort_scored(&mut ranked, is_directory);
    ranked.truncate(ranked.len() - cut);
    ranked
}

/// Keeps the ranked candidates that still exist, up to `limit`, carrying score
/// as rank.
#[must_use]
pub fn results_from_ranked(
    ranked: &[ScoredCandidate],
    limit: usize,
    exists: impl Fn(&Path) -> bool,
) -> Vec<IndexerFileResult> {
    if limit == 0 {
        return Vec::new();
    }
    ranked
        .iter()
        .filter(|scored| exists(&scored.candidate.path))
        .take(limit)
        .map(|scored| IndexerFileResult {
            path: scored.candidate.path.clone(),
            rank: f64::from(scored.score),
            category: scored.candidate.category,
            mime_type: scored.candidate.mime_type.clone(),
        })
        .collect()
}

/// The score a plan would earn if every term matched perfectly.
#[must_use]
pub fn ideal_score_for_plan(plan: &CorrectionPlan) -> f64 {
    if plan.choices.is_empty() {
        return 0.0;
    }
    plan.choices
        .iter()
        .map(|choice| 100.0 * choice.weight)
        .sum::<f64>()
        / plan.choices.len() as f64
}

/// How the best result compares to that ideal: 0 with no results or no ideal.
#[must_use]
pub fn correction_confidence(ranked: &[ScoredCandidate], plan: &CorrectionPlan) -> f64 {
    let Some(best) = ranked.first() else {
        return 0.0;
    };
    let ideal = ideal_score_for_plan(plan);
    if ideal <= 0.0 {
        return 0.0;
    }
    f64::from(best.score) / ideal
}

/// Whether correction results win: confident, or a full page at half
/// confidence.
#[must_use]
pub fn accepts_correction_results(
    ranked: &[ScoredCandidate],
    plan: &CorrectionPlan,
    limit: usize,
) -> bool {
    let confidence = correction_confidence(ranked, plan);
    confidence >= CORRECTION_CONFIDENCE_THRESHOLD
        || (limit > 0
            && ranked.len() >= limit
            && confidence >= CORRECTION_FULL_PAGE_CONFIDENCE_THRESHOLD)
}

/// Whether skeleton candidates merge into a strict ranking: always with no
/// strict candidates, never when the best is confident — unless a short page
/// meets an abbreviation-like query.
#[must_use]
pub fn skeleton_merge_decision(
    ranked: &[ScoredCandidate],
    query: &str,
    limit: usize,
) -> SkeletonMergeDecision {
    let Some(best) = ranked.first() else {
        return SkeletonMergeDecision {
            should_merge: true,
            reason: "no-strict-candidates",
            best_path: None,
            best_score: 0,
            confidence: 0.0,
        };
    };
    let mut decision = SkeletonMergeDecision {
        should_merge: false,
        reason: "strict-confident",
        best_path: Some(best.candidate.path.clone()),
        best_score: best.score,
        confidence: f64::from(best.score) / 100.0,
    };
    if decision.confidence < SKELETON_CONFIDENCE_THRESHOLD {
        decision.should_merge = true;
        decision.reason = "low-strict-confidence";
    } else if ranked.len() < limit && is_abbreviation_like_query(query) {
        decision.should_merge = true;
        decision.reason = "sparse-abbreviation-query";
    }
    decision
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_policy::{CorrectionChoice, CorrectionPlan};

    fn candidate(path: &str) -> SearchCandidate {
        SearchCandidate {
            path: PathBuf::from(path),
            category: IndexedFileCategory::Other,
            mime_type: None,
        }
    }

    fn scored(path: &str, score: i32) -> ScoredCandidate {
        ScoredCandidate {
            candidate: candidate(path),
            score,
        }
    }

    fn never_a_directory(path: &Path) -> bool {
        let _ = path;
        false
    }

    #[test]
    fn a_token_start_beats_an_inner_match() {
        assert_eq!(
            substring_match("document.txt", "doc"),
            SubstringMatch::TokenStart
        );
        assert_eq!(
            substring_match("my-doc.txt", "doc"),
            SubstringMatch::TokenStart
        );
        assert_eq!(substring_match("adoc.txt", "doc"), SubstringMatch::Inner);
        assert_eq!(
            substring_match("adoc doc", "doc"),
            SubstringMatch::TokenStart
        );
        assert_eq!(substring_match("adoc", "doc"), SubstringMatch::Inner);
        assert_eq!(substring_match("txt", "doc"), SubstringMatch::None);
        assert_eq!(substring_match("doc", ""), SubstringMatch::None);
        assert_eq!(substring_match("do", "doc"), SubstringMatch::None);
    }

    #[test]
    fn matching_ignores_case() {
        assert_eq!(
            substring_match("Document.txt", "doc"),
            SubstringMatch::TokenStart
        );
    }

    #[test]
    fn single_token_means_no_whitespace() {
        assert!(is_single_token_query("doc"));
        assert!(!is_single_token_query("my doc"));
        assert!(!is_single_token_query(""));
    }

    #[test]
    fn abbreviations_are_short_and_vowelless() {
        assert!(is_abbreviation_like_word("cfg"));
        assert!(is_abbreviation_like_word("crypt"));
        assert!(
            !is_abbreviation_like_word("rhythm"),
            "six bytes is too long"
        );
        assert!(!is_abbreviation_like_word("config"));
        assert!(!is_abbreviation_like_word("hello"));
        assert!(!is_abbreviation_like_word("ab"));
        assert!(is_abbreviation_like_query("open cfg"));
        assert!(!is_abbreviation_like_query("open config"));
        assert!(!is_abbreviation_like_query(""));
    }

    #[test]
    fn the_substring_boost_prefers_token_starts() {
        let query = "doc";
        assert_eq!(
            substring_match_multiplier(&candidate("/home/ada/document.txt"), query),
            1.5
        );
        assert_eq!(
            substring_match_multiplier(&candidate("/home/ada/adoc.txt"), query),
            1.05
        );
        assert_eq!(
            substring_match_multiplier(&candidate("/home/ada/notes.txt"), query),
            1.0
        );
        assert_eq!(
            substring_match_multiplier(&candidate("/home/ada/document.txt"), "my doc"),
            1.0
        );
    }

    #[test]
    fn the_directory_counts_for_the_boost() {
        assert_eq!(
            substring_match_multiplier(&candidate("/home/ada/Documents/notes.txt"), "doc"),
            1.5
        );
    }

    #[test]
    fn backups_objects_and_swaps_are_penalized_in_order() {
        // The '#' checks read the whole path string: an absolute path never
        // starts with '#', so only a relative autosave name or a '#'-extension
        // counts — a trailing '#' alone penalizes nothing.
        assert_eq!(
            file_relevance_multiplier(&candidate("/home/ada/file#")),
            1.0
        );
        assert_eq!(
            file_relevance_multiplier(&candidate("/home/ada/#file#")),
            1.0
        );
        assert_eq!(file_relevance_multiplier(&candidate("#notes#")), 0.1);
        assert_eq!(
            file_relevance_multiplier(&candidate("/home/ada/notes.#tmp")),
            0.1
        );
        assert_eq!(
            file_relevance_multiplier(&candidate("/home/ada/main.o")),
            0.5
        );
        assert_eq!(
            file_relevance_multiplier(&candidate("/home/ada/notes~")),
            0.3
        );
        assert_eq!(
            file_relevance_multiplier(&candidate("/home/ada/notes.swp")),
            0.5
        );
        assert_eq!(
            file_relevance_multiplier(&candidate("/home/ada/notes.swo")),
            0.5
        );
        assert_eq!(
            file_relevance_multiplier(&candidate("/home/ada/notes.swm")),
            0.5
        );
        assert_eq!(
            file_relevance_multiplier(&candidate("/home/ada/notes.txt")),
            1.0
        );
        assert_eq!(file_relevance_multiplier(&candidate("Makefile")), 1.0);
    }

    #[test]
    fn views_split_at_the_last_separator() {
        assert_eq!(basename_of("/home/ada/notes.txt"), "notes.txt");
        assert_eq!(basename_of("notes.txt"), "notes.txt");
        assert_eq!(dirname_of("/home/ada/notes.txt"), "/home/ada");
        assert_eq!(dirname_of("notes.txt"), "notes.txt");
        assert_eq!(file_extension_of("notes.txt"), "txt");
        assert_eq!(file_extension_of("Makefile"), "Makefile");
    }

    #[test]
    fn the_quality_gate_cuts() {
        let query = Query::new("report");
        assert_eq!(
            score_candidate(&candidate("/home/ada/report.txt"), &query, u32::MAX),
            0
        );
        assert!(score_candidate(&candidate("/home/ada/report.txt"), &query, 0) > 0);
    }

    #[test]
    fn the_kind_penalty_survives_scoring() {
        let query = Query::new("notes");
        let plain = score_candidate(&candidate("/home/ada/notes.txt"), &query, 0);
        let backup = score_candidate(&candidate("/home/ada/notes.txt~"), &query, 0);
        assert!(plain > 0 && backup > 0 && backup < plain);
    }

    #[test]
    fn a_plan_hit_scores_and_a_miss_zeroes() {
        let plan = CorrectionPlan {
            choices: vec![CorrectionChoice {
                original: "reprot".to_owned(),
                term: "report".to_owned(),
                weight: 1.0,
                corrected: true,
            }],
        };
        assert!(score_candidate_with_plan(&candidate("/home/ada/report.txt"), &plan) > 0);
        assert_eq!(
            score_candidate_with_plan(&candidate("/home/ada/notes.txt"), &plan),
            0
        );
    }

    #[test]
    fn a_zeroed_weight_zeroes_the_plan() {
        let plan = CorrectionPlan {
            choices: vec![CorrectionChoice {
                original: "report".to_owned(),
                term: "report".to_owned(),
                weight: 0.0,
                corrected: true,
            }],
        };
        assert_eq!(
            score_candidate_with_plan(&candidate("/home/ada/report.txt"), &plan),
            0
        );
    }

    #[test]
    fn ranking_orders_score_path_length_files_then_names() {
        let mut ranked = vec![
            scored("/b.txt", 50),
            scored("/averylongpath.txt", 50),
            scored("/a.txt", 60),
            scored("/a.txt", 50),
        ];
        sort_scored(&mut ranked, never_a_directory);
        assert_eq!(
            ranked.iter().map(|s| s.score).collect::<Vec<_>>(),
            [60, 50, 50, 50]
        );
        assert_eq!(ranked[1].candidate.path, PathBuf::from("/a.txt"));
    }

    #[test]
    fn files_come_before_directories_on_a_full_tie() {
        let mut ranked = vec![scored("/aa", 50), scored("/bb", 50)];
        sort_scored(&mut ranked, |path| path == Path::new("/aa"));
        assert_eq!(ranked[0].candidate.path, PathBuf::from("/bb"));
    }

    #[test]
    fn scoring_cuts_zeroes_and_keeps_order() {
        let query = Query::new("report");
        let candidates = vec![
            candidate("/home/ada/report.txt"),
            candidate("/home/ada/zzz-no-match-here-qqq.txt"),
            candidate("/home/ada/my report draft.txt"),
        ];
        let ranked = score_candidates(
            candidates,
            |candidate| score_candidate(candidate, &query, 0),
            never_a_directory,
        );
        assert!(!ranked.is_empty());
        assert!(ranked.iter().all(|scored| scored.score > 0));
        assert!(
            ranked
                .iter()
                .zip(ranked.iter().skip(1))
                .all(|(a, b)| a.score >= b.score)
        );
    }

    #[test]
    fn parallel_scoring_agrees_with_itself() {
        let query = Query::new("report");
        let candidates: Vec<SearchCandidate> = (0..600)
            .map(|n| candidate(&format!("/home/ada/report{n}.txt")))
            .collect();
        let first = score_candidates(
            candidates.clone(),
            |candidate| score_candidate(candidate, &query, 0),
            never_a_directory,
        );
        let second = score_candidates(
            candidates,
            |candidate| score_candidate(candidate, &query, 0),
            never_a_directory,
        );
        assert_eq!(first.len(), 600);
        assert_eq!(first, second);
    }

    #[test]
    fn results_keep_what_exists_up_to_the_limit() {
        let ranked = vec![scored("/a", 90), scored("/b", 80), scored("/c", 70)];
        let results = results_from_ranked(&ranked, 2, |path| path != Path::new("/b"));
        assert_eq!(
            results.iter().map(|r| r.rank).collect::<Vec<_>>(),
            [90.0, 70.0]
        );
        assert!(results_from_ranked(&ranked, 0, |_| true).is_empty());
    }

    #[test]
    fn confidence_compares_best_against_ideal() {
        let plan = CorrectionPlan {
            choices: vec![
                CorrectionChoice {
                    original: "a".to_owned(),
                    term: "alpha".to_owned(),
                    weight: 1.0,
                    corrected: true,
                },
                CorrectionChoice {
                    original: "b".to_owned(),
                    term: "beta".to_owned(),
                    weight: 0.5,
                    corrected: true,
                },
            ],
        };
        assert!((ideal_score_for_plan(&plan) - 75.0).abs() < 1e-9);
        assert_eq!(
            ideal_score_for_plan(&CorrectionPlan { choices: vec![] }),
            0.0
        );

        let ranked = vec![scored("/x", 60)];
        assert!((correction_confidence(&ranked, &plan) - 0.8).abs() < 1e-9);
        assert_eq!(correction_confidence(&[], &plan), 0.0);

        assert!(accepts_correction_results(&ranked, &plan, 10));
        let weak = vec![scored("/x", 30)];
        assert!(!accepts_correction_results(&weak, &plan, 10));
        let full: Vec<ScoredCandidate> = (0..10).map(|n| scored(&format!("/{n}"), 45)).collect();
        assert!((correction_confidence(&full, &plan) - 0.6).abs() < 1e-9);
        assert!(accepts_correction_results(&full, &plan, 10));
    }

    #[test]
    fn the_merge_decision_reasons() {
        let empty: Vec<ScoredCandidate> = Vec::new();
        let decision = skeleton_merge_decision(&empty, "cfg", 10);
        assert!(decision.should_merge);
        assert_eq!(decision.reason, "no-strict-candidates");

        let confident = vec![scored("/home/ada/report.txt", 90)];
        let decision = skeleton_merge_decision(&confident, "report", 10);
        assert!(!decision.should_merge);
        assert_eq!(decision.reason, "strict-confident");
        assert_eq!(
            decision.best_path,
            Some(PathBuf::from("/home/ada/report.txt"))
        );
        assert!((decision.confidence - 0.9).abs() < 1e-9);

        let weak = vec![scored("/x", 30)];
        let decision = skeleton_merge_decision(&weak, "report", 10);
        assert!(decision.should_merge);
        assert_eq!(decision.reason, "low-strict-confidence");

        let sparse = vec![scored("/x", 90)];
        let decision = skeleton_merge_decision(&sparse, "cfg", 10);
        assert!(decision.should_merge);
        assert_eq!(decision.reason, "sparse-abbreviation-query");
    }
}
