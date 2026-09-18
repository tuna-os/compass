//! How the file index ranks what it found.
//!
//! Ported from `src/file-indexer/src/file-indexer-query-engine.cpp`.

use compass_core::query_policy::{CorrectionChoice, CorrectionPlan};
use compass_core::query_ranking::{
    CANDIDATE_LIMIT, RankedCandidate, SCORING_BATCH_SIZE, SKELETON_MIN_QUALITY, SubstringMatch,
    boosted_score, file_relevance_multiplier, is_abbreviation_like_query,
    is_abbreviation_like_word, is_single_token_query, keep_scoring, plan_score, plan_word_score,
    score_candidate, scores_in_parallel, scoring_thread_count, sort_candidates, substring_match,
    substring_match_multiplier,
};

fn cand(path: &str, score: i32, is_directory: bool) -> RankedCandidate {
    RankedCandidate {
        path: path.to_owned(),
        score,
        is_directory,
    }
}

fn paths(list: &[RankedCandidate]) -> Vec<&str> {
    list.iter().map(|c| c.path.as_str()).collect()
}

// --- shape of the query -------------------------------------------------

#[test]
fn a_query_with_no_spaces_is_one_token() {
    assert!(is_single_token_query("report"));
    assert!(!is_single_token_query("annual report"));
}

#[test]
fn an_empty_query_is_not_a_single_token() {
    assert!(!is_single_token_query(""));
}

#[test]
fn any_whitespace_splits_it_not_just_a_space() {
    assert!(!is_single_token_query("a\tb"));
    assert!(!is_single_token_query("a\nb"));
}

#[test]
fn short_vowel_poor_words_look_like_abbreviations() {
    // `src`, `pkg`, `cfg`, `dst` — the things people type when they mean a
    // directory rather than a word.
    for word in ["src", "pkg", "cfg", "dst", "html"] {
        assert!(is_abbreviation_like_word(word), "{word}");
    }
}

#[test]
fn a_real_word_does_not() {
    for word in ["file", "report", "notes"] {
        assert!(!is_abbreviation_like_word(word), "{word}");
    }
}

#[test]
fn two_characters_is_too_little_to_tell() {
    assert!(!is_abbreviation_like_word("js"));
    assert!(!is_abbreviation_like_word("go"));
}

#[test]
fn six_characters_is_long_enough_to_be_a_word() {
    // A vowel-poor spelling that long is more likely a real word than an
    // initialism.
    assert!(!is_abbreviation_like_word("strchr"));
}

#[test]
fn one_vowel_is_allowed_and_two_are_not() {
    assert!(is_abbreviation_like_word("dist"));
    assert!(!is_abbreviation_like_word("data"));
}

#[test]
fn one_abbreviation_anywhere_makes_the_whole_query_one() {
    assert!(is_abbreviation_like_query("annual src"));
    assert!(!is_abbreviation_like_query("annual report"));
}

// --- where the query turns up -------------------------------------------

#[test]
fn a_match_at_the_start_of_the_text_is_a_token_start() {
    assert_eq!(
        substring_match("report.pdf", "report"),
        SubstringMatch::TokenStart
    );
}

#[test]
fn a_match_after_a_separator_is_a_token_start() {
    // This is what separates `report` in `report-2024.pdf` from `report` in
    // `finalreport.pdf`.
    assert_eq!(
        substring_match("annual-report.pdf", "report"),
        SubstringMatch::TokenStart
    );
    assert_eq!(
        substring_match("annual_report.pdf", "report"),
        SubstringMatch::TokenStart
    );
}

#[test]
fn a_match_inside_a_word_is_only_an_inner_match() {
    assert_eq!(
        substring_match("finalreport.pdf", "report"),
        SubstringMatch::Inner
    );
}

#[test]
fn the_scan_keeps_looking_after_an_inner_match() {
    // A later occurrence may still be at a token start, and the best one is
    // what the caller wants.
    assert_eq!(
        substring_match("xreport-report.pdf", "report"),
        SubstringMatch::TokenStart
    );
}

