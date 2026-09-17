//! Turning a clipboard search box into the two things SQLite can be asked.
//!
//! The C++ engine's `ClipboardDatabase::query` does not send the user's text to
//! one matcher. It splits the query into words and sorts each word into one of
//! two buckets, because the FTS5 index cannot answer for both:
//!
//! * words the **trigram tokenizer can index** become quoted FTS match phrases,
//!   combined with `AND`;
//! * words it **cannot** become `instr(lower(content), lower(?)) > 0`
//!   conditions against the same column, also `AND`ed.
//!
//! The split is not a heuristic. `selection_fts` is declared
//! `tokenize='fuzzy_trigram remove_diacritics 2'` (migration `002_trigram_fts.sql`),
//! and a trigram tokenizer emits no tokens at all for a word with fewer than
//! three consecutive indexable characters. `clipboard-db.cpp` says what happens
//! then, right above its own copy of this predicate:
//!
//! > `fuzzy_trigram` strips separators: a term without a 3+ run of word chars
//! > (`"->>"`, `"a!b"`) yields no trigrams, so MATCH would find nothing and the
//! > instr scan must be used instead
//!
//! Find *nothing*, not everything — the empty token list makes the term
//! unsatisfiable rather than unconstraining. Checked against SQLite 3.45.1's
//! stock `trigram` tokenizer, which shares the three-character floor: with
//! `"ab"` present in the corpus, `MATCH '"ab"'` returns zero rows. So a
//! two-letter search routed to FTS does not return too much, it returns an
//! empty history, and the `instr` fallback is the only thing that makes such a
//! search work at all.
//!
//! # Why this counts UTF-16 code units
//!
//! The C++ predicate is, per character of a `QString`:
//!
//! ```cpp
//! bool const wordChar = c.unicode() >= 0x80 || c.isLetterOrNumber();
//! ```
//!
//! A `QString` iterates **UTF-16 code units**, not Unicode scalar values, so a
//! character outside the Basic Multilingual Plane is two iterations, and both
//! of its surrogates are `>= 0x80` and therefore both count as word characters.
//! `"a😀"` is a run of three to the C++ engine and would be a run of two to a
//! port that iterated Rust `char`s. That is a real difference in what the user
//! gets back, so [`has_trigram_run`] iterates `encode_utf16` and the test
//! module holds the scalar-iterating version as a control, to show the
//! surrogate case is what distinguishes them rather than being decoration.
//!
//! The `isLetterOrNumber()` half needs no Unicode tables here: it is only ever
//! reached for a code unit below `0x80`, where it is exactly ASCII
//! alphanumeric. Everything at or above `0x80` was already decided by the left
//! operand.

/// The three-character minimum the `fuzzy_trigram` tokenizer needs before it
/// emits anything at all.
const TRIGRAM: usize = 3;

/// How the C++ engine splits a raw query into words.
///
/// `QString::simplified()` collapses every run of whitespace to one space and
/// trims the ends; the subsequent `split(' ', SkipEmptyParts)` then yields the
/// words. Splitting on Rust's whitespace is the same thing in one step: both
/// definitions are Unicode `White_Space` (`Zs ∪ Zl ∪ Zp ∪ {U+0009..U+000D,
/// U+0085}`), which is also how `QChar::isSpace` is specified.
pub fn search_terms(query: &str) -> Vec<&str> {
    query.split_whitespace().collect()
}

/// Whether `word` contains three consecutive characters the trigram tokenizer
/// will index — i.e. whether asking `MATCH` about it constrains anything.
///
/// See the module docs for why this walks UTF-16 code units.
#[must_use]
pub fn has_trigram_run(word: &str) -> bool {
    let mut run = 0usize;

    for unit in word.encode_utf16() {
        let word_char = unit >= 0x80 || (unit as u8).is_ascii_alphanumeric();
        run = if word_char { run + 1 } else { 0 };
        if run >= TRIGRAM {
            return true;
        }
    }

    false
}

