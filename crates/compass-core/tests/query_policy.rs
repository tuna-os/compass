//! How a typed query becomes the questions the file index is asked.
//!
//! Read off `file_indexer::query`
//! (`src/file-indexer/src/file-indexer-query-policy.cpp`).

use compass_core::query_policy::{
    CORRECTION_PENALTY, CorrectionChoice, CorrectionPlan, MAX_CORRECTION_DISTANCE,
    MIN_CORRECTION_TERM_LENGTH, QueryWord, RANK_LOG_WEIGHT, TRUSTED_WORD_MIN_RANK,
    VocabularySuggestion, adjusted_suggestion_score, build_correction_plans, correction_weight,
    pick_corrections, prepare_candidate_search_query, prepare_correction_search_query, same_family,
    split_query_words, stem,
};

/// A suggestion of `word` at `distance`.
fn suggestion(word: &str, distance: i32) -> VocabularySuggestion {
    VocabularySuggestion {
        word: word.to_owned(),
        distance,
        score: 100,
        rank: 1,
    }
}

/// A query word with the given corrections.
fn query_word(word: &str, corrections: &[&str]) -> QueryWord {
    QueryWord {
        word: word.to_owned(),
        corrections: corrections
            .iter()
            .enumerate()
            .map(|(index, text)| suggestion(text, index as i32 + 1))
            .collect(),
    }
}

#[test]
fn the_constants_are_the_cpp_ones() {
    assert_eq!(RANK_LOG_WEIGHT, 10.0);
    assert_eq!(TRUSTED_WORD_MIN_RANK, 3);
    assert_eq!(CORRECTION_PENALTY, 0.85);
    assert_eq!(MAX_CORRECTION_DISTANCE, 120);
    assert_eq!(MIN_CORRECTION_TERM_LENGTH, 3);
}

#[test]
fn a_query_splits_into_words() {
    assert_eq!(split_query_words("annual report"), vec!["annual", "report"]);
}

#[test]
fn runs_of_spaces_and_edge_spaces_produce_no_empty_words() {
    assert_eq!(split_query_words("  a   b  "), vec!["a", "b"]);
    assert_eq!(split_query_words(""), Vec::<&str>::new());
    assert_eq!(split_query_words("   "), Vec::<&str>::new());
}

#[test]
fn the_split_is_on_spaces_and_the_trim_comes_after() {
    // splitQueryWords splits on " " only and then trims the rest, so a tab
    // between two words leaves one word with a tab inside it. Reproduced
    // because it is what the index was built against.
    assert_eq!(split_query_words("a\tb"), vec!["a\tb"]);
    assert_eq!(split_query_words(" \ta\t "), vec!["a"]);
}

#[test]
fn a_candidate_query_quotes_every_word() {
    // Quoting is what stops a word being read as index syntax: a query
    // containing OR or * would otherwise change the search rather than be
    // searched for.
    assert_eq!(
        prepare_candidate_search_query("annual report"),
        "\"annual\" \"report\""
    );
    assert_eq!(
        prepare_candidate_search_query("a OR b"),
        "\"a\" \"OR\" \"b\""
    );
}

#[test]
fn an_empty_candidate_query_is_empty() {
    assert_eq!(prepare_candidate_search_query("   "), "");
}

#[test]
fn a_correction_query_drops_terms_shorter_than_three_characters() {
    // A two-letter correction matches almost everything and would rank the
    // plan's real matches below noise.
    let plan = CorrectionPlan {
        choices: vec![
            CorrectionChoice::uncorrected("of"),
            CorrectionChoice::uncorrected("report"),
        ],
    };
    assert_eq!(prepare_correction_search_query(&plan), "\"report\"");
}

#[test]
fn a_three_character_term_is_kept() {
    // The C++ is `word.size() <= 2`, so three is the first length that stays.
    let plan = CorrectionPlan {
        choices: vec![CorrectionChoice::uncorrected("tax")],
    };
    assert_eq!(prepare_correction_search_query(&plan), "\"tax\"");
}

#[test]
fn a_plan_whose_terms_are_all_too_short_asks_nothing() {
    let plan = CorrectionPlan {
        choices: vec![CorrectionChoice::uncorrected("of")],
    };
    assert_eq!(prepare_correction_search_query(&plan), "");
}

#[test]
fn a_common_word_scores_worse_than_a_rare_one_with_the_same_score() {
    // A very common word is a bad correction even when the spelling index
    // likes it: it matches a great many files, none of them the one wanted.
    let rare = VocabularySuggestion {
        word: "quixotic".to_owned(),
        distance: 1,
        score: 100,
        rank: 1,
    };
    let common = VocabularySuggestion {
        rank: 100_000,
        ..rare.clone()
    };
    assert!(adjusted_suggestion_score(&rare) > adjusted_suggestion_score(&common));
}

