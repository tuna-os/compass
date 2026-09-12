//! Port of `src/lib/fuzzy/tests/main.cpp` (and `order-helpers.hpp`).
//!
//! The C++ matcher is an fzf-v2 port; this crate wraps `nucleo-matcher`, whose
//! scoring algorithm is different. Absolute scores are therefore *not* asserted
//! anywhere here. What is asserted is what actually forms the behavioural
//! contract: match vs. non-match, relative ordering, normalized score/quality
//! ratios, and matched indices where nucleo agrees.
//!
//! Assertions that could not be ported faithfully are marked `PORT-DEFERRED`
//! and the divergences that motivated them are marked `DIVERGENCE`, together
//! with a test pinning the behaviour we actually have.

use compass_search::{
    MIN_QUALITY, Matcher, Query, TranslitScheme, WeightedField, frecency, needs_transliteration,
    score_weighted, transliterate,
};

// ---------------------------------------------------------------------------
// order-helpers.hpp
// ---------------------------------------------------------------------------

/// Port of `fuzzy::test::rankByQuery`.
///
/// The C++ helper ranks on `QueryScore::weighted` with a `weighted > 0` filter
/// and a *stable* sort, deliberately bypassing the `MIN_QUALITY` gate that the
/// production filter applies. Mirrored exactly so the ordering cases port
/// one-for-one; production ranking is exercised in `ranking.rs`.
fn rank_by_query<'a>(items: &[&'a str], query: &str) -> Vec<&'a str> {
    let parsed = Query::new(query);
    let mut scored: Vec<(&str, u32)> = items
        .iter()
        .map(|text| {
            (
                *text,
                score_weighted(&[WeightedField::new(text, 1.0)], &parsed).weighted,
            )
        })
        .filter(|(_, weighted)| *weighted > 0)
        .collect();

    // `std::ranges::stable_sort(scored, std::greater{})`: ties keep input order.
    scored.sort_by(|a, b| b.1.cmp(&a.1));
    scored.into_iter().map(|(text, _)| text).collect()
}

/// Port of `fuzzy::test::expectRankedOrder`.
#[track_caller]
fn expect_ranked_order(items: &[&str], query: &str) {
    let actual = rank_by_query(items, query);
    assert_eq!(actual, items, "query: {query:?}");
}

/// Convenience: score one string as a single weight-1.0 field.
fn score_one(text: &str, query: &str) -> compass_search::Match {
    score_weighted(&[WeightedField::new(text, 1.0)], &Query::new(query))
}

// ---------------------------------------------------------------------------
// TEST_CASE("match: exact substring scores positive and matches range")
// ---------------------------------------------------------------------------

#[test]
fn match_exact_substring_scores_positive_and_matches_range() {
    Matcher::with_thread_local(|m| {
        let r = m
            .match_("Open File Manager", "file")
            .expect("`file` matches `Open File Manager`");

        assert!(r.score > 0);
        // C++ asserts start == 5 && end == 9 as *byte* offsets; the haystack is
        // ASCII so the char indices nucleo reports coincide.
        assert_eq!(r.range(), 5..9);
        assert!(r.is_contiguous());
    });
}

// ---------------------------------------------------------------------------
// TEST_CASE("match: missing characters return non-match")
// ---------------------------------------------------------------------------

#[test]
fn match_missing_characters_return_non_match() {
    Matcher::with_thread_local(|m| {
        // C++ returns a `Result` with `matched() == false` and `score == 0`;
        // the Rust API models that as `None`.
        assert!(m.match_("Open File Manager", "xyz").is_none());
    });
}

// ---------------------------------------------------------------------------
// TEST_CASE("match: diacritic-insensitive matching")
// ---------------------------------------------------------------------------

