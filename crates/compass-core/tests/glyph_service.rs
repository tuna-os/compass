//! What a person's emoji history does to the picker.
//!
//! Read off `GlyphService` (`src/server/src/services/glyph-service/`).

use compass_core::glyph_service::{
    BUILTIN_KEYWORD_WEIGHT, CATEGORY_WEIGHT, GlyphService, NAME_WEIGHT, SerializedEmojiMetadata,
    USER_KEYWORD_WEIGHT, search_fields,
};

/// Everything is a known glyph.
fn anything_known(_character: &str) -> bool {
    true
}

/// Only these characters are known glyphs.
fn only(known: &'static [&'static str]) -> impl Fn(&str) -> bool {
    move |character| known.contains(&character)
}

#[test]
fn the_field_weights_are_the_cpp_ones() {
    assert_eq!(USER_KEYWORD_WEIGHT, 2.0);
    assert_eq!(NAME_WEIGHT, 1.0);
    assert_eq!(BUILTIN_KEYWORD_WEIGHT, 0.7);
    assert_eq!(CATEGORY_WEIGHT, 0.5);
}

#[test]
fn the_weights_rank_a_persons_own_keyword_above_everything() {
    // Someone who typed a keyword onto an emoji was saying what they will look
    // for it by; the category matches a great many glyphs at once.
    const { assert!(USER_KEYWORD_WEIGHT > NAME_WEIGHT) };
    const { assert!(NAME_WEIGHT > BUILTIN_KEYWORD_WEIGHT) };
    const { assert!(BUILTIN_KEYWORD_WEIGHT > CATEGORY_WEIGHT) };
}

#[test]
fn every_builtin_keyword_becomes_its_own_field() {
    let fields = search_fields("mine", "grinning face", "Smileys", &["grin", "smile"]);
    assert_eq!(fields.len(), 5);
    assert_eq!(fields[0].text, "mine");
    assert_eq!(fields[0].weight, USER_KEYWORD_WEIGHT);
    assert_eq!(fields[1].text, "grinning face");
    assert_eq!(fields[2].text, "Smileys");
    assert_eq!(fields[2].weight, CATEGORY_WEIGHT);
    assert!(
        fields[3..]
            .iter()
            .all(|field| field.weight == BUILTIN_KEYWORD_WEIGHT)
    );
}

#[test]
fn a_glyph_with_no_user_keyword_still_has_the_field() {
    // The C++ passes an empty string view rather than dropping the field, so
    // the field count does not change with the data.
    let fields = search_fields("", "grinning face", "Smileys", &[]);
    assert_eq!(fields.len(), 3);
    assert_eq!(fields[0].text, "");
}

#[test]
fn a_fresh_store_knows_nothing() {
    let service = GlyphService::new();
    assert_eq!(service.find("😀"), None);
    assert!(service.visited(anything_known).is_empty());
}

#[test]
fn a_visit_is_counted_and_timed() {
    let mut service = GlyphService::new();
    service.register_visit("😀", 1000);

    let entry = service.find("😀").expect("recorded");
    assert_eq!(entry.visit_count, 1);
    assert_eq!(entry.last_visited_at, Some(1000));
}

#[test]
fn visits_accumulate() {
    let mut service = GlyphService::new();
    service.register_visit("😀", 1000);
    service.register_visit("😀", 2000);

    let entry = service.find("😀").expect("recorded");
    assert_eq!(entry.visit_count, 2);
    assert_eq!(entry.last_visited_at, Some(2000));
    assert_eq!(service.entries().len(), 1, "one entry, not two");
}

#[test]
fn pinning_records_when() {
    let mut service = GlyphService::new();
    service.pin("😀", 500);
    assert_eq!(service.find("😀").expect("pinned").pinned_at, Some(500));
}

#[test]
fn unpinning_something_never_pinned_creates_nothing() {
    // Writing an empty entry would put a glyph in the file for no reason.
    let mut service = GlyphService::new();
    assert!(!service.unpin("😀"));
    assert!(service.entries().is_empty());
}

#[test]
fn unpinning_keeps_the_visit_count() {
    // A pin and a history are different things; removing one must not remove
    // the other.
    let mut service = GlyphService::new();
    service.register_visit("😀", 1000);
    service.pin("😀", 1000);

    assert!(service.unpin("😀"));
    let entry = service.find("😀").expect("still there");
    assert_eq!(entry.pinned_at, None);
    assert_eq!(entry.visit_count, 1);
}

#[test]
fn resetting_the_ranking_keeps_the_pin_and_the_tone() {
    let mut service = GlyphService::new();
    service.register_visit("😀", 1000);
    service.pin("😀", 1000);
    assert!(service.set_skin_tone("😀", "medium"));

    assert!(service.reset_ranking("😀"));
    let entry = service.find("😀").expect("still there");
    assert_eq!(entry.visit_count, 0);
    assert_eq!(entry.last_visited_at, None);
    assert_eq!(entry.pinned_at, Some(1000));
    assert_eq!(entry.skin_tone.as_deref(), Some("medium"));
}