#[test]
fn matching_is_case_insensitive() {
    assert_eq!(
        substring_match("Report.pdf", "report"),
        SubstringMatch::TokenStart
    );
    assert_eq!(
        substring_match("report.pdf", "REPORT"),
        SubstringMatch::TokenStart
    );
}

#[test]
fn a_query_longer_than_the_text_matches_nothing() {
    assert_eq!(substring_match("ab", "abcdef"), SubstringMatch::None);
}

#[test]
fn an_empty_query_matches_nothing() {
    assert_eq!(substring_match("report.pdf", ""), SubstringMatch::None);
}

#[test]
fn a_digit_before_the_query_is_not_a_word_boundary() {
    // The boundary test is alphanumeric, not alphabetic: `2024report` is one
    // word to a reader and should be one to the ranker.
    assert_eq!(
        substring_match("2024report.pdf", "report"),
        SubstringMatch::Inner
    );
}

#[test]
fn a_query_that_is_not_there_matches_nothing() {
    assert_eq!(
        substring_match("report.pdf", "invoice"),
        SubstringMatch::None
    );
}

// --- what a substring match is worth ------------------------------------

#[test]
fn a_token_start_is_worth_half_as_much_again() {
    assert!((substring_match_multiplier("/home/me/report.pdf", "report") - 1.5).abs() < 1e-9);
}

#[test]
fn an_inner_match_is_barely_worth_anything() {
    // 1.05 is a tie-break, not a ranking: a query buried inside a longer word
    // is weak evidence, and treating it as strong would put `finalreport.pdf`
    // above `report.pdf`.
    assert!((substring_match_multiplier("/home/me/finalreport.pdf", "report") - 1.05).abs() < 1e-9);
}

#[test]
fn a_match_in_the_directory_counts_too() {
    assert!((substring_match_multiplier("/home/me/report/x.pdf", "report") - 1.5).abs() < 1e-9);
}

#[test]
fn the_better_of_the_two_places_is_taken() {
    // An inner match in the name and a token start in the directory gives the
    // token start.
    let m = substring_match_multiplier("/home/me/report/finalreport.pdf", "report");
    assert!((m - 1.5).abs() < 1e-9);
}

#[test]
fn a_multi_word_query_gets_no_substring_bonus_at_all() {
    // Its words are matched separately and may be far apart in the path, so a
    // substring of the whole query is not a meaningful thing to look for.
    assert!(
        (substring_match_multiplier("/home/me/annual report.pdf", "annual report") - 1.0).abs()
            < 1e-9
    );
}

// --- what a file is worth on its own ------------------------------------

#[test]
fn an_ordinary_file_is_worth_its_face_value() {
    assert!((file_relevance_multiplier("/home/me/notes.md") - 1.0).abs() < 1e-9);
}

#[test]
fn an_emacs_autosave_is_pushed_right_down() {
    assert!((file_relevance_multiplier("#notes.md#") - 0.1).abs() < 1e-9);
}

#[test]
fn an_emacs_lock_file_is_caught_by_its_extension() {
    assert!((file_relevance_multiplier("/home/me/notes.#lock") - 0.1).abs() < 1e-9);
}

#[test]
fn an_object_file_is_halved() {
    assert!((file_relevance_multiplier("/home/me/main.o") - 0.5).abs() < 1e-9);
}

#[test]
fn a_backup_file_is_pushed_further_down_than_an_object_file() {
    // A `~` file is somebody's previous version; an object file is at least
    // current.
    assert!((file_relevance_multiplier("/home/me/notes.md~") - 0.3).abs() < 1e-9);
}

#[test]
fn every_vim_swap_extension_is_halved() {
    for ext in ["swp", "swo", "swm"] {
        let path = format!("/home/me/.notes.md.{ext}");
        assert!(
            (file_relevance_multiplier(&path) - 0.5).abs() < 1e-9,
            "{ext}"
        );
    }
}

#[test]
fn these_files_are_demoted_rather_than_removed() {
    // An editor swap file *is* sometimes what you are looking for, right
    // after a crash, and a search that cannot find it is worse than one that
    // ranks it last.
    for path in ["#a#", "a.o", "a~", "a.swp"] {
        assert!(file_relevance_multiplier(path) > 0.0, "{path}");
    }
}

