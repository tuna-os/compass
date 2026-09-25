//! The root search view's own behaviour.
//!
//! Ported from `src/server/src/builtins/root/`.

use compass_core::root_view::{
    ClockState, SpaceOutcome, alias_form_initial_value, alias_form_title, alias_submit_outcome,
    clock_state, history_entry_at, next_clock_tick_secs, next_history_offset, on_action_executed,
    space_outcome, text_change_resets_history, up_cycles_history,
};

fn hist(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

// --- the clock ----------------------------------------------------------

#[test]
fn the_clock_ticks_on_the_boundary_rather_than_an_interval_from_now() {
    // A clock showing minutes updates *on* the minute instead of drifting to
    // whenever the window happened to open.
    assert_eq!(next_clock_tick_secs(1_700_000_000, 60), 40);
    assert_eq!(next_clock_tick_secs(1_700_000_020, 60), 20);
}

#[test]
fn a_clock_exactly_on_the_boundary_waits_a_whole_interval() {
    assert_eq!(next_clock_tick_secs(1_700_000_040, 60), 60);
}

#[test]
fn a_clock_re_enabled_mid_interval_falls_back_into_step_with_one_short_tick() {
    // Rather than being permanently offset by however late it started.
    // 1_700_000_039 sits one second before a minute boundary.
    let delay = next_clock_tick_secs(1_700_000_039, 60);
    assert_eq!(delay, 1);
}

#[test]
fn a_nonsense_interval_does_not_produce_a_nonsense_delay() {
    assert_eq!(next_clock_tick_secs(1_700_000_000, 0), 0);
    assert_eq!(next_clock_tick_secs(1_700_000_000, -60), 0);
}

#[test]
fn a_disabled_clock_clears_the_title_as_well_as_stopping() {
    // Leaving the last time on screen would show a clock that had silently
    // stopped, which is worse than no clock.
    assert_eq!(clock_state(false, None, 60, 1_700_000_000), ClockState::Off);
    assert_eq!(
        clock_state(false, Some("HH:mm"), 60, 1_700_000_000),
        ClockState::Off
    );
}

#[test]
fn an_enabled_clock_reports_its_next_tick_and_whether_it_is_custom() {
    assert_eq!(
        clock_state(true, None, 60, 1_700_000_000),
        ClockState::On {
            next_tick_secs: 40,
            custom_format: false
        }
    );
    assert_eq!(
        clock_state(true, Some("HH:mm:ss"), 60, 1_700_000_000),
        ClockState::On {
            next_tick_secs: 40,
            custom_format: true
        }
    );
}

// --- the space-bar alias shortcut ---------------------------------------

#[test]
fn space_runs_the_item_when_the_query_is_its_alias() {
    assert_eq!(
        space_outcome("gh", Some("gh"), false, true, true),
        SpaceOutcome::Activate
    );
}

#[test]
fn the_alias_is_compared_lowercased() {
    // An alias is stored lowercased, and someone typing it with a capital
    // still means it.
    assert_eq!(
        space_outcome("GH", Some("gh"), false, true, true),
        SpaceOutcome::Activate
    );
}

#[test]
fn space_is_just_a_space_when_the_query_is_not_the_alias() {
    assert_eq!(
        space_outcome("git", Some("gh"), false, true, true),
        SpaceOutcome::TypeSpace
    );
    // A prefix is not a match either.
    assert_eq!(
        space_outcome("g", Some("gh"), false, true, true),
        SpaceOutcome::TypeSpace
    );
}

#[test]
fn an_item_with_no_alias_never_triggers_the_shortcut() {
    assert_eq!(
        space_outcome("gh", None, false, true, true),
        SpaceOutcome::TypeSpace
    );
}

#[test]
fn space_over_an_empty_box_does_not_run_an_item_with_no_alias() {
    // This is the case that separates "has no alias" from "has an empty
    // alias". Treating an absent alias as the empty string would make every
    // press of space over an empty search box activate whatever was selected.
    assert_eq!(
        space_outcome("", None, false, true, true),
        SpaceOutcome::TypeSpace
    );
}

#[test]
fn a_command_that_cannot_use_the_shortcut_gets_a_plain_space() {
    // A no-view command does not support it: there would be nothing to show.
    assert_eq!(
        space_outcome("gh", Some("gh"), false, true, false),
        SpaceOutcome::TypeSpace
    );
}

#[test]
fn an_open_completer_takes_the_space_while_its_fields_are_empty() {
    assert_eq!(
        space_outcome("gh", Some("gh"), true, true, true),
        SpaceOutcome::FocusCompleter
    );
}

#[test]
fn once_something_has_been_typed_into_the_completer_space_is_a_space() {
    // Stealing it would make the arguments unwritable.
    assert_eq!(
        space_outcome("gh", Some("gh"), true, false, true),
        SpaceOutcome::TypeSpace
    );
}

#[test]
fn a_completer_overrides_the_items_own_support_either_way() {
    // The completer branch returns before the support check is reached.
    assert_eq!(
        space_outcome("gh", Some("gh"), true, true, false),
        SpaceOutcome::FocusCompleter
    );
}

// --- cycling back through past searches ---------------------------------

#[test]
fn the_up_arrow_reaches_history_from_the_first_row() {
    assert!(up_cycles_history(false, 0, 0));
}

#[test]
fn it_does_not_when_there_is_still_list_above() {
    // The arrow is being used to move up through the list.
    assert!(!up_cycles_history(false, 3, 0));
}

#[test]
fn wrapping_navigation_makes_history_unreachable_by_design() {
    // With wrapping on, the up arrow at the top jumps to the bottom, so it
    // cannot also mean "previous search". This is a conflict, not a
    // preference.
    assert!(!up_cycles_history(true, 0, 0));
}

#[test]
fn the_first_press_reaches_the_most_recent_search() {
    // Offset 0, not 1 — one press should reach the last thing typed.
    assert_eq!(next_history_offset(None), 0);
}

#[test]
fn each_later_press_goes_one_further_back() {
    assert_eq!(next_history_offset(Some(0)), 1);
    assert_eq!(next_history_offset(Some(7)), 8);
}

#[test]
fn history_returns_the_entry_at_the_offset() {
    let h = hist(&["slack", "figma", "terminal"]);
    assert_eq!(history_entry_at(&h, 1, ""), Some((1, "figma".to_owned())));
}

#[test]
fn an_entry_equal_to_what_is_typed_is_skipped() {
    let h = hist(&["slack", "figma"]);
    assert_eq!(
        history_entry_at(&h, 0, "slack"),
        Some((1, "figma".to_owned()))
    );
}

#[test]
fn a_run_of_identical_entries_is_skipped_all_at_once() {
    // The skip is a loop, not a single step. Stopping after one would leave
    // the arrow doing nothing on the second press.
    let h = hist(&["slack", "slack", "slack", "figma"]);
    assert_eq!(
        history_entry_at(&h, 0, "slack"),
        Some((3, "figma".to_owned()))
    );
}

#[test]
fn running_off_the_end_of_history_returns_nothing() {
    // And the box is left as it is, rather than cleared.
    let h = hist(&["slack"]);
    assert_eq!(history_entry_at(&h, 5, ""), None);
    assert_eq!(history_entry_at(&h, 0, "slack"), None);
}

#[test]
fn an_empty_history_returns_nothing() {
    assert_eq!(history_entry_at(&[], 0, ""), None);
}

#[test]
fn typing_starts_again_from_the_newest_entry() {
    assert!(text_change_resets_history(false));
}

#[test]
fn the_view_writing_history_into_the_box_does_not_reset_it() {
    // Otherwise the arrow would never get past the first entry.
    assert!(!text_change_resets_history(true));
}

// --- what running something records --------------------------------------

#[test]
fn the_search_text_is_recorded_whatever_was_run() {
    // A search that ended in a file or a calculation is still reachable by
    // the arrow.
    let (query, visit) = on_action_executed("weather", None);
    assert_eq!(query, "weather");
    assert_eq!(visit, None);
}

#[test]
fn a_visit_is_recorded_only_for_a_root_item() {
    // Only those have a frecency score to raise.
    let (_, visit) = on_action_executed("slack", Some("app.slack"));
    assert_eq!(visit.as_deref(), Some("app.slack"));
}

#[test]
fn an_empty_search_is_still_recorded() {
    // Running the first item with an empty box is a thing people do, and the
    // C++ adds the text unconditionally.
    let (query, _) = on_action_executed("", Some("app.slack"));
    assert_eq!(query, "");
}

// --- the alias form -----------------------------------------------------

#[test]
fn the_alias_form_names_the_item_it_is_for() {
    assert_eq!(alias_form_title("Slack"), "Set alias - Slack");
}

#[test]
fn an_item_with_no_alias_opens_with_an_empty_field() {
    assert_eq!(alias_form_initial_value(None), "");
    assert_eq!(alias_form_initial_value(Some("sl")), "sl");
}

#[test]
fn a_saved_alias_closes_the_form() {
    assert_eq!(
        alias_submit_outcome(true),
        ("Alias modified", "success", true)
    );
}

#[test]
fn a_failed_save_leaves_the_form_open() {
    // The form is the only place the text still exists; popping the view
    // would lose what was typed.
    let (message, style, pops) = alias_submit_outcome(false);
    assert_eq!(message, "Failed to modify alias");
    assert_eq!(style, "danger");
    assert!(!pops);
}

#[test]
fn the_search_history_keeps_one_of_each_newest_first_in_the_cpp_shape() {
    use compass_core::root_view::{MAX_HISTORY_SIZE, SearchHistory};
    let mut history = SearchHistory::default();
    assert!(!history.add("", 1), "an empty search is not kept");
    assert!(history.add("fire", 1));
    assert!(history.add("term", 2));
    assert!(history.add("fire", 3));
    assert_eq!(history.queries(), ["fire", "term"]);

    for n in 0..MAX_HISTORY_SIZE + 5 {
        history.add(&format!("q{n}"), 4);
    }
    assert_eq!(history.entries.len(), MAX_HISTORY_SIZE);
    assert_eq!(history.queries()[0], format!("q{}", MAX_HISTORY_SIZE + 4));

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vicinae").join("search-history.json");
    let mut small = SearchHistory::default();
    small.add("gimp", 1_700_000_000);
    small.save_file(&path).unwrap();
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        written,
        serde_json::json!({"entries": [{"q": "gimp", "ts": 1_700_000_000u64}]})
    );
    assert_eq!(SearchHistory::load_file(&path), small);
    assert_eq!(
        SearchHistory::load_file(&dir.path().join("missing.json")),
        SearchHistory::default()
    );
}