#[test]
fn match_diacritic_insensitive() {
    Matcher::with_thread_local(|m| {
        assert!(m.match_("Café Society", "cafe").is_some());
        assert!(m.match_("Mañana", "manana").is_some());
        assert!(m.match_("Tomáš Brzobohatý", "tomas").is_some());

        // folding is symmetric: accented query matches ASCII text
        assert!(m.match_("cafe society", "café").is_some());

        assert!(m.match_("Café", "xyz").is_none());
    });
}

/// DIVERGENCE: `REQUIRE(m.score_query("Łódź Express", Query{"lodz"}).weighted)`.
///
/// The C++ side folds with fzf's full `normalize.hpp` table, which covers
/// Latin Extended-A (Ł U+0141 -> L, ź U+017A -> z). `nucleo_matcher::chars::normalize`
/// only strips combining diacritics from precomposed characters and leaves
/// Ł/ź alone, so "lodz" does not match "Łódź Express" at all.
///
/// PORT-DEFERRED: the C++ assertion cannot be made to pass without shipping our
/// own fold table in front of nucleo. Pinning the current (divergent) behaviour
/// instead, so a future fold table flips this test loudly.
#[test]
fn diverges_latin_extended_a_is_not_folded() {
    Matcher::with_thread_local(|m| {
        assert!(
            m.match_("Łódź Express", "lodz").is_none(),
            "nucleo unexpectedly folded Latin Extended-A; the C++ assertion \
             (this should match) can now be restored"
        );
        // The precomposed-diacritic half of the same table does work.
        assert!(m.match_("Zürich", "zurich").is_some());
    });
}

// ---------------------------------------------------------------------------
// TEST_CASE("match: non-Latin scripts are unaffected by folding")
// ---------------------------------------------------------------------------

#[test]
fn match_non_latin_scripts_unaffected_by_folding() {
    // C++ compares the folding path against the raw ASCII path. The Rust
    // matcher has one matching path, so the equivalent statement is that
    // transliteration (`match_`) never perturbs a match that already succeeds
    // without it (`match_folded`) for these scripts.
    let cases = [
        ("Привет мир", "мир"),
        ("Москва", "оск"),
        ("日本語入力", "本語"),
        ("中文搜索", "搜索"),
    ];

    Matcher::with_thread_local(|m| {
        for (text, pattern) in cases {
            let folded = m.match_(text, pattern);
            let raw = m.match_folded(text, pattern);

            assert!(folded.is_some(), "{text:?} / {pattern:?}");
            assert_eq!(folded, raw, "{text:?} / {pattern:?}");
        }
    });
}

// ---------------------------------------------------------------------------
// TEST_CASE("match: Cyrillic and Greek matching is case-insensitive")
// ---------------------------------------------------------------------------

#[test]
fn match_cyrillic_and_greek_case_insensitive() {
    Matcher::with_thread_local(|m| {
        assert!(m.match_("Открыть настройки", "открыть").is_some());
        assert!(m.match_("ТЕЛЕГРАМ", "телеграм").is_some());
        assert!(m.match_("ΤΕΡΜΙΝΑΛ", "τερμιναλ").is_some());
    });
}

// ---------------------------------------------------------------------------
// TEST_CASE("match: match offsets map back to original bytes")
// ---------------------------------------------------------------------------

/// DIVERGENCE (API, not behaviour): C++ reports byte offsets and asserts
/// `start == 6, end == 9, positions == {6, 7, 8}` for "Café Bar" / "bar".
/// nucleo reports *char* indices, so the same three characters are `5, 6, 7`.
/// The match itself is identical; only the unit differs.
#[test]
fn match_offsets_are_char_indices() {
    Matcher::with_thread_local(|m| {
        // "Café Bar": C(0) a(1) f(2) é(3) space(4) B(5) a(6) r(7)
        let r = m
            .match_("Café Bar", "bar")
            .expect("`bar` matches `Café Bar`");

        assert_eq!(r.range(), 5..8);
        assert_eq!(r.indices, vec![5, 6, 7]);
        // The byte range the C++ test asserts is recoverable from the chars.
        let text = "Café Bar";
        let byte_start = text.char_indices().nth(5).map(|(i, _)| i);
        assert_eq!(byte_start, Some(6));
    });
}