#[test]
fn the_rank_discount_flattens_out() {
    // Logarithmic: the penalty grows quickly at first and then slowly, so a
    // merely common word is not treated like an impossibly common one.
    let at = |rank: i64| {
        adjusted_suggestion_score(&VocabularySuggestion {
            word: "w".to_owned(),
            distance: 1,
            score: 100,
            rank,
        })
    };
    // Measured over equal absolute increments, which is what "flattens" means
    // for a log: one more occurrence costs a lot at rank 1 and almost nothing
    // at rank 1000.
    let first_step = at(1) - at(2);
    let later_step = at(1000) - at(1001);
    assert!(
        first_step > later_step,
        "{first_step} should exceed {later_step}"
    );
}

#[test]
fn a_correction_never_outranks_an_uncorrected_match() {
    // The best possible correction is still worth less than 1.0.
    assert!(correction_weight(0) < 1.0);
    assert_eq!(correction_weight(0), CORRECTION_PENALTY);
}

#[test]
fn a_closer_correction_is_worth_more() {
    assert!(correction_weight(1) > correction_weight(50));
    assert!(correction_weight(50) > correction_weight(MAX_CORRECTION_DISTANCE));
}

#[test]
fn the_furthest_correction_is_worth_half_the_penalty() {
    let furthest = correction_weight(MAX_CORRECTION_DISTANCE);
    assert!(
        (furthest - CORRECTION_PENALTY * 0.5).abs() < 1e-9,
        "{furthest}"
    );
}

#[test]
fn trailing_digits_are_stemmed_away() {
    // So file2 and file3 are one family: a numbered series is the same word as
    // far as suggesting a correction goes.
    assert_eq!(stem("file2"), "file");
    assert_eq!(stem("file"), "file");
    assert_eq!(stem("2024"), "", "all digits stems to nothing");
}

#[test]
fn words_are_family_by_prefix_or_by_stem() {
    assert!(same_family("report", "rep"), "a prefix");
    assert!(same_family("rep", "report"), "and the other way round");
    assert!(same_family("file2", "file3"), "the same stem");
    assert!(!same_family("report", "receipt"));
}

#[test]
fn a_word_the_corpus_knows_well_is_not_corrected() {
    // Correcting it would replace a search that works with several that do not.
    let suggestions = vec![VocabularySuggestion {
        word: "report".to_owned(),
        distance: 0,
        score: 100,
        rank: TRUSTED_WORD_MIN_RANK,
    }];
    assert!(pick_corrections(&suggestions, "report", 3, true).is_empty());
}

#[test]
fn a_word_the_corpus_barely_knows_is_still_corrected() {
    // Rank below the threshold: appearing once is as likely to be somebody
    // else's typo as a real word.
    let suggestions = vec![
        VocabularySuggestion {
            word: "reprot".to_owned(),
            distance: 0,
            score: 100,
            rank: TRUSTED_WORD_MIN_RANK - 1,
        },
        suggestion("report", 2),
    ];
    let picked = pick_corrections(&suggestions, "reprot", 3, true);
    assert_eq!(picked.len(), 1);
    assert_eq!(picked[0].word, "report");
}

#[test]
fn known_word_trust_can_be_switched_off() {
    // With trust on, a well-known exact match stops correction entirely. With
    // it off, the other suggestions are considered on their own merits — so
    // the second suggestion here is one the prefix rule would not have thrown
    // away anyway.
    let suggestions = vec![
        VocabularySuggestion {
            word: "report".to_owned(),
            distance: 0,
            score: 100,
            rank: 100,
        },
        suggestion("receipt", 1),
    ];
    assert!(pick_corrections(&suggestions, "report", 3, true).is_empty());

    let untrusting = pick_corrections(&suggestions, "report", 3, false);
    assert_eq!(untrusting.len(), 1);
    assert_eq!(untrusting[0].word, "receipt");
}

#[test]
fn a_suggestion_too_far_away_is_never_used() {
    let suggestions = vec![suggestion("something", MAX_CORRECTION_DISTANCE + 1)];
    assert!(pick_corrections(&suggestions, "typo", 3, false).is_empty());
}

#[test]
fn a_suggestion_at_exactly_the_limit_is_used() {
    let suggestions = vec![suggestion("something", MAX_CORRECTION_DISTANCE)];
    assert_eq!(pick_corrections(&suggestions, "typo", 3, false).len(), 1);
}

#[test]
fn the_typed_word_and_its_extensions_are_not_corrections() {
    // The candidate search already covers those, so trying them again is a
    // second query for results the first one has.
    let suggestions = vec![
        suggestion("rep", 0),
        suggestion("report", 1),
        suggestion("receipt", 2),
    ];
    let picked = pick_corrections(&suggestions, "rep", 3, false);
    let words: Vec<_> = picked.iter().map(|s| s.word.clone()).collect();
    assert_eq!(words, vec!["receipt".to_owned()]);
}