#[test]
fn resetting_the_ranking_of_an_unknown_glyph_creates_nothing() {
    let mut service = GlyphService::new();
    assert!(!service.reset_ranking("😀"));
    assert!(service.entries().is_empty());
}

#[test]
fn a_skin_tone_is_stored_by_id_and_can_be_cleared() {
    let mut service = GlyphService::new();
    assert!(service.set_skin_tone("👋", "dark"));
    assert_eq!(
        service.find("👋").expect("set").skin_tone.as_deref(),
        Some("dark")
    );

    assert!(service.reset_skin_tone("👋"));
    assert_eq!(service.find("👋").expect("still there").skin_tone, None);
}

#[test]
fn an_unknown_skin_tone_id_is_rejected_and_stores_nothing() {
    // The C++ takes the SkinTone enum, so an unknown id cannot be produced
    // there. Storing one here would persist a tone `skin_tone_by_id` resolves
    // to None, silently dropping the person's choice on the next load.
    let mut service = GlyphService::new();
    assert!(!service.set_skin_tone("😀", "bogus"));
    assert!(!service.set_skin_tone("👋", "Medium"));
    assert!(service.entries().is_empty());
}

#[test]
fn every_canonical_skin_tone_id_is_accepted() {
    let mut service = GlyphService::new();
    for id in [
        "default",
        "light",
        "medium-light",
        "medium",
        "medium-dark",
        "dark",
    ] {
        assert!(service.set_skin_tone("😀", id));
        assert_eq!(
            service.find("😀").expect("stored").skin_tone.as_deref(),
            Some(id)
        );
    }
}

#[test]
fn resetting_a_tone_never_set_creates_nothing() {
    let mut service = GlyphService::new();
    assert!(!service.reset_skin_tone("👋"));
    assert!(service.entries().is_empty());
}

#[test]
fn empty_keywords_clear_the_field_rather_than_storing_an_empty_string() {
    // A cleared field must weigh nothing in the search, not match everything
    // with a zero-length term.
    let mut service = GlyphService::new();
    service.set_keywords("😀", "happy");
    assert_eq!(
        service.find("😀").expect("set").keyword.as_deref(),
        Some("happy")
    );

    service.set_keywords("😀", "");
    assert_eq!(service.find("😀").expect("still there").keyword, None);
}

#[test]
fn the_visited_list_holds_only_pinned_or_picked_glyphs() {
    // A glyph that only has a skin tone set has not been used, and would pad
    // the recent list with something nobody chose.
    let mut service = GlyphService::new();
    assert!(service.set_skin_tone("👋", "dark"));
    service.register_visit("😀", 1000);

    let visited = service.visited(anything_known);
    assert_eq!(visited.len(), 1);
    assert_eq!(visited[0].emoji, "😀");
}

#[test]
fn pinned_glyphs_come_first_whatever_their_counts() {
    let service = GlyphService::from_entries(vec![
        SerializedEmojiMetadata {
            emoji: "popular".to_owned(),
            visit_count: 100,
            last_visited_at: Some(9999),
            ..SerializedEmojiMetadata::default()
        },
        SerializedEmojiMetadata {
            emoji: "pinned".to_owned(),
            visit_count: 1,
            pinned_at: Some(5),
            ..SerializedEmojiMetadata::default()
        },
    ]);

    let visited = service.visited(anything_known);
    assert_eq!(visited[0].emoji, "pinned");
    assert_eq!(visited[1].emoji, "popular");
}

#[test]
fn the_most_recently_pinned_comes_first() {
    let service = GlyphService::from_entries(vec![
        SerializedEmojiMetadata {
            emoji: "older".to_owned(),
            pinned_at: Some(100),
            ..SerializedEmojiMetadata::default()
        },
        SerializedEmojiMetadata {
            emoji: "newer".to_owned(),
            pinned_at: Some(200),
            ..SerializedEmojiMetadata::default()
        },
    ]);
    assert_eq!(service.visited(anything_known)[0].emoji, "newer");
}

#[test]
fn unpinned_glyphs_sort_by_count_then_by_recency() {
    let service = GlyphService::from_entries(vec![
        SerializedEmojiMetadata {
            emoji: "once-recent".to_owned(),
            visit_count: 1,
            last_visited_at: Some(9999),
            ..SerializedEmojiMetadata::default()
        },
        SerializedEmojiMetadata {
            emoji: "twice-old".to_owned(),
            visit_count: 2,
            last_visited_at: Some(1),
            ..SerializedEmojiMetadata::default()
        },
        SerializedEmojiMetadata {
            emoji: "twice-recent".to_owned(),
            visit_count: 2,
            last_visited_at: Some(5000),
            ..SerializedEmojiMetadata::default()
        },
    ]);

    let order: Vec<_> = service
        .visited(anything_known)
        .iter()
        .map(|entry| entry.emoji.clone())
        .collect();
    assert_eq!(
        order,
        vec![
            "twice-recent".to_owned(),
            "twice-old".to_owned(),
            "once-recent".to_owned()
        ],
        "count first, then recency"
    );
}

