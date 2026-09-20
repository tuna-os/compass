use compass_search::{Matcher, Query, WeightedField, score_weighted};
use proptest::prelude::*;

#[test]
fn a_unicode_separator_does_not_hide_a_later_word_boundary() {
    let mut matcher = Matcher::new();
    let ascii = matcher
        .match_("Bear Factory - Animation Editor", "e")
        .unwrap();
    for needle in ["e", "é", "E"] {
        let unicode = matcher
            .match_("Bear Factory — Animation Editor", needle)
            .unwrap();
        assert_eq!(unicode.score, ascii.score);
        assert_eq!(unicode.indices, ascii.indices);
        assert_eq!(unicode.indices, [25]);
        assert_eq!(
            matcher.score("Bear Factory — Animation Editor", needle),
            Some(ascii.score)
        );
        assert_eq!(
            score_weighted(
                &[WeightedField::new("Bear Factory — Animation Editor", 1.0)],
                &Query::new(needle),
            )
            .score,
            100,
        );
    }
}

#[test]
fn single_unicode_matches_keep_char_indices_and_earliest_ties() {
    let mut matcher = Matcher::new();
    for (text, query, expected) in [
        ("école Éditeur", "e", 0),
        ("mélange Éditeur", "é", 8),
        ("— редактор Енот", "е", 11),
        ("— 猫 猫", "猫", 2),
        ("— one one", "o", 2),
    ] {
        let found = matcher.match_(text, query).unwrap();
        assert_eq!(found.indices, [expected], "{text:?} / {query:?}");
        assert_eq!(matcher.score(text, query), Some(found.score));
        assert!(found.coherent);
    }
    assert!(matcher.match_("— aucun", "z").is_none());
    assert!(matcher.score("— aucun", "z").is_none());
}

#[test]
fn harvested_editor_titles_match_their_later_word_starts() {
    for (title, query) in [
        ("Bear Factory — Animation Editor", "A"),
        ("Bear Factory — Animation Editor", "a"),
        ("Bear Factory — Animation Editor", "E"),
        ("Bear Factory — Level Editor", "E"),
        ("Bear Factory — Model Editor", "E"),
        ("Bear Factory — Spritedesc interpreter", "I"),
    ] {
        let result = score_weighted(&[WeightedField::new(title, 1.0)], &Query::new(query));
        assert_eq!(result.score, 100, "{title:?} / {query:?}");
        assert_eq!(result.quality, 100);
    }
}

proptest! {
    #[test]
    fn appending_unicode_does_not_change_single_ascii_matches(
        text in "[a-zA-Z0-9 /,:;|_-]{0,80}",
        query in "[a-z0-9]",
    ) {
        let mut matcher = Matcher::new();
        let expected = matcher.match_(&text, &query);
        let unicode = format!("{text}—");
        prop_assert_eq!(matcher.match_(&unicode, &query), expected);
        prop_assert_eq!(matcher.score(&unicode, &query), matcher.score(&text, &query));
    }
}
