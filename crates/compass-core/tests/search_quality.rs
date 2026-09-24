//! What app search OWES A PERSON, asserted absolutely.
//!
//! # Why this is not the parity harness
//!
//! Suite 0 compares this engine's ranking against the C++ one. That is a
//! useful tripwire and a poor specification: Compass is not a byte-for-byte
//! reproduction of vicinae, it is a launcher in the same spirit meant to be
//! faster and better, so a check that reddens whenever the two disagree does
//! not merely tolerate a worse ranking — it blocks a better one.
//!
//! These assertions have no second implementation in them. They say what
//! typing something SHOULD surface, in the form
//! `rank_of("blend", "Blender") == 1`, and they are as true of an engine that
//! beats fzf as of one that matches it. Modelled on
//! `src/file-indexer/tests/query-quality.cpp`, which does the same for file
//! search.
//!
//! # Chosen so the right answer is not a matter of taste
//!
//! Every query below is a prefix that matches exactly ONE displayable name in
//! the 757-entry corpus, verified against the corpus rather than picked by
//! eye. Where a query could reasonably surface several apps, it asserts
//! membership in the top few rather than a specific winner. A test that
//! encodes one person's preference is a test the next person deletes.

use compass_core::AppIndex;
use compass_testkit::corpus::corpus_root;

fn index() -> AppIndex {
    AppIndex::builder()
        .dir(corpus_root().join("desktop-entries").join("real"))
        .build()
}

/// The 1-based position of `title` for `query`, or `None` if it does not rank.
fn rank_of(index: &AppIndex, query: &str, title: &str) -> Option<usize> {
    index
        .search_root(query, None)
        .iter()
        .position(|hit| hit.item.name() == title)
        .map(|zero_based| zero_based + 1)
}

fn in_top(index: &AppIndex, query: &str, title: &str, n: usize) -> bool {
    rank_of(index, query, title).is_some_and(|rank| rank <= n)
}

/// Typing an application's whole name puts it first. The floor of the floor.
#[test]
fn an_exact_name_ranks_first() {
    let index = index();
    for name in [
        "Alacritty",
        "Blender",
        "HandBrake",
        "ClamTk",
        "Bitcoin Core",
    ] {
        assert_eq!(
            rank_of(&index, name, name),
            Some(1),
            "typing {name:?} in full must put it first"
        );
    }
}

/// A prefix that can only mean one application puts it first.
///
/// Each of these matches exactly one displayable name in the corpus, so there
/// is no second candidate to argue for.
#[test]
fn an_unambiguous_prefix_ranks_first() {
    let index = index();
    for (query, name) in [
        ("alacr", "Alacritty"),
        ("blend", "Blender"),
        ("handb", "HandBrake"),
        ("clam", "ClamTk"),
        ("akre", "Akregator"),
    ] {
        assert_eq!(
            rank_of(&index, query, name),
            Some(1),
            "{query:?} can only mean {name:?}"
        );
    }
}

/// Case is not a thing a person should have to get right.
#[test]
fn case_does_not_change_the_answer() {
    let index = index();
    for query in ["blender", "BLENDER", "BlEnDeR"] {
        assert_eq!(
            rank_of(&index, query, "Blender"),
            Some(1),
            "{query:?} must find Blender"
        );
    }
}

/// A multi-word name is findable by typing it with its space.
#[test]
fn a_multi_word_name_is_findable_as_typed() {
    let index = index();
    assert_eq!(rank_of(&index, "bitcoin core", "Bitcoin Core"), Some(1));
    assert_eq!(rank_of(&index, "bitcoin", "Bitcoin Core"), Some(1));
}

/// A DROPPED character still finds the application.
///
/// The matcher is a subsequence matcher, so a query missing a letter is still
/// a subsequence of the name and ranks normally. This is the typo shape we
/// handle today, and it is worth pinning before the two below are fixed.
#[test]
fn a_dropped_character_still_finds_it() {
    let index = index();
    for (typo, name) in [
        ("alacrity", "Alacritty"),
        ("hanbrake", "HandBrake"),
        ("blener", "Blender"),
    ] {
        assert!(
            in_top(&index, typo, name, 3),
            "{typo:?} must still surface {name:?} in the top 3, ranked {:?}",
            rank_of(&index, typo, name)
        );
    }
}

/// A TRANSPOSED or DOUBLED character finds NOTHING AT ALL, and should not.
///
/// # A real gap, and the reason this file exists
///
/// A subsequence matcher requires every query character to appear in order.
/// A transposition breaks the order and an extra keystroke has no character
/// to match, so both drop the application out of the results entirely —
/// not ranked low, absent:
///
/// ```text
///   alacrtity   (transposed)  -> Alacritty: not ranked
///   alacrittyy  (doubled y)   -> Alacritty: not ranked
///   alacrity    (dropped t)   -> Alacritty: ranked, fine
/// ```
///
/// Typing a letter twice is an ordinary slip, and it currently makes the app
/// you are looking at disappear.
///
/// The file indexer already solves this — `src/file-indexer/tests/query-quality.cpp`
/// asserts `inTop("budgte", "budget_2024.xlsx", 3)` and
/// `inTop("mayonaise", "mayonnaise.flac", 3)`, backed by a spellfix fallback.
/// App search has no equivalent.
///
/// IGNORED, NOT DELETED. The assertion is right and the engine does not meet
/// it yet; deleting it would remove the only record that a person typing
/// `alacrittyy` gets nothing. Tracked in #205.
///
/// NOTE this is exactly the kind of gap the Suite 0 differential cannot
/// find: the C++ app search is a subsequence matcher too, so both engines
/// agree — on being unhelpful.
#[test]
#[ignore = "app search has no typo fallback for transposed or doubled characters; see #205"]
fn a_transposed_or_doubled_character_still_finds_it() {
    let index = index();
    for (typo, name) in [
        ("alacrtity", "Alacritty"),
        ("alacrittyy", "Alacritty"),
        ("blneder", "Blender"),
        ("handbrkae", "HandBrake"),
    ] {
        assert!(
            in_top(&index, typo, name, 3),
            "{typo:?} must still surface {name:?} in the top 3, ranked {:?}",
            rank_of(&index, typo, name)
        );
    }
}

/// An empty query lists applications rather than nothing.
///
/// The pre-typing state of a launcher. Returning nothing here would make the
/// window useless until the first keystroke.
#[test]
fn an_empty_query_lists_the_catalog() {
    let index = index();
    let hits = index.search_root("", None);
    assert!(
        hits.len() > 400,
        "an empty query should list the catalog, got {}",
        hits.len()
    );
}

/// CONTROL. The corpus these assertions run against must actually be loaded.
///
/// Every assertion above is satisfied by an index that ranks nothing, if
/// `rank_of` returns `None` and the test asserts `None`. It does not — they
/// assert `Some(1)` — but the corpus itself could still silently be empty in
/// a checkout where the path moved, and then `an_empty_query_lists_the_catalog`
/// is the only thing standing between that and a green run. This says it
/// outright.
#[test]
fn the_corpus_is_really_there() {
    let entries = std::fs::read_dir(corpus_root().join("desktop-entries").join("real"))
        .expect("the corpus directory exists")
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "desktop"))
        .count();
    assert!(
        entries > 700,
        "the quality assertions are measured against this corpus; found {entries} entries"
    );
}
