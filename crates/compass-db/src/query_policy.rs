//! How a file-index query is shaped before it reaches SQLite.
//!
//! Ports `file-indexer-query-policy.cpp`: splitting the query into words,
//! quoting them into FTS match strings, and turning spellfix suggestions into
//! correction plans with distance-weighted choices. Pure over strings and
//! scores — no database handle — so the query engine and its tests can use it
//! without one.

/// A spellfix suggestion for one query word, mirroring the database's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VocabularySuggestion {
    /// The suggested word.
    pub word: String,
    /// The edit distance from the query word.
    pub distance: i32,
    /// The spellfix score.
    pub score: i32,
    /// How often the word occurs: higher is more familiar.
    pub rank: i64,
}

/// One query word with its candidate corrections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryWord {
    /// The word as typed.
    pub word: String,
    /// Its corrections, best first.
    pub corrections: Vec<VocabularySuggestion>,
}

/// One word of a correction plan: the original or its replacement.
#[derive(Debug, Clone, PartialEq)]
pub struct CorrectionChoice {
    /// The word as typed.
    pub original: String,
    /// The word to search for.
    pub term: String,
    /// The distance penalty, 1.0 when uncorrected.
    pub weight: f64,
    /// Whether the term differs from the original.
    pub corrected: bool,
}

/// One corrected reading of the whole query.
#[derive(Debug, Clone, PartialEq)]
pub struct CorrectionPlan {
    /// One choice per query word, in order.
    pub choices: Vec<CorrectionChoice>,
}

/// How much rank discounts a suggestion's score, in bits.
pub const RANK_LOG_WEIGHT: f64 = 10.0;

/// How familiar a word must be to veto correcting the query to something else.
pub const TRUSTED_WORD_MIN_RANK: i64 = 3;

/// The penalty every correction starts from.
pub const CORRECTION_PENALTY: f64 = 0.85;

/// Past this edit distance a suggestion is not a correction.
pub const MAX_CORRECTION_DISTANCE: i32 = 120;

