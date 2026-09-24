//! Turning what somebody typed into what the file index is asked.
//!
//! A port of `file_indexer::query`
//! (`src/file-indexer/src/file-indexer-query-policy.cpp`).
//!
//! # Spelling correction is the part that can quietly get worse
//!
//! Splitting and quoting a query is mechanical. Deciding *which* misspellings
//! to try is not: too few and a typo finds nothing, too many and the index is
//! asked several questions per keystroke, each of which ranks worse than the
//! one the person meant. None of that shows up as a failure — it shows up as
//! search feeling slightly unreliable. So every rule below has a test naming
//! the thing it prevents.

/// How heavily a suggestion's corpus rank counts against its score.
pub const RANK_LOG_WEIGHT: f64 = 10.0;

/// How often a word must appear before a zero-distance match is trusted as
/// correctly spelled.
///
/// Three, not one: a word appearing once in the corpus is as likely to be
/// somebody else's typo as a real word.
pub const TRUSTED_WORD_MIN_RANK: i64 = 3;

/// What a corrected term's score is multiplied by at best.
pub const CORRECTION_PENALTY: f64 = 0.85;

/// The largest edit distance a suggestion may have and still be used.
pub const MAX_CORRECTION_DISTANCE: i32 = 120;

/// The shortest a corrected term may be before it is dropped from a query.
///
/// A two-letter correction matches almost anything and drags the whole plan's
/// results down with it.
pub const MIN_CORRECTION_TERM_LENGTH: usize = 3;

/// One suggestion from the spelling index.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VocabularySuggestion {
    /// The suggested word, always lowercase.
    pub word: String,
    /// Its edit distance from what was typed.
    pub distance: i32,
    /// The index's own score for it.
    pub score: i32,
    /// How often it appears in the corpus.
    pub rank: i64,
}

/// One word of the query, with the corrections offered for it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueryWord {
    /// What was typed.
    pub word: String,
    /// What might have been meant, best first.
    pub corrections: Vec<VocabularySuggestion>,
}

/// What one word of the query becomes in one plan.
#[derive(Debug, Clone, PartialEq)]
pub struct CorrectionChoice {
    /// What was typed.
    pub original: String,
    /// What is being searched for instead.
    pub term: String,
    /// How much this choice's results are worth, 1.0 for an uncorrected word.
    pub weight: f64,
    /// Whether this is a correction rather than the original.
    pub corrected: bool,
}

impl CorrectionChoice {
    /// The choice that leaves `word` alone.
    #[must_use]
    pub fn uncorrected(word: &str) -> Self {
        Self {
            original: word.to_owned(),
            term: word.to_owned(),
            weight: 1.0,
            corrected: false,
        }
    }
}

/// One whole alternative reading of the query.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CorrectionPlan {
    /// One choice per query word, in order.
    pub choices: Vec<CorrectionChoice>,
}

impl CorrectionPlan {
    /// Whether any word in this plan was corrected.
    #[must_use]
    pub fn has_correction(&self) -> bool {
        self.choices.iter().any(|choice| choice.corrected)
    }