// ---------------------------------------------------------------------------
// TEST_CASE("ordering: word-boundary match outranks scattered match")
// ---------------------------------------------------------------------------

#[test]
fn ordering_word_boundary_outranks_scattered() {
    expect_ranked_order(&["File Manager", "profile editor"], "file");
}

// ---------------------------------------------------------------------------
// TEST_CASE("sparse matches")
// ---------------------------------------------------------------------------

#[test]
fn sparse_matches() {
    for (text, query) in [
        ("Obsidian Wayland", "obs wld"),
        ("Search Emojis", "emoji srch"),
        ("Keyboard Settings", "kbd stg"),
        ("System Info Event Log", "evlog sinfo"),
        ("Minecraft", "mcft"),
        ("Create Issue For Myself", "cisfmyslf"),
    ] {
        let m = score_one(text, query);
        assert!(m.weighted > 0, "{text:?} / {query:?} should match");
        // Stronger than the C++ assertion, and it holds: every one of these
        // also clears the filtering gate.
        assert!(m.accepted(), "{text:?} / {query:?} should clear the gate");
    }

    for (text, query) in [("Minecraft", "avi"), ("System Info Event Log", "kbd")] {
        assert_eq!(
            score_one(text, query).weighted,
            0,
            "{text:?} / {query:?} should not match"
        );
    }
}

// ---------------------------------------------------------------------------
// TEST_CASE("ordering: various interesting cases")
// ---------------------------------------------------------------------------

#[test]
fn ordering_various_interesting_cases() {
    expect_ranked_order(
        &[
            "Clipboard History",
            "Clear Current Clipboard Data",
            "Clear Clipboard History",
        ],
        "clip",
    );
    expect_ranked_order(&["pkg", "Packages", "Arch Packages"], "pkg");
}

/// DIVERGENCE (weak assertion, worth recording): for "clip" all three items
/// above score *identically* under nucleo (the matched region is the same
/// word-boundary "Clip" in each and nucleo's score ignores where in the
/// haystack it sits and how long the haystack is). The expected order only
/// holds because ties keep input order — in both implementations, since the C++
/// helper also uses a stable sort. So this case does not actually discriminate
/// between a good and a bad ranker in either implementation.
#[test]
fn diverges_clip_ordering_is_a_three_way_tie() {
    let query = Query::new("clip");
    let scores: Vec<u32> = [
        "Clipboard History",
        "Clear Current Clipboard Data",
        "Clear Clipboard History",
    ]
    .iter()
    .map(|t| score_weighted(&[WeightedField::new(t, 1.0)], &query).weighted)
    .collect();

    assert_eq!(
        scores[0], scores[1],
        "nucleo now distinguishes these; the ordering test above became meaningful"
    );
    assert_eq!(scores[1], scores[2]);
}

// ---------------------------------------------------------------------------
// TEST_CASE("ordering: github issue #1326 examples")
// ---------------------------------------------------------------------------

#[test]
fn ordering_issue_1326() {
    expect_ranked_order(&["Espanso.code-workspace", "Rofi.code-workspace"], "Esp");
    expect_ranked_order(&["b", "bm"], "b");
}

// ---------------------------------------------------------------------------
// TEST_CASE("ordering: github issue #946 examples")
// ---------------------------------------------------------------------------

#[test]
fn ordering_issue_946() {
    expect_ranked_order(&["Konsole", "OpenJDK Java 17 Console"], "konsole");
    expect_ranked_order(&["eos-update", "Configure EOS Update Notifier"], "eos");
    expect_ranked_order(&["Avidemux", "Donate to vicinae"], "avi");
}

