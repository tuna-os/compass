//! The Raycast extension store's list and detail views.
//!
//! Ported from `src/server/src/builtins/raycast/`.

use compass_core::raycast_store::CompatInfo;
use compass_core::raycast_store_view::{
    CompatTier, FAILED_FETCH_CLEARS_LOADING, INITIAL_NAVIGATION_TITLE, LIST_HEADING,
    SEARCH_DEBOUNCE_MS, SEARCH_HEADING, build_alert, can_populate, compat_tier_from_info,
    detail_actions, detail_navigation_title, extension_load_failure, format_count, has_screenshots,
    list_result_is_current, query_result_is_current, row_actions, row_compat_tier,
    row_compat_value, text_starts_search,
};

fn info(status: &str) -> CompatInfo {
    CompatInfo {
        status: status.to_owned(),
        confidence: "high".to_owned(),
        notes: None,
        has_equivalent: None,
    }
}

// --- reading the compatibility sheet ------------------------------------

#[test]
fn the_three_known_statuses_map_to_their_tiers() {
    assert_eq!(compat_tier_from_info(&info("full")), CompatTier::Compatible);
    assert_eq!(compat_tier_from_info(&info("partial")), CompatTier::Partial);
    assert_eq!(
        compat_tier_from_info(&info("impossible")),
        CompatTier::Incompatible
    );
}

#[test]
fn a_status_nobody_recognises_is_unknown_rather_than_an_error() {
    // The sheet can gain a status later. An unrecognised one says "nobody has
    // checked here" rather than claiming a compatibility nobody asserted.
    assert_eq!(compat_tier_from_info(&info("mostly")), CompatTier::Unknown);
    assert_eq!(compat_tier_from_info(&info("")), CompatTier::Unknown);
}

#[test]
fn the_status_is_matched_exactly() {
    assert_eq!(compat_tier_from_info(&info("Full")), CompatTier::Unknown);
    assert_eq!(compat_tier_from_info(&info("full ")), CompatTier::Unknown);
}

#[test]
fn the_tier_order_matches_the_cpp_enum() {
    // The view model sends the integer, so the order is a wire format.
    assert_eq!(CompatTier::Compatible.as_i32(), 0);
    assert_eq!(CompatTier::Partial.as_i32(), 1);
    assert_eq!(CompatTier::Incompatible.as_i32(), 2);
    assert_eq!(CompatTier::Unknown.as_i32(), 3);
}

// --- the badge on a row -------------------------------------------------

#[test]
fn a_platform_with_no_sheet_shows_no_badge_at_all() {
    // Not the same as an Unknown badge: one says the question does not apply,
    // the other says it was asked and not answered.
    assert_eq!(row_compat_tier(false, Some(&info("full"))), None);
    assert_eq!(row_compat_value(None), -1);
}

#[test]
fn an_extension_the_sheet_does_not_mention_is_unknown_not_absent() {
    assert_eq!(row_compat_tier(true, None), Some(CompatTier::Unknown));
    assert_eq!(row_compat_value(Some(CompatTier::Unknown)), 3);
}

#[test]
fn a_mentioned_extension_shows_its_tier() {
    assert_eq!(
        row_compat_tier(true, Some(&info("partial"))),
        Some(CompatTier::Partial)
    );
}

// --- the detail banner --------------------------------------------------

#[test]
fn a_compatible_extension_gets_a_success_banner() {
    let alert = build_alert(Some(&info("full")));
    assert_eq!(alert.kind, "success");
    assert_eq!(alert.message, "This extension should be fully compatible.");
}

#[test]
fn a_partial_extension_is_warned_about_rather_than_refused() {
    let alert = build_alert(Some(&info("partial")));
    assert_eq!(alert.kind, "warning");
    assert_eq!(alert.message, "This extension works but has a few quirks.");
}

#[test]
fn an_incompatible_extension_gets_a_danger_banner() {
    let alert = build_alert(Some(&info("impossible")));
    assert_eq!(alert.kind, "danger");
    assert_eq!(alert.message, "This extension is not compatible.");
}

#[test]
fn the_two_muted_banners_say_different_things() {
    // An extension the sheet does not mention can be resolved by someone
    // adding a row to the sheet; one the sheet mentions with an unknown status
    // cannot. They look the same and read differently, which is the point.
    let unlisted = build_alert(None);
    let listed_unknown = build_alert(Some(&info("mystery")));
    assert_eq!(unlisted.kind, "muted");
    assert_eq!(listed_unknown.kind, "muted");
    assert_ne!(unlisted.message, listed_unknown.message);
    assert!(unlisted.message.contains("may or may not work"));
}

#[test]
fn the_sheets_notes_are_carried_into_the_banner() {
    let mut i = info("partial");
    i.notes = Some(vec!["Clipboard history is not available".to_owned()]);
    let alert = build_alert(Some(&i));
    assert_eq!(alert.notes, ["Clipboard history is not available"]);
}

