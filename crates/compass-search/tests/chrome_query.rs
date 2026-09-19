//! Issue #105: a search for a Flatpak application's name must reach it.
//!
//! The report is that Flatpak applications do not appear in search. Indexing
//! and matching are separate halves and only one of them can be at fault; this
//! pins the matching half so the investigation does not have to revisit it.

use compass_search::rank;

fn corpus() -> Vec<String> {
    ["Files", "Firefox", "Google Chrome", "Terminal", "Settings"]
        .iter()
        .map(|name| (*name).to_owned())
        .collect()
}

#[test]
fn every_plausible_query_for_chrome_reaches_it() {
    // A two-word title searched by its second word is the case worth pinning:
    // a matcher anchored to the start of the string would find "google" and
    // miss "chrome", which is precisely what the reporter typed.
    let corpus = corpus();
    for query in ["chrome", "Chrome", "chr", "goog", "google chrome"] {
        let hits = rank(query, &corpus);
        assert!(
            hits.iter().any(|scored| scored.item == "Google Chrome"),
            "{query:?} did not reach Google Chrome"
        );
    }
}

#[test]
fn a_query_matching_nothing_returns_nothing() {
    // The control for the test above: if `rank` returned everything, the
    // assertions there would pass without meaning anything.
    let corpus = corpus();
    assert!(rank("zzzzqqq", &corpus).is_empty());
}
