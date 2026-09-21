//! Phase 5 Track A parity harness — verifies builtin breadth without VM.
//! Each builtin already exists in `crates/compass-core/src/` and is reachable
//! via the RootItem registry. This harness ensures they remain wired as Phase 5
//! adds wlroots/Rhai tracks in parallel.

use compass_core::{calculator_history, emoji_grid, file_search};
use compass_search::{Query, WeightedField, score_weighted};

#[test]
fn calculator_live_calc_and_history_actions_are_wired() {
    let calc = calculator_history::live_calc("1+2");
    assert!(matches!(
        calc,
        calculator_history::LiveCalc::Compute(_) | calculator_history::LiveCalc::None
    ));
    let title = calculator_history::live_result_title("1+2", "3");
    assert!(!title.is_empty());
    let panel = calculator_history::history_action_panel(false);
    assert!(!panel.is_empty());
}

#[test]
fn emoji_grid_skin_tone_helpers_are_present() {
    // Emoji grid helpers for skin tones must remain. The dataset regeneration
    // should not remove the tone plumbing.
    assert!(
        emoji_grid::skin_tone_by_id("light").is_some()
            || emoji_grid::skin_tone_by_id("medium").is_some()
    );
    let tone = emoji_grid::effective_tone(None, None);
    // Should return a valid tone, not panic.
    let _ = emoji_grid::apply_skin_tone("👍", tone);
}

#[test]
fn file_search_query_planning_and_fuzzy_scoring_parity() {
    assert!(file_search::is_explicit_path_query("~/doc"));
    assert!(!file_search::is_explicit_path_query("doc"));
    let query = Query::new("doc");
    let m = score_weighted(&[WeightedField::new("document.txt", 1.0)], &query);
    assert!(m.accepted(), "prefix should score");
    let no_m = score_weighted(&[WeightedField::new("zzzzzzzzz", 1.0)], &query);
    assert!(!no_m.accepted(), "gibberish should not score");
}