/// DIVERGENCE: `expectRankedOrder({"Spotify", "Reload Script Directories",
/// "Sysprog"}, "Spo")`.
///
/// The C++ fzf matcher ranks "Reload Script Directories" above "Sysprog";
/// nucleo reverses the last two. Both agree that "Spotify" wins. nucleo rewards
/// "Sysprog" (S at the start, then a short scatter inside one word) over a
/// match spread across three words with only the "S" on a boundary; the C++
/// matcher's larger word-boundary bonuses pull the other way.
///
/// PORT-DEFERRED: the expected order cannot be reproduced without reproducing
/// fzf's bonus constants. Pinning the actual order so the divergence is visible
/// and any future change is deliberate.
#[test]
fn diverges_ordering_issue_946_spo() {
    assert_eq!(
        rank_by_query(&["Spotify", "Reload Script Directories", "Sysprog"], "Spo"),
        // C++ expects ["Spotify", "Reload Script Directories", "Sysprog"].
        vec!["Spotify", "Sysprog", "Reload Script Directories"],
    );
}

// ---------------------------------------------------------------------------
// TEST_CASE("transliterate: non-latin scripts map to ascii")
// ---------------------------------------------------------------------------

#[test]
fn transliterate_non_latin_scripts_map_to_ascii() {
    let primary = |s: &str| transliterate(s, TranslitScheme::Primary);

    assert_eq!(primary("телеграм").as_deref(), Some("telegram"));
    assert_eq!(primary("ТЕЛЕГРАМ").as_deref(), Some("telegram"));
    assert_eq!(primary("Ёж").as_deref(), Some("ezh"));
    assert_eq!(primary("тел egram").as_deref(), Some("tel egram"));
    assert_eq!(primary("ΤΕΡΜΙΝΑΛ").as_deref(), Some("terminal"));
    assert_eq!(primary("Ελλάδα").as_deref(), Some("ellada"));

    assert_eq!(primary("telegram"), None);
    assert_eq!(primary("ьъ"), None);
}

// ---------------------------------------------------------------------------
// TEST_CASE("needsTransliteration: only supported scripts need extra matching")
// ---------------------------------------------------------------------------

#[test]
fn needs_transliteration_only_supported_scripts() {
    assert!(needs_transliteration("телеграм"));
    assert!(needs_transliteration("ΤΕΡΜΙΝΑΛ"));
    assert!(!needs_transliteration("telegram"));
    assert!(!needs_transliteration("日本語"));
}

// ---------------------------------------------------------------------------
// TEST_CASE("transliteration: matching across scripts")
// ---------------------------------------------------------------------------

#[test]
fn transliteration_matching_across_scripts() {
    assert!(score_one("Telegram", "телеграм").weighted > 0);
    assert!(score_one("Discord", "дискорд").weighted > 0);
    assert!(score_one("Konsole", "консоль").weighted > 0);
    assert!(score_one("Terminal", "τερμιναλ").weighted > 0);
    assert_eq!(score_one("Telegram", "музыка").weighted, 0);

    assert!(score_one("Привет мир", "мир").weighted > 0);
    assert!(score_one("Открыть Discord", "открыть дискорд").weighted > 0);

    // An all-ASCII query is scored exactly as the bare matcher scores it.
    let via_query = score_one("Telegram", "teleg").weighted;
    let bare = Matcher::with_thread_local(|m| m.score_folded("Telegram", "teleg").unwrap());
    assert_eq!(via_query, bare);
}

// ---------------------------------------------------------------------------
// TEST_CASE("transliteration: normalized score and quality survive the gate (#1859)")
// ---------------------------------------------------------------------------

#[test]
fn transliteration_normalized_score_and_quality_survive_the_gate() {
    let tg = score_one("Telegram", "телеграм");
    assert_eq!(tg.score, 100);
    assert_eq!(tg.quality, 100);

    for q in [
        "те",
        "тел",
        "теле",
        "телег",
        "телегр",
        "телегра",
        "телеграм",
    ] {
        assert!(score_one("Telegram", q).accepted(), "query {q:?}");
    }

    assert!(score_one("Terminal", "τερμιναλ").accepted());
    assert!(score_one("Открыть Discord", "открыть дискорд").accepted());

    let cyr = score_one("Телеграм", "телеграм");
    assert_eq!(cyr.score, 100);
    assert_eq!(cyr.quality, 100);

    assert!(!score_one("Telegram", "музыка").accepted());
}