    /// The plan's identity, for spotting duplicates.
    ///
    /// The terms joined by a newline — a separator a query word cannot
    /// contain, since [`split_query_words`] splits on spaces and trims the
    /// rest.
    #[must_use]
    pub fn key(&self) -> String {
        self.choices
            .iter()
            .map(|choice| choice.term.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The whitespace `splitQueryWords` trims.
const WHITESPACE: &[char] = &[' ', '\t', '\n', '\r', '\u{c}', '\u{b}'];

/// Split a query into words, dropping empties.
///
/// Splits on the space character only and *then* trims the other whitespace,
/// which is why a tab between two words leaves them as one word with a tab in
/// the middle. Reproduced: it is what the index was built against.
#[must_use]
pub fn split_query_words(query: &str) -> Vec<&str> {
    query
        .split(' ')
        .map(|word| word.trim_matches(WHITESPACE))
        .filter(|word| !word.is_empty())
        .collect()
}

/// The query to look for candidates with: every word, quoted.
///
/// Quoting is what stops a word being read as index syntax — a query
/// containing `OR` or `*` would otherwise change the search rather than be
/// searched for.
#[must_use]
pub fn prepare_candidate_search_query(query: &str) -> String {
    split_query_words(query)
        .into_iter()
        .map(|word| format!("\"{word}\""))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The query for one correction plan.
///
/// Terms shorter than [`MIN_CORRECTION_TERM_LENGTH`] are dropped rather than
/// searched: a two-letter correction matches almost everything, and including
/// it would rank the plan's real matches below noise.
#[must_use]
pub fn prepare_correction_search_query(plan: &CorrectionPlan) -> String {
    plan.choices
        .iter()
        .filter(|choice| choice.term.len() > MIN_CORRECTION_TERM_LENGTH - 1)
        .map(|choice| format!("\"{}\"", choice.term))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A suggestion's score, discounted by how common the word is.
///
/// A very common word is a bad correction even when the spelling index likes
/// it: it will match a great many files, none of them the one wanted. The
/// discount is logarithmic so the penalty grows quickly at first and then
/// flattens.
#[must_use]
pub fn adjusted_suggestion_score(suggestion: &VocabularySuggestion) -> f64 {
    f64::from(suggestion.score) - RANK_LOG_WEIGHT * (1.0 + suggestion.rank as f64).log2()
}

/// What a correction at `distance` is worth, relative to an exact match.
///
/// Between [`CORRECTION_PENALTY`] at distance zero and half of it at the
/// maximum, so a correction never outranks an uncorrected match.
#[must_use]
pub fn correction_weight(distance: i32) -> f64 {
    let closeness = 0.5f64.mul_add(
        -(f64::from(distance) / f64::from(MAX_CORRECTION_DISTANCE)),
        1.0,
    );
    CORRECTION_PENALTY * closeness
}

/// The word with trailing digits removed.
///
/// So `file2` and `file3` are one family — a numbered series is the same word
/// as far as suggesting a correction goes.
#[must_use]
pub fn stem(word: &str) -> &str {
    word.trim_end_matches(|character: char| character.is_ascii_digit())
}

/// Whether two suggestions are close enough that only one is worth trying.
#[must_use]
pub fn same_family(left: &str, right: &str) -> bool {
    left.starts_with(right) || right.starts_with(left) || stem(left) == stem(right)
}

/// Choose which suggestions are worth searching for.
///
/// `trust_known_words` skips correction entirely when the typed word is itself
/// a word the corpus knows well — see [`TRUSTED_WORD_MIN_RANK`].
#[must_use]
pub fn pick_corrections(
    suggestions: &[VocabularySuggestion],
    original: &str,
    max_count: usize,
    trust_known_words: bool,
) -> Vec<VocabularySuggestion> {
    let lowered = original.to_lowercase();

    // A word the corpus knows is not a typo, so correcting it would replace a
    // search that works with several that do not.
    let known_word = trust_known_words
        && suggestions.iter().any(|suggestion| {
            suggestion.distance == 0
                && suggestion.word == lowered
                && suggestion.rank >= TRUSTED_WORD_MIN_RANK
        });
    if known_word {
        return Vec::new();
    }

    let mut picked: Vec<VocabularySuggestion> = Vec::with_capacity(max_count);

    for suggestion in suggestions {
        if suggestion.distance > MAX_CORRECTION_DISTANCE {
            continue;
        }

        // The word itself, or something it is a prefix of, is not a
        // correction: the candidate search already covers those.
        if suggestion.word == lowered || suggestion.word.starts_with(&lowered) {
            continue;
        }

        if let Some(existing) = picked
            .iter_mut()
            .find(|existing| same_family(&existing.word, &suggestion.word))
        {
            // Within a family keep the shorter word, which matches more.
            if existing.word.starts_with(&suggestion.word) {
                *existing = suggestion.clone();
            }
            continue;
        }

        if picked.len() < max_count {
            picked.push(suggestion.clone());
        }
    }

    picked
}

/// Build the alternative readings of a query worth searching for.
///
/// # One correction at a time
///
/// The first plan takes each word's best correction. Every plan after it
/// differs from that one in exactly *one* word. Enumerating every combination
/// would be exponential in the number of words and would mostly produce
/// readings nobody typed — two typos in one query is rare, three is a
/// different query.
///
/// Plans with no correction at all are dropped, because the uncorrected query
/// is already being searched separately, and duplicates are dropped by
/// [`CorrectionPlan::key`].
#[must_use]
pub fn build_correction_plans(words: &[QueryWord], max_plans: usize) -> Vec<CorrectionPlan> {
    let mut plans: Vec<CorrectionPlan> = Vec::new();

    if words.is_empty() || max_plans == 0 {
        return plans;
    }

    let make_choice = |word: &QueryWord, correction_index: usize| -> CorrectionChoice {
        match word.corrections.get(correction_index) {
            None => CorrectionChoice::uncorrected(&word.word),
            Some(correction) => CorrectionChoice {
                original: word.word.clone(),
                term: correction.word.clone(),
                weight: correction_weight(correction.distance),
                corrected: true,
            },
        }
    };

    let add_plan = |plans: &mut Vec<CorrectionPlan>, plan: CorrectionPlan| {
        if !plan.has_correction() {
            return;
        }
        let key = plan.key();
        if plans.iter().any(|existing| existing.key() == key) {
            return;
        }
        // No cap here: the loop below stops once `max_plans` is reached, and
        // this is the only other caller. Two guards for one rule means the
        // rule can be broken in one of them without a test noticing.
        plans.push(plan);
    };

    let best_plan = CorrectionPlan {
        choices: words.iter().map(|word| make_choice(word, 0)).collect(),
    };
    add_plan(&mut plans, best_plan.clone());

    for (word_index, word) in words.iter().enumerate() {
        for correction_index in 1..word.corrections.len() {
            if plans.len() >= max_plans {
                return plans;
            }
            let mut plan = best_plan.clone();
            plan.choices[word_index] = make_choice(word, correction_index);
            add_plan(&mut plans, plan);
        }
    }

    plans
}