/// A query split into the terms each matcher can actually answer for.
///
/// Both lists are `AND`ed together by the caller: a row must satisfy every
/// phrase *and* every substring. Empty is meaningful — [`Plan::is_empty`] being
/// true is what tells `query` to take the unfiltered, paginated path rather
/// than joining against `selection_fts` at all.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan<'a> {
    /// Words with a trigram run, already quoted for FTS5 and ready to be
    /// joined with `" AND "` into one `MATCH` argument.
    pub match_phrases: Vec<String>,
    /// Words without one, to be bound one per `instr(...)` condition.
    pub short_terms: Vec<&'a str>,
}

impl Plan<'_> {
    /// Whether this query constrains nothing, so no `selection_fts` join is needed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.match_phrases.is_empty() && self.short_terms.is_empty()
    }
}

/// Sort every word of `query` into the matcher that can answer for it.
#[must_use]
pub fn plan(query: &str) -> Plan<'_> {
    let mut plan = Plan::default();

    for word in search_terms(query) {
        if has_trigram_run(word) {
            plan.match_phrases.push(quote_phrase(word));
        } else {
            plan.short_terms.push(word);
        }
    }

    plan
}

/// Wrap `word` as an FTS5 string literal.
///
/// FTS5 escapes a double quote inside a quoted string by doubling it, so this
/// is `'"' + word.replace('"', "\"\"") + '"'` — the same expression the C++
/// engine builds. Without it a query containing a quote is not a search that
/// finds nothing, it is a syntax error from SQLite that loses the whole result
/// set.
fn quote_phrase(word: &str) -> String {
    let mut out = String::with_capacity(word.len() + 2);
    out.push('"');
    for c in word.chars() {
        if c == '"' {
            out.push('"');
        }
        out.push(c);
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The port that iterates Rust `char`s instead of UTF-16 code units.
    ///
    /// This is here to be *wrong* in exactly one way, so that the assertions
    /// below are shown to be sensitive to the thing the module docs claim they
    /// are sensitive to. A test that only ever exercises the real function
    /// cannot tell you that.
    fn has_trigram_run_over_scalars(word: &str) -> bool {
        let mut run = 0usize;
        for c in word.chars() {
            let word_char = (c as u32) >= 0x80 || c.is_ascii_alphanumeric();
            run = if word_char { run + 1 } else { 0 };
            if run >= TRIGRAM {
                return true;
            }
        }
        false
    }

    #[test]
    fn a_non_bmp_character_is_two_word_characters_not_one() {
        // The C++ engine sees 'a' and two surrogates: a run of three.
        assert!(
            has_trigram_run("a😀"),
            "a surrogate pair must count as two word characters, as QString iteration does"
        );

        // Control: the scalar-iterating port disagrees here, which is what
        // makes the assertion above evidence rather than decoration.
        assert!(
            !has_trigram_run_over_scalars("a😀"),
            "if the control agrees, this case no longer distinguishes the two ports \
             and the assertion above has stopped testing anything"
        );
    }

    #[test]
    fn the_two_ports_agree_everywhere_the_bmp_reaches() {
        // Scoping the control: outside the surrogate case the naive port is
        // fine, so the divergence above is specific rather than a general
        // disagreement that happens to show up in one example.
        for word in [
            "",
            "a",
            "ab",
            "abc",
            "a-b",
            "a-bc",
            "ab-c",
            "a_b_c",
            "héllo",
            "日本語",
            "日本",
            "..",
            "a1b",
            "1",
            "12",
            "123",
            "--a--",
            "a--b--c",
        ] {
            assert_eq!(
                has_trigram_run(word),
                has_trigram_run_over_scalars(word),
                "the ports disagree on {word:?}, which is outside the surrogate case"
            );
        }
    }

    #[test]
    fn three_indexable_characters_must_be_consecutive() {
        assert!(has_trigram_run("abc"));
        assert!(!has_trigram_run("ab"));

        // Three word characters, but never three in a row: the tokenizer emits
        // nothing, so this has to fall to `instr`.
        assert!(!has_trigram_run("a-b-c"));
        assert!(!has_trigram_run("a.b.c"));

        // The run resets rather than accumulating across the separator.
        assert!(!has_trigram_run("ab-ab"));
        assert!(has_trigram_run("ab-abc"));
    }

    #[test]
    fn anything_above_ascii_is_indexable_without_asking_what_it_is() {
        // Two CJK characters are two word characters — not three.
        assert!(!has_trigram_run("日本"));
        assert!(has_trigram_run("日本語"));

        // Punctuation above 0x80 counts too, because the C++ predicate never
        // consults isLetterOrNumber() once unicode() >= 0x80.
        assert!(
            has_trigram_run("「」〜"),
            "the C++ predicate short-circuits on >= 0x80, so non-ASCII punctuation is a word char"
        );
    }

    #[test]
    fn ascii_punctuation_breaks_a_run() {
        assert!(!has_trigram_run("!@#"));
        assert!(!has_trigram_run("   "));
    }

    #[test]
    fn splitting_matches_simplified_then_split() {
        assert_eq!(search_terms("  hello   world  "), vec!["hello", "world"]);
        assert_eq!(search_terms(""), Vec::<&str>::new());
        assert_eq!(search_terms("   "), Vec::<&str>::new());
        assert_eq!(
            search_terms("tab\tand\nnewline"),
            vec!["tab", "and", "newline"]
        );
        // A no-break space is Zs, so QChar::isSpace and White_Space both split here.
        assert_eq!(search_terms("a\u{a0}b"), vec!["a", "b"]);
    }

    #[test]
    fn a_word_goes_to_exactly_one_matcher() {
        let plan = plan("ab abc");
        assert_eq!(plan.match_phrases, vec!["\"abc\""]);
        assert_eq!(plan.short_terms, vec!["ab"]);

        // Every word is accounted for exactly once: dropping one silently
        // widens the result set, which is the failure mode that looks like
        // success.
        assert_eq!(
            plan.match_phrases.len() + plan.short_terms.len(),
            search_terms("ab abc").len()
        );
    }

    #[test]
    fn an_empty_query_constrains_nothing() {
        assert!(plan("").is_empty());
        assert!(plan("   ").is_empty());
        assert!(!plan("ab").is_empty(), "a short term still constrains");
        assert!(!plan("abc").is_empty());
    }

    #[test]
    fn a_quote_is_doubled_rather_than_ending_the_phrase() {
        assert_eq!(quote_phrase("abc"), "\"abc\"");
        assert_eq!(quote_phrase("a\"bc"), "\"a\"\"bc\"");
        assert_eq!(quote_phrase("\"\""), "\"\"\"\"\"\"");

        // The property that matters: quotes inside the phrase are balanced, so
        // FTS5 sees one string literal and not a truncated one followed by
        // garbage.
        let quoted = quote_phrase("we\"ird");
        assert!(quoted.starts_with('"') && quoted.ends_with('"'));
        assert_eq!(
            quoted.matches('"').count() % 2,
            0,
            "an odd number of quotes is a SQLite syntax error, not a narrower search"
        );
    }

    #[test]
    fn a_quoted_word_still_reaches_fts_when_it_has_a_run() {
        // '"' is ASCII punctuation, so it breaks a run like any other.
        let unquotable = plan("a\"bc");
        assert!(
            unquotable.match_phrases.is_empty(),
            "a, then b c: no run of three"
        );
        assert_eq!(unquotable.short_terms, vec!["a\"bc"]);

        let quotable = plan("ab\"cde");
        assert_eq!(quotable.match_phrases, vec!["\"ab\"\"cde\""]);
        assert!(quotable.short_terms.is_empty());
    }
}