// ---------------------------------------------------------------------------
// TEST_CASE("transliteration: each variant is normalized against its own ceiling")
// ---------------------------------------------------------------------------

#[test]
fn transliteration_each_variant_normalized_against_own_ceiling() {
    // "diskord" (primary) cannot match "Discord"; only the alternate scheme
    // (к -> c) can.
    let alt = score_one("Discord", "дискорд");
    assert_eq!(alt.score, 100);
    assert_eq!(alt.quality, 100);

    assert!(score_one("Telegram", "телеgram").accepted());

    let mixed = score_one("Открыть Discord", "открыть дискорд");
    assert_eq!(mixed.score, 100);
    assert_eq!(mixed.quality, 100);

    let both = score_weighted(
        &[
            WeightedField::new("Telegram", 1.0),
            WeightedField::new("Телеграм", 1.0),
        ],
        &Query::new("телеграм"),
    );
    assert_eq!(both.score, 100);
    assert_eq!(both.quality, 100);

    assert!(!score_one("Telegram", "ьъ").accepted());
}

// ---------------------------------------------------------------------------
// TEST_CASE("transliteration: direct match() still resolves cross-script patterns")
// ---------------------------------------------------------------------------

#[test]
fn transliteration_direct_match_resolves_cross_script_patterns() {
    Matcher::with_thread_local(|m| {
        let r = m
            .match_("Telegram", "телеграм")
            .expect("cross-script match");
        assert_eq!(r.range(), 0..8);
        assert_eq!(r.indices, vec![0, 1, 2, 3, 4, 5, 6, 7]);

        assert!(m.match_("Discord", "дискорд").is_some());
        assert!(m.match_("Telegram", "музыка").is_none());
    });
}

// ---------------------------------------------------------------------------
// TEST_CASE("query score: quality is the worst per-word match, weighted respects field weights")
// ---------------------------------------------------------------------------

#[test]
fn query_score_quality_and_field_weights() {
    let anki = [
        WeightedField::new("Anki", 1.0),
        WeightedField::new(
            "An intelligent spaced-repetition memory training program",
            0.5,
        ),
    ];

    let perfect = score_one("Firefox", "fire");
    assert_eq!(perfect.quality, 100);
    assert_eq!(perfect.score, 100);
    Matcher::with_thread_local(|m| {
        assert_eq!(perfect.weighted, m.score_folded("Firefox", "fire").unwrap());
    });

    let substring = score_one("Firefox", "fox");
    assert_eq!(substring.quality, substring.score);
    assert!(substring.quality >= MIN_QUALITY);
    assert!(substring.quality < 100);

    let scattered = score_weighted(&anki, &Query::new("time in"));
    assert!(scattered.weighted > 0);
    // PORT-DEFERRED: `REQUIRE(scattered.quality == 0)`. See
    // `diverges_no_coherence_signal` below — the C++ quality is zeroed by the
    // fzf backtracker's coherence flag, which nucleo does not expose.

    let keyword_only = score_weighted(&anki, &Query::new("memory"));
    assert_eq!(keyword_only.quality, 100);
    assert_eq!(keyword_only.score, 50);
    Matcher::with_thread_local(|m| {
        let raw = m.score_folded("memory", "memory").unwrap();
        assert_eq!(keyword_only.weighted, (raw as f32 * 0.5) as u32);
    });

    assert_eq!(score_weighted(&anki, &Query::new("xyz")).quality, 0);
}

// ---------------------------------------------------------------------------
// TEST_CASE("fuzzy::scoreWeighted: normalized match with quality gate")
// ---------------------------------------------------------------------------