#[test]
fn only_one_word_per_family_is_tried() {
    // Searching for both "report" and "reports" asks the index nearly the same
    // question twice and spends a slot that another reading could have used.
    let suggestions = vec![
        suggestion("reports", 1),
        suggestion("report", 2),
        suggestion("receipt", 3),
    ];
    let picked = pick_corrections(&suggestions, "reprot", 3, false);
    let words: Vec<_> = picked.iter().map(|s| s.word.clone()).collect();
    assert_eq!(words.len(), 2);
    assert!(words.contains(&"receipt".to_owned()));
}

#[test]
fn within_a_family_the_shorter_word_wins() {
    // It matches more, and the longer one's matches are a subset.
    let suggestions = vec![suggestion("reports", 1), suggestion("report", 2)];
    let picked = pick_corrections(&suggestions, "reprot", 3, false);
    assert_eq!(picked.len(), 1);
    assert_eq!(picked[0].word, "report");
}

#[test]
fn no_more_than_the_asked_for_number_is_picked() {
    let suggestions = vec![
        suggestion("alpha", 1),
        suggestion("bravo", 2),
        suggestion("charlie", 3),
        suggestion("delta", 4),
    ];
    assert_eq!(pick_corrections(&suggestions, "typo", 2, false).len(), 2);
    assert_eq!(pick_corrections(&suggestions, "typo", 0, false).len(), 0);
}

#[test]
fn no_words_means_no_plans() {
    assert!(build_correction_plans(&[], 5).is_empty());
}

#[test]
fn asking_for_no_plans_gives_none() {
    assert!(build_correction_plans(&[query_word("reprot", &["report"])], 0).is_empty());
}

#[test]
fn a_query_with_nothing_to_correct_produces_no_plans() {
    // The uncorrected query is searched separately, so a plan identical to it
    // would be the same question asked twice.
    let words = vec![query_word("annual", &[]), query_word("report", &[])];
    assert!(build_correction_plans(&words, 5).is_empty());
}

#[test]
fn the_first_plan_takes_every_words_best_correction() {
    let words = vec![
        query_word("anual", &["annual", "annul"]),
        query_word("reprot", &["report"]),
    ];
    let plans = build_correction_plans(&words, 5);

    assert_eq!(plans[0].choices[0].term, "annual");
    assert_eq!(plans[0].choices[1].term, "report");
    assert!(plans[0].choices.iter().all(|choice| choice.corrected));
}

#[test]
fn a_word_with_no_corrections_is_left_alone_in_every_plan() {
    let words = vec![query_word("annual", &[]), query_word("reprot", &["report"])];
    let plans = build_correction_plans(&words, 5);

    assert_eq!(plans[0].choices[0].term, "annual");
    assert!(!plans[0].choices[0].corrected);
    assert_eq!(plans[0].choices[0].weight, 1.0, "uncorrected is worth full");
}

#[test]
fn every_later_plan_differs_from_the_best_one_in_exactly_one_word() {
    // Enumerating every combination would be exponential in the number of
    // words and would mostly produce readings nobody typed: two typos in one
    // query is rare, three is a different query.
    let words = vec![
        query_word("anual", &["annual", "annul"]),
        query_word("reprot", &["report", "reported"]),
    ];
    let plans = build_correction_plans(&words, 10);
    let best = &plans[0];

    for plan in &plans[1..] {
        let differences = plan
            .choices
            .iter()
            .zip(&best.choices)
            .filter(|(a, b)| a.term != b.term)
            .count();
        assert_eq!(
            differences, 1,
            "plan {plan:?} differs in {differences} words"
        );
    }
}

#[test]
fn the_number_of_plans_is_capped() {
    let words = vec![
        query_word("a", &["a1", "a2", "a3", "a4", "a5"]),
        query_word("b", &["b1", "b2", "b3", "b4", "b5"]),
    ];
    assert_eq!(build_correction_plans(&words, 3).len(), 3);
}

#[test]
fn duplicate_plans_are_not_kept() {
    // Two words whose alternatives produce the same set of terms would
    // otherwise spend two slots on one question.
    let words = vec![query_word("x", &["same", "same"])];
    let plans = build_correction_plans(&words, 5);
    assert_eq!(plans.len(), 1);
}

#[test]
fn a_corrected_choice_records_what_was_typed_as_well_as_what_is_searched() {
    // The original is what a "did you mean" line has to show.
    let words = vec![query_word("reprot", &["report"])];
    let plans = build_correction_plans(&words, 5);

    assert_eq!(plans[0].choices[0].original, "reprot");
    assert_eq!(plans[0].choices[0].term, "report");
    assert!(plans[0].choices[0].corrected);
    assert!(plans[0].choices[0].weight < 1.0);
}

#[test]
fn a_plans_key_joins_its_terms_with_a_separator_no_word_can_contain() {
    // splitQueryWords splits on spaces and trims whitespace, so a newline
    // cannot appear inside a term and cannot make two plans collide.
    let plan = CorrectionPlan {
        choices: vec![
            CorrectionChoice::uncorrected("a"),
            CorrectionChoice::uncorrected("b"),
        ],
    };
    assert_eq!(plan.key(), "a\nb");
}