/// Splits a query into words.
///
/// Splits on the space character and trims each piece of surrounding
/// whitespace, dropping empties. A tab between words does *not* split — only
/// spaces do — which survives here because changing it would reshape every
/// multi-word query the engine has ever run.
#[must_use]
pub fn split_query_words(query: &str) -> Vec<String> {
    query
        .split(' ')
        .map(|word| word.trim_matches([' ', '\t', '\n', '\r', '\x0c', '\x0b']))
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Quotes each query word into an FTS phrase match: `"foo" "bar"`.
#[must_use]
pub fn prepare_candidate_search_query(query: &str) -> String {
    split_query_words(query)
        .iter()
        .map(|word| format!("\"{word}\""))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Quotes a correction plan's long-enough terms for the same FTS match.
///
/// Terms of two characters or fewer are skipped: they match too much to
/// correct anything.
#[must_use]
pub fn prepare_correction_search_query(plan: &CorrectionPlan) -> String {
    plan.choices
        .iter()
        .filter(|choice| choice.term.len() > 2)
        .map(|choice| format!("\"{}\"", choice.term))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Discounts a suggestion's score by how unfamiliar its word is.
#[must_use]
pub fn adjusted_suggestion_score(suggestion: &VocabularySuggestion) -> f64 {
    f64::from(suggestion.score) - RANK_LOG_WEIGHT * (1.0 + suggestion.rank as f64).log2()
}

/// The weight of a correction at an edit distance: the full penalty up close,
/// half of it at the farthest distance still considered.
#[must_use]
pub fn correction_weight(distance: i32) -> f64 {
    let closeness = 1.0 - 0.5 * (f64::from(distance) / f64::from(MAX_CORRECTION_DISTANCE));
    CORRECTION_PENALTY * closeness
}

/// The word without its trailing digits: `report2` and `report` are one
/// family, and corrections should not offer both.
fn stem(word: &str) -> &str {
    let bytes = word.as_bytes();
    match bytes.iter().rposition(|byte| !byte.is_ascii_digit()) {
        // All digits: the whole word is the stem. Otherwise cut after the
        // last non-digit, which cannot split a character: everything after it
        // is ASCII digits.
        None => word,
        Some(end) => &word[..=end],
    }
}

/// Whether two suggestions correct toward the same word: one prefixes the
/// other, or their digit-stripped stems agree.
fn same_family(first: &str, second: &str) -> bool {
    first.starts_with(second) || second.starts_with(first) || stem(first) == stem(second)
}

/// Picks which suggestions become corrections for `original`.
///
/// A word the index already knows well vetoes every correction. Otherwise each
/// suggestion past the distance cap, the original itself, and anything the
/// original already prefixes is skipped; relatives collapse to one per family,
/// preferring the shorter word; and the list stops at `max_count`.
#[must_use]
pub fn pick_corrections(
    suggestions: &[VocabularySuggestion],
    original: &str,
    max_count: usize,
    trust_known_words: bool,
) -> Vec<VocabularySuggestion> {
    let lowered = original.to_ascii_lowercase();

    let known_word = trust_known_words
        && suggestions.iter().any(|suggestion| {
            suggestion.distance == 0
                && suggestion.word == lowered
                && suggestion.rank >= TRUSTED_WORD_MIN_RANK
        });
    if known_word {
        return Vec::new();
    }

    let mut picked: Vec<VocabularySuggestion> = Vec::new();
    for suggestion in suggestions {
        if suggestion.distance > MAX_CORRECTION_DISTANCE {
            continue;
        }
        let word = &suggestion.word;
        if *word == lowered || word.starts_with(&lowered) {
            continue;
        }
        if let Some(family) = picked
            .iter_mut()
            .find(|existing| same_family(&existing.word, word))
        {
            if family.word.starts_with(word) {
                *family = suggestion.clone();
            }
            continue;
        }
        if picked.len() < max_count {
            picked.push(suggestion.clone());
        }
    }

    picked
}

/// Builds the correction plans worth searching: the best correction of every
/// word, then one plan per further correction of each single word.
///
/// Plans that correct nothing are dropped, duplicate readings collapse, and
/// the list stops at `max_plans`.
#[must_use]
pub fn build_correction_plans(words: &[QueryWord], max_plans: usize) -> Vec<CorrectionPlan> {
    fn make_choice(word: &QueryWord, correction_index: usize) -> CorrectionChoice {
        let Some(correction) = word.corrections.get(correction_index) else {
            return CorrectionChoice {
                original: word.word.clone(),
                term: word.word.clone(),
                weight: 1.0,
                corrected: false,
            };
        };
        CorrectionChoice {
            original: word.word.clone(),
            term: correction.word.clone(),
            weight: correction_weight(correction.distance),
            corrected: true,
        }
    }

    fn plan_key(plan: &CorrectionPlan) -> String {
        plan.choices
            .iter()
            .map(|choice| choice.term.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    if words.is_empty() || max_plans == 0 {
        return Vec::new();
    }

    // Takes the list explicitly: a closure capturing it would freeze it for
    // the whole loop, and the loop reads its length to stop early.
    let add_plan = |plans: &mut Vec<CorrectionPlan>, plan: CorrectionPlan| {
        if !plan.choices.iter().any(|choice| choice.corrected) {
            return;
        }
        let key = plan_key(&plan);
        let duplicate = plans.iter().any(|existing| plan_key(existing) == key);
        if !duplicate && plans.len() < max_plans {
            plans.push(plan);
        }
    };

    let mut plans: Vec<CorrectionPlan> = Vec::new();
    let best: CorrectionPlan = CorrectionPlan {
        choices: words.iter().map(|word| make_choice(word, 0)).collect(),
    };
    add_plan(&mut plans, best.clone());

    for (word_index, word) in words.iter().enumerate() {
        for correction_index in 1..word.corrections.len() {
            if plans.len() >= max_plans {
                return plans;
            }
            let mut plan = best.clone();
            plan.choices[word_index] = make_choice(word, correction_index);
            add_plan(&mut plans, plan);
        }
    }

    plans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suggestion(word: &str, distance: i32, score: i32, rank: i64) -> VocabularySuggestion {
        VocabularySuggestion {
            word: word.to_owned(),
            distance,
            score,
            rank,
        }
    }

    #[test]
    fn words_split_on_spaces_and_trim_whitespace() {
        assert_eq!(split_query_words("  foo\tbar  baz\n"), ["foo\tbar", "baz"]);
        assert_eq!(split_query_words(""), Vec::<String>::new());
        assert_eq!(split_query_words("   "), Vec::<String>::new());
    }

    #[test]
    fn only_spaces_split_words() {
        // A tab between words does not split — only spaces do. Surprising,
        // and the shipped behaviour, so multi-word queries keep working.
        assert_eq!(split_query_words("a\tb"), ["a\tb"]);
    }

    #[test]
    fn the_candidate_query_quotes_every_word() {
        assert_eq!(prepare_candidate_search_query("foo bar"), "\"foo\" \"bar\"");
        assert_eq!(prepare_candidate_search_query(""), "");
    }

    #[test]
    fn the_correction_query_skips_short_terms() {
        let plan = CorrectionPlan {
            choices: vec![
                CorrectionChoice {
                    original: "a".to_owned(),
                    term: "ab".to_owned(),
                    weight: 0.8,
                    corrected: true,
                },
                CorrectionChoice {
                    original: "doc".to_owned(),
                    term: "document".to_owned(),
                    weight: 0.8,
                    corrected: true,
                },
                CorrectionChoice {
                    original: "x".to_owned(),
                    term: "x".to_owned(),
                    weight: 1.0,
                    corrected: false,
                },
            ],
        };
        assert_eq!(prepare_correction_search_query(&plan), "\"document\"");
    }

    #[test]
    fn rank_discounts_the_score_in_bits() {
        // 100 - 10 * log2(1 + 3) = 80.
        let score = adjusted_suggestion_score(&suggestion("word", 0, 100, 3));
        assert!((score - 80.0).abs() < 1e-9, "got {score}");
        assert_eq!(RANK_LOG_WEIGHT, 10.0);
    }

    #[test]
    fn correction_weight_falls_with_distance() {
        assert!((correction_weight(0) - CORRECTION_PENALTY).abs() < 1e-12);
        assert!(
            (correction_weight(MAX_CORRECTION_DISTANCE) - CORRECTION_PENALTY / 2.0).abs() < 1e-12
        );
        assert_eq!(CORRECTION_PENALTY, 0.85);
        assert_eq!(MAX_CORRECTION_DISTANCE, 120);
    }

    #[test]
    fn a_known_word_vetoes_every_correction() {
        let suggestions = vec![suggestion("report", 0, 90, TRUSTED_WORD_MIN_RANK)];
        assert!(pick_corrections(&suggestions, "Report", 5, true).is_empty());
        // Distrusted, the same suggestion corrects a nearby typo normally.
        assert_eq!(
            pick_corrections(&suggestions, "reprot", 5, false),
            suggestions
        );
    }

    #[test]
    fn barely_known_words_do_not_veto() {
        // Rank 2 is one short of trusted: the exact match is skipped as the
        // original itself, but the other suggestion still corrects — while at
        // rank 3 the veto wipes the whole list.
        let weak = vec![
            suggestion("report", 0, 90, TRUSTED_WORD_MIN_RANK - 1),
            suggestion("export", 5, 50, 1),
        ];
        assert_eq!(
            pick_corrections(&weak, "report", 5, true),
            [suggestion("export", 5, 50, 1)]
        );
        let trusted = vec![
            suggestion("report", 0, 90, TRUSTED_WORD_MIN_RANK),
            suggestion("export", 5, 50, 1),
        ];
        assert!(pick_corrections(&trusted, "report", 5, true).is_empty());
    }

    #[test]
    fn the_original_and_its_prefixes_are_not_corrections() {
        let suggestions = vec![
            suggestion("report", 5, 50, 1),
            suggestion("reporter", 6, 50, 1),
            suggestion("export", 6, 50, 1),
        ];
        assert_eq!(
            pick_corrections(&suggestions, "report", 5, false),
            [suggestion("export", 6, 50, 1)]
        );
    }

    #[test]
    fn suggestions_past_the_distance_cap_are_not_corrections() {
        let suggestions = vec![suggestion("far", MAX_CORRECTION_DISTANCE + 1, 50, 1)];
        assert!(pick_corrections(&suggestions, "near", 5, false).is_empty());
    }

    #[test]
    fn one_family_yields_one_correction_preferring_the_shorter_word() {
        // report2 stems to report: relatives, and the longer word already
        // picked is replaced by the shorter arrival.
        let suggestions = vec![
            suggestion("reporter", 6, 50, 1),
            suggestion("report", 5, 60, 2),
        ];
        assert_eq!(
            pick_corrections(&suggestions, "xyz", 5, false),
            [suggestion("report", 5, 60, 2)]
        );
    }

    #[test]
    fn digit_stems_share_a_family() {
        assert_eq!(stem("report2"), "report");
        assert_eq!(stem("report"), "report");
        assert_eq!(stem("123"), "123");
        let suggestions = vec![
            suggestion("report2", 6, 50, 1),
            suggestion("report", 5, 60, 2),
        ];
        assert_eq!(
            pick_corrections(&suggestions, "xyz", 5, false),
            [suggestion("report", 5, 60, 2)]
        );
    }

    #[test]
    fn the_pick_stops_at_the_count() {
        let suggestions = vec![
            suggestion("alpha", 6, 50, 1),
            suggestion("beta", 6, 50, 1),
            suggestion("gamma", 6, 50, 1),
        ];
        assert_eq!(
            pick_corrections(&suggestions, "xyz", 2, false),
            [suggestion("alpha", 6, 50, 1), suggestion("beta", 6, 50, 1)]
        );
    }

    #[test]
    fn plans_correct_the_best_of_every_word_then_each_further_one() {
        let words = vec![
            QueryWord {
                word: "reprot".to_owned(),
                corrections: vec![suggestion("report", 2, 80, 5)],
            },
            QueryWord {
                word: "fle".to_owned(),
                corrections: vec![suggestion("file", 1, 90, 9), suggestion("flea", 2, 40, 2)],
            },
        ];
        let plans = build_correction_plans(&words, 10);
        let terms = |plan: &CorrectionPlan| {
            plan.choices
                .iter()
                .map(|choice| choice.term.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            plans.iter().map(terms).collect::<Vec<_>>(),
            [["report", "file"], ["report", "flea"]]
        );
        assert!(
            plans
                .iter()
                .all(|plan| plan.choices.iter().all(|choice| choice.corrected))
        );
        assert!((plans[0].choices[0].weight - correction_weight(2)).abs() < 1e-12);
    }

    #[test]
    fn plans_that_correct_nothing_are_dropped() {
        let words = vec![QueryWord {
            word: "file".to_owned(),
            corrections: Vec::new(),
        }];
        assert!(build_correction_plans(&words, 10).is_empty());
    }

    #[test]
    fn duplicate_readings_collapse() {
        let word = QueryWord {
            word: "fle".to_owned(),
            corrections: vec![suggestion("file", 1, 90, 9), suggestion("file", 1, 80, 8)],
        };
        let plans = build_correction_plans(&[word], 10);
        assert_eq!(plans.len(), 1);
    }

    #[test]
    fn plans_stop_at_the_cap() {
        let words = vec![QueryWord {
            word: "fle".to_owned(),
            corrections: vec![
                suggestion("file", 1, 90, 9),
                suggestion("flea", 2, 40, 2),
                suggestion("fled", 2, 30, 1),
            ],
        }];
        assert_eq!(build_correction_plans(&words, 1).len(), 1);
    }

    #[test]
    fn no_words_or_no_plans_means_no_plans() {
        assert!(build_correction_plans(&[], 10).is_empty());
        let words = vec![QueryWord {
            word: "fle".to_owned(),
            corrections: vec![suggestion("file", 1, 90, 9)],
        }];
        assert!(build_correction_plans(&words, 0).is_empty());
    }
}