#[test]
fn an_entry_for_a_glyph_this_build_no_longer_knows_is_dropped() {
    // An old file can name one, and a row with nothing to draw is worse than a
    // missing row.
    let mut service = GlyphService::new();
    service.register_visit("😀", 1);
    service.register_visit("\u{1FAF7}", 1);

    let visited = service.visited(only(&["😀"]));
    assert_eq!(visited.len(), 1);
    assert_eq!(visited[0].emoji, "😀");
}

#[test]
fn the_json_key_is_emoji_for_backward_compatibility() {
    // The C++ comment says why: renaming it would make every existing file
    // unreadable and silently reset everyone's pins and counts.
    let mut service = GlyphService::new();
    service.register_visit("😀", 1000);
    service.pin("😀", 1000);

    let json: serde_json::Value =
        serde_json::from_str(&service.to_json().expect("serialises")).expect("JSON");
    assert_eq!(json[0]["emoji"], "😀");
    assert_eq!(json[0]["visitCount"], 1);
    assert_eq!(json[0]["pinnedAt"], 1000);
}

#[test]
fn the_cpp_file_is_read_with_its_camel_case_keys() {
    // What glaze writes for `SerializedEmojiMetadata`: the members as they
    // are declared.
    let service = GlyphService::from_json(
        r#"[{"emoji":"👍","visitCount":4,"pinnedAt":1700000000,"lastVisitedAt":1700000100,"skinTone":"medium","keyword":"yes ok"}]"#,
    );
    let entry = service.find("👍").expect("loaded");
    assert_eq!(entry.visit_count, 4);
    assert_eq!(entry.pinned_at, Some(1_700_000_000));
    assert_eq!(entry.last_visited_at, Some(1_700_000_100));
    assert_eq!(entry.skin_tone.as_deref(), Some("medium"));
    assert_eq!(entry.keyword.as_deref(), Some("yes ok"));

    let text = service.to_json().expect("serialises");
    for key in ["visitCount", "pinnedAt", "lastVisitedAt", "skinTone"] {
        assert!(text.contains(key), "{key} missing from {text}");
    }
    assert!(!text.contains("visit_count"), "{text}");
}

#[test]
fn a_file_from_an_older_build_still_loads() {
    // This port's first files were snake_case.
    let service = GlyphService::from_json(r#"[{"emoji":"😀","visit_count":3}]"#);
    let entry = service.find("😀").expect("loaded");
    assert_eq!(entry.visit_count, 3);
    assert_eq!(entry.pinned_at, None);
    assert_eq!(entry.skin_tone, None);
}

#[test]
fn the_file_is_written_and_read_back_and_a_missing_one_is_empty() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("emojis").join("emojis.json");
    assert!(GlyphService::load_file(&path).entries().is_empty());

    let mut service = GlyphService::new();
    service.pin("🍕", 10);
    service.save_file(&path).expect("saved");
    assert_eq!(GlyphService::load_file(&path), service);
    assert!(!path.with_extension("json.partial").exists());
}

#[test]
fn a_visit_raises_a_glyph_among_matches_and_a_keyword_makes_it_match() {
    use compass_core::glyph::lookup;
    use compass_core::glyph_service::score;
    use compass_search::Query;

    let now = 1_000_000;
    let heart = lookup("❤️").expect("in the table");
    let query = Query::new("heart");
    let cold = score(heart, None, &query, now).expect("matches");
    let mut service = GlyphService::new();
    for _ in 0..5 {
        service.register_visit(heart.character, u64::try_from(now).unwrap());
    }
    let warm = score(heart, service.find(heart.character), &query, now).expect("matches");
    assert!(warm > cold, "{warm} should beat {cold}");

    let pizza = lookup("🍕").expect("in the table");
    let mine = Query::new("zzlunch");
    assert!(score(pizza, None, &mine, now).is_none());
    service.set_keywords(pizza.character, "zzlunch");
    assert!(score(pizza, service.find(pizza.character), &mine, now).is_some());
}

#[test]
fn a_file_that_does_not_parse_yields_an_empty_store_not_a_partial_one() {
    // Half-read metadata would rank the picker by a history that never
    // happened.
    assert!(
        GlyphService::from_json("[{\"emoji\":\"😀\"")
            .entries()
            .is_empty()
    );
    assert!(GlyphService::from_json("not json").entries().is_empty());
}

#[test]
fn the_store_round_trips() {
    let mut service = GlyphService::new();
    service.register_visit("😀", 1000);
    service.set_keywords("😀", "happy face");
    service.set_skin_tone("👋", "dark");

    let reloaded = GlyphService::from_json(&service.to_json().expect("serialises"));
    assert_eq!(reloaded, service);
}

#[test]
fn absent_optional_fields_are_left_out_of_the_file() {
    // So a file stays readable, and an entry with only a visit count does not
    // carry four nulls.
    let mut service = GlyphService::new();
    service.register_visit("😀", 1000);

    let text = service.to_json().expect("serialises");
    assert!(!text.contains("pinnedAt"), "{text}");
    assert!(!text.contains("skinTone"), "{text}");
}