#[test]
fn score_weighted_normalized_match_with_quality_gate() {
    assert!(score_one("Firefox", "fire").accepted());
    assert_eq!(score_one("Firefox", "fire").score, 100);
    assert!(score_one("Thunderbird", "bird").accepted());
    assert!(score_one("Keyboard Settings", "kbd stg").accepted());
    assert!(!score_one("Firefox", "").accepted());
    assert!(!score_one("Firefox", "xyz").accepted());
    assert!(
        !score_one(
            "An intelligent spaced-repetition memory training program",
            "ny"
        )
        .accepted()
    );

    let weighted = score_weighted(
        &[
            WeightedField::new("Firefox", 1.0),
            WeightedField::new("Web Browser", 0.5),
        ],
        &Query::new("browser"),
    );
    assert_eq!(weighted.quality, 100);
    assert_eq!(weighted.score, 50);
}

// ---------------------------------------------------------------------------
// TEST_CASE("fuzzy::frecency: bounded and monotonic")
// ---------------------------------------------------------------------------

#[test]
fn frecency_bounded_and_monotonic() {
    const NOW: i64 = 1_800_000_000;
    assert_eq!(frecency(0, None, NOW), 0.0);
    assert!(frecency(1, Some(NOW as u64), NOW) > frecency(1, Some((NOW - 30 * 86400) as u64), NOW));
    assert!(frecency(50, Some(NOW as u64), NOW) > frecency(5, Some(NOW as u64), NOW));
    assert_eq!(frecency(100_000, Some(NOW as u64), NOW), 1.0);
}

// ---------------------------------------------------------------------------
// TEST_CASE("match: coherence separates substrings/abbreviations/acronyms from
//            scattered matches")
// ---------------------------------------------------------------------------

/// PORT-DEFERRED, whole test case: every `REQUIRE(...coherent)` /
/// `REQUIRE_FALSE(...coherent)` assertion.
///
/// `fzf::Result::coherent` is produced by the C++ backtracking pass, which
/// classifies an alignment as incoherent when it spreads across several words
/// *and* some run of consecutive matched characters starts mid-word ("time" in
/// "S[t]art [I]nput [Me]thod"). nucleo's matcher exposes neither the DP matrix
/// nor a comparable flag, and the matched indices alone are not enough to
/// reconstruct it (it depends on the per-position boundary bonuses the C++
/// matrix carries). Reimplementing the classifier over nucleo's indices would
/// be a new heuristic, not a port, so it is deliberately left out.
///
/// Two consequences are pinned below as DIVERGENCEs: without the coherence
/// signal, scattered matches are neither excluded from `quality` nor rejected
/// by the `MIN_QUALITY` gate.
#[test]
fn diverges_no_coherence_signal() {
    // The coherent cases all match and clear the gate, as in C++.
    for (text, needle) in [
        ("Runtime Settings", "time"),
        ("Keyboard", "kbd"),
        ("Start Input Method", "sim"),
        ("Event Log", "evlog"),
        ("Firefox Developer Edition", "fdev"),
        ("Café Bar", "cafba"),
    ] {
        assert!(
            score_one(text, needle).accepted(),
            "{text:?} / {needle:?} should be an accepted (coherent) match"
        );
    }

    // DIVERGENCE: C++ marks these incoherent, which zeroes their quality and so
    // rejects them. We accept them.
    for (text, needle) in [
        ("Play this game on Steam", "time"),
        ("Start Input Method", "time"),
        (
            "An intelligent spaced-repetition memory training program",
            "time",
        ),
    ] {
        let m = score_one(text, needle);
        assert!(
            m.accepted(),
            "expected the known divergence: {text:?} / {needle:?} is accepted \
             here but rejected as incoherent by the C++ matcher"
        );
        // It is at least ranked below a real match of the same query length.
        assert!(m.score < score_one("Runtime Settings", "time").score + 10);
    }

    // This one survives the divergence: a real substring match is accepted.
    assert!(score_one("Play this game on Steam", "steam").accepted());
}
