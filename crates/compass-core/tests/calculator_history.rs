//! The calculator history view's own decisions.
//!
//! Ported from `src/server/src/builtins/calculator/`.

use compass_core::calculator_history::{
    CALCULATOR_MIN_CHARS, LiveCalc, history_action_panel, history_icon, live_action_panel,
    live_calc, live_result_title, visible_groups,
};

fn computes(text: &str) -> Option<String> {
    match live_calc(text) {
        LiveCalc::Compute(body) => Some(body),
        LiveCalc::None => None,
    }
}

// --- when the search box is also a calculator ---------------------------

#[test]
fn short_text_is_only_a_search() {
    // Almost any two characters parse as something, and a result flickering
    // into the list on the way to typing a word is worse than no result.
    assert_eq!(computes(""), None);
    assert_eq!(computes("1"), None);
    assert_eq!(computes("1+"), None);
}

#[test]
fn three_characters_are_enough() {
    assert_eq!(computes("1+1").as_deref(), Some("1+1"));
}

#[test]
fn the_threshold_is_three() {
    assert_eq!(CALCULATOR_MIN_CHARS, 3);
}

#[test]
fn an_equals_sign_asks_explicitly_however_short_the_rest() {
    // This is the whole point of the prefix: it gets under the length rule.
    assert_eq!(computes("=1").as_deref(), Some("1"));
}

#[test]
fn the_equals_sign_is_stripped_before_computing() {
    assert_eq!(computes("=1+1").as_deref(), Some("1+1"));
}

#[test]
fn a_bare_equals_sign_still_asks() {
    // The C++ hands the empty string to the backend and lets it decline, so
    // this is not short-circuited here.
    assert_eq!(computes("=").as_deref(), Some(""));
}

#[test]
fn an_equals_sign_elsewhere_is_not_a_prefix() {
    assert_eq!(computes("x="), None);
    assert_eq!(computes("x=y").as_deref(), Some("x=y"));
}

#[test]
fn the_length_is_counted_in_characters_not_bytes() {
    // "éàü" is three characters and six bytes. Counting bytes would let a
    // two-character accented word through the gate.
    assert_eq!(computes("éà"), None);
    assert_eq!(computes("éàü").as_deref(), Some("éàü"));
}

// --- the live row -------------------------------------------------------

#[test]
fn the_live_row_reads_as_an_equation() {
    assert_eq!(live_result_title("1+1", "2"), "1+1 = 2");
}

#[test]
fn the_separator_carries_its_own_spaces() {
    // The spaces are part of the title rather than something the view draws,
    // so they belong here and not in the theme.
    assert_eq!(live_result_title("a", "b"), "a = b");
}

// --- icons --------------------------------------------------------------

#[test]
fn a_conversion_gets_the_switch_glyph() {
    assert_eq!(history_icon(true), "switch");
}

#[test]
fn ordinary_arithmetic_gets_the_calculator_glyph() {
    assert_eq!(history_icon(false), "calculator");
}

// --- the action panels --------------------------------------------------

#[test]
fn a_remembered_row_offers_three_sections() {
    // The sections are where the panel draws its separators, so the shape is
    // behaviour and not layout.
    assert_eq!(history_action_panel(false).len(), 3);
}

#[test]
fn an_unpinned_row_offers_to_pin_and_a_pinned_one_to_unpin() {
    assert_eq!(history_action_panel(false)[0][0].id, "pin");
    assert_eq!(history_action_panel(true)[0][0].id, "unpin");
}

#[test]
fn copying_the_answer_is_what_the_return_key_does() {
    // The question is what was typed; the answer is what is wanted.
    let panel = history_action_panel(false);
    let primary: Vec<_> = panel
        .iter()
        .flatten()
        .filter(|action| action.primary)
        .collect();
    assert_eq!(primary.len(), 1);
    assert_eq!(primary[0].id, "copy-answer");
}

#[test]
fn all_three_copies_are_offered_together() {
    let panel = history_action_panel(false);
    let ids: Vec<_> = panel[1].iter().map(|a| a.id).collect();
    assert_eq!(ids, ["copy-answer", "copy-question", "copy-expression"]);
}

#[test]
fn the_destructive_actions_sit_behind_a_separator_of_their_own() {
    // They are kept away from the primary action by a section break rather
    // than by a confirmation, so the section boundary is the safeguard.
    let panel = history_action_panel(false);
    let ids: Vec<_> = panel[2].iter().map(|a| a.id).collect();
    assert_eq!(ids, ["remove", "remove-all"]);
}

#[test]
fn the_live_result_offers_one_section() {
    assert_eq!(live_action_panel(false).len(), 1);
}

#[test]
fn the_live_result_copies_the_answer_first() {
    let panel = live_action_panel(false);
    assert!(panel[0][0].primary);
    assert_eq!(panel[0][0].id, "copy-answer");
}

#[test]
fn an_unformatted_answer_adds_a_third_action() {
    assert_eq!(live_action_panel(false)[0].len(), 2);
    assert_eq!(live_action_panel(true)[0].len(), 3);
    assert_eq!(live_action_panel(true)[0][2].id, "copy-unformatted-answer");
}

#[test]
fn a_result_with_no_unformatted_form_gets_no_duplicate_copy() {
    // Without the check the panel would carry two actions that do the same
    // thing whenever the answer was already unformatted.
    let ids: Vec<_> = live_action_panel(false)[0].iter().map(|a| a.id).collect();
    assert!(!ids.contains(&"copy-unformatted-answer"));
}

// --- the view's half of the grouping ------------------------------------

#[test]
fn empty_groups_are_dropped_by_the_view() {
    let groups = vec![
        ("Pinned".to_owned(), Vec::<u8>::new()),
        ("Today".to_owned(), vec![1]),
        ("This week".to_owned(), Vec::new()),
    ];
    let names: Vec<_> = visible_groups(groups)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names, ["Today"]);
}

#[test]
fn a_group_with_rows_keeps_its_order_and_contents() {
    let groups = vec![("Today".to_owned(), vec![3, 1, 2])];
    assert_eq!(
        visible_groups(groups),
        [("Today".to_owned(), vec![3, 1, 2])]
    );
}