// --- applying the bonus -------------------------------------------------

#[test]
fn no_bonus_leaves_a_score_alone() {
    assert!((boosted_score(40.0, 1.0) - 40.0).abs() < 1e-9);
}

#[test]
fn the_bonus_applies_to_the_remaining_headroom() {
    // 40 + (100 - 40) * 0.5 = 70.
    assert!((boosted_score(40.0, 1.5) - 70.0).abs() < 1e-9);
}

#[test]
fn a_strong_match_gains_almost_nothing_from_the_bonus() {
    // Which is the point: the bonus re-orders the middle of the list without
    // letting a weak match overtake a strong one.
    let strong = boosted_score(95.0, 1.5) - 95.0;
    let weak = boosted_score(40.0, 1.5) - 40.0;
    assert!(strong < weak / 5.0, "{strong} vs {weak}");
}

#[test]
fn the_bonus_cannot_push_a_score_past_a_hundred() {
    assert!(boosted_score(100.0, 1.5) <= 100.0);
    assert!(boosted_score(99.0, 1.5) <= 100.0);
}

#[test]
fn a_candidate_below_the_quality_floor_scores_nothing() {
    // The floor is on the quality the matcher reports, not on the score, so a
    // short path matching weakly is cut for matching weakly rather than for
    // being short.
    assert_eq!(score_candidate("/a/b.md", 90.0, 10, 40, "b"), 0);
}

#[test]
fn a_candidate_at_the_floor_is_kept() {
    assert!(score_candidate("/a/b.md", 90.0, 40, 40, "b") > 0);
}

#[test]
fn a_demoted_file_is_scored_and_then_demoted() {
    let ordinary = score_candidate("/a/main.md", 80.0, 90, 40, "main");
    let object = score_candidate("/a/main.o", 80.0, 90, 40, "main");
    assert!(object < ordinary);
    assert!(object > 0);
}

#[test]
fn the_skeleton_floor_is_forty() {
    assert_eq!(SKELETON_MIN_QUALITY, 40);
}

// --- scoring a correction plan ------------------------------------------

#[test]
fn a_word_is_normalised_against_what_it_scores_against_itself() {
    // Without it a six-letter word would always outscore a three-letter one
    // on the same quality of match.
    assert_eq!(plan_word_score(50.0, 0.0, 100.0), 50);
    assert_eq!(plan_word_score(50.0, 0.0, 50.0), 100);
}

#[test]
fn a_match_in_the_path_is_discounted_against_one_in_the_filename() {
    // 100 * 0.7 = 70, against 80 in the filename.
    assert_eq!(plan_word_score(80.0, 100.0, 100.0), 80);
    // 100 * 0.7 = 70, against 60 in the filename.
    assert_eq!(plan_word_score(60.0, 100.0, 100.0), 70);
}

#[test]
fn a_word_that_cannot_score_against_itself_scores_nothing() {
    assert_eq!(plan_word_score(90.0, 90.0, 0.0), 0);
}

#[test]
fn the_normalised_score_is_capped_at_a_hundred() {
    // The normalisation can exceed it when a word matches better than it
    // matches itself.
    assert_eq!(plan_word_score(200.0, 0.0, 100.0), 100);
}

fn plan(terms: &[(&str, f64)]) -> CorrectionPlan {
    CorrectionPlan {
        choices: terms
            .iter()
            .map(|(term, weight)| CorrectionChoice {
                original: (*term).to_owned(),
                term: (*term).to_owned(),
                weight: *weight,
                corrected: *weight < 1.0,
            })
            .collect(),
    }
}

#[test]
fn a_plan_averages_its_words_rather_than_summing_them() {
    // So a three-word plan is not worth three times a one-word plan.
    let p = plan(&[("a", 1.0), ("b", 1.0), ("c", 1.0)]);
    assert_eq!(plan_score(&p, &[60, 60, 60]), 60);
}