#[test]
fn an_entry_with_no_notes_gets_an_empty_list_not_a_missing_one() {
    assert!(build_alert(Some(&info("full"))).notes.is_empty());
    assert!(build_alert(None).notes.is_empty());
}

// --- when the list may be drawn -----------------------------------------

#[test]
fn the_list_waits_for_both_the_page_and_the_sheet() {
    // A rendezvous, not a sequence: whichever finishes second draws the list.
    // Drawing on the page alone would show every row with no badge and then
    // flicker when the sheet landed.
    assert!(!can_populate(false, true));
    assert!(!can_populate(true, false));
    assert!(can_populate(true, true));
}

#[test]
fn a_finished_list_is_dropped_once_something_has_been_typed() {
    // The list is only shown for the empty query, so the test is that the box
    // is still empty.
    assert!(list_result_is_current(""));
    assert!(!list_result_is_current("slack"));
}

#[test]
fn a_finished_search_is_dropped_when_the_query_has_moved_on() {
    assert!(query_result_is_current("sla", "sla"));
    assert!(!query_result_is_current("sla", "slack"));
}

#[test]
fn emptying_the_box_reloads_the_list_without_waiting() {
    // The answer is already cached, and waiting 200ms to show something the
    // user has just come back to would be a pause with nothing behind it.
    assert!(!text_starts_search(""));
    assert!(text_starts_search("s"));
}

#[test]
fn typing_settles_for_two_hundred_milliseconds() {
    assert_eq!(SEARCH_DEBOUNCE_MS, 200);
}

#[test]
fn the_two_lists_are_headed_differently() {
    assert_eq!(LIST_HEADING, "Extensions");
    assert_eq!(SEARCH_HEADING, "Results");
}

#[test]
fn a_failed_fetch_leaves_the_spinner_running() {
    // Both handlers report the failure and return before clearing the loading
    // state. Ported as it is: it is visible behaviour, and changing it here
    // would make the two implementations disagree while the C++ still ships.
    const { assert!(!FAILED_FETCH_CLEARS_LOADING) };
}

// --- the detail page ----------------------------------------------------

#[test]
fn a_failure_to_load_quotes_the_identifier_back() {
    // This view is reachable from a deep link where the user never saw a
    // list, so the name they typed is the only context they have.
    let (title, body) = extension_load_failure("alice", "slack");
    assert_eq!(title, "Failed to load extension");
    assert!(body.contains("\"alice/slack\""));
    assert!(body.contains("may not exist"));
}

#[test]
fn the_detail_page_is_titled_after_the_extension() {
    assert_eq!(
        detail_navigation_title("Slack Status"),
        "Extension Store - Slack Status"
    );
}

#[test]
fn the_title_before_anything_loads_names_the_store() {
    assert_eq!(INITIAL_NAVIGATION_TITLE, "Extension Store");
}

#[test]
fn a_rows_destructive_action_sits_behind_a_separator() {
    assert_eq!(row_actions(), [vec!["show-details"], vec!["uninstall"]]);
}

#[test]
fn installing_and_uninstalling_are_never_both_offered() {
    // An extension is one or the other.
    assert_eq!(detail_actions(false), ["install", "report-issue"]);
    assert_eq!(detail_actions(true), ["uninstall", "report-issue"]);
}

#[test]
fn reporting_an_issue_is_always_offered() {
    // An extension that will not install is exactly the one worth reporting.
    for installed in [true, false] {
        assert!(detail_actions(installed).contains(&"report-issue"));
    }
}

#[test]
fn screenshots_are_decided_by_the_listings_count() {
    // The URLs are derived, so the count is what says whether deriving them is
    // worth doing.
    assert!(!has_screenshots(0));
    assert!(has_screenshots(1));
}

// --- the download count -------------------------------------------------

#[test]
fn a_small_count_is_printed_as_it_is() {
    assert_eq!(format_count(0), "0");
    assert_eq!(format_count(999), "999");
}

#[test]
fn both_thresholds_are_strict() {
    // Exactly a thousand prints as a thousand; 1,001 is the first to abbreviate.
    assert_eq!(format_count(1_000), "1000");
    assert_eq!(format_count(1_001), "1.1K");
    assert_eq!(format_count(1_000_000), "1000K");
    assert_eq!(format_count(1_000_001), "1.1M");
}

#[test]
fn the_figure_is_rounded_up_rather_than_to_nearest() {
    // 1,001 downloads reads as 1.1K. That is generous, and it is what the C++
    // does — `ceil(count / 1000 * 10) / 10`.
    assert_eq!(format_count(1_001), "1.1K");
    assert_eq!(format_count(1_040), "1.1K");
    assert_eq!(format_count(1_100), "1.1K");
    assert_eq!(format_count(1_101), "1.2K");
}

#[test]
fn a_round_figure_drops_its_trailing_zero() {
    // Qt prints a float, so 2.0 comes out as "2" and not "2.0".
    assert_eq!(format_count(2_000), "2K");
    assert_eq!(format_count(1_500), "1.5K");
}