#[test]
fn one_word_scoring_nothing_drops_the_whole_plan() {
    // A plan is an *and*: a correction finding files that match two of its
    // three words is not a reading of the query, it is a different query.
    let p = plan(&[("a", 1.0), ("b", 1.0)]);
    assert_eq!(plan_score(&p, &[90, 0]), 0);
}

#[test]
fn a_weight_that_zeroes_a_word_drops_the_plan_too() {
    let p = plan(&[("a", 1.0), ("b", 0.0)]);
    assert_eq!(plan_score(&p, &[90, 90]), 0);
}

#[test]
fn a_corrected_word_is_worth_less_than_an_exact_one() {
    let exact = plan(&[("a", 1.0)]);
    let corrected = plan(&[("a", 0.6)]);
    assert!(plan_score(&corrected, &[100]) < plan_score(&exact, &[100]));
}

#[test]
fn an_empty_plan_scores_nothing() {
    assert_eq!(plan_score(&CorrectionPlan::default(), &[]), 0);
}

// --- the order of the results -------------------------------------------

#[test]
fn a_higher_score_comes_first() {
    let mut list = vec![cand("/a", 10, false), cand("/b", 90, false)];
    sort_candidates(&mut list);
    assert_eq!(paths(&list), ["/b", "/a"]);
}

#[test]
fn between_equal_scores_the_shorter_path_wins() {
    // `~/notes.md` over `~/archive/2019/old/notes.md`: between two equally
    // good matches the shorter path is the more likely one.
    let mut list = vec![
        cand("/home/me/archive/2019/notes.md", 90, false),
        cand("/home/me/notes.md", 90, false),
    ];
    sort_candidates(&mut list);
    assert_eq!(paths(&list)[0], "/home/me/notes.md");
}

#[test]
fn a_file_comes_before_a_directory_of_the_same_length_and_score() {
    // A directory matching as well as a file inside it is usually not what
    // was meant, because the file is the thing you open.
    let mut list = vec![cand("/a/bb", 90, true), cand("/a/cc", 90, false)];
    sort_candidates(&mut list);
    assert_eq!(paths(&list), ["/a/cc", "/a/bb"]);
}

#[test]
fn the_path_itself_breaks_the_last_tie() {
    // So the order is total and two runs agree.
    let mut list = vec![cand("/a/z", 90, false), cand("/a/a", 90, false)];
    sort_candidates(&mut list);
    assert_eq!(paths(&list), ["/a/a", "/a/z"]);
}

#[test]
fn the_keys_are_applied_in_order() {
    // A longer path with a higher score still comes first: length never beats
    // score.
    let mut list = vec![
        cand("/a", 10, false),
        cand("/home/me/archive/notes.md", 90, false),
    ];
    sort_candidates(&mut list);
    assert_eq!(paths(&list)[0], "/home/me/archive/notes.md");
}

#[test]
fn candidates_that_scored_nothing_are_dropped() {
    // They are scored and then removed rather than skipped, because the
    // scoring runs in parallel over a fixed-size buffer.
    let kept = keep_scoring(vec![cand("/a", 0, false), cand("/b", 1, false)]);
    assert_eq!(paths(&kept), ["/b"]);
}

// --- how the work is divided --------------------------------------------

#[test]
fn a_small_batch_is_scored_inline() {
    // Starting a thread to score 200 paths costs more than scoring them.
    assert!(!scores_in_parallel(SCORING_BATCH_SIZE - 1));
    assert!(scores_in_parallel(SCORING_BATCH_SIZE));
}

#[test]
fn one_thread_is_started_per_batch() {
    assert_eq!(scoring_thread_count(1_000, 16), 2);
    assert_eq!(scoring_thread_count(1_001, 16), 3);
}

#[test]
fn the_thread_count_never_exceeds_the_cores() {
    assert_eq!(scoring_thread_count(10_000, 4), 4);
}

#[test]
fn a_machine_reporting_no_cores_still_gets_one_thread() {
    // `hardware_concurrency` is allowed to return 0.
    assert_eq!(scoring_thread_count(10_000, 0), 1);
}

#[test]
fn the_index_hands_back_at_most_ten_thousand_rows() {
    assert_eq!(CANDIDATE_LIMIT, 10_000);
}
