//! The Search Files command.
//!
//! Ported from `src/server/src/builtins/file/`.

use compass_core::file_category::FileCategory;
use compass_core::file_search::{
    CATEGORY_FILTER_KEYS, CATEGORY_STORAGE_KEY, DIRECT_PATH_HEADING, QueryPlan, RECENT_FILES_LIMIT,
    RECENT_HEADING, ResultMode, SEARCH_IS_FALLBACK, accepts_filter_change, category_for_index,
    empty_query_mode, is_explicit_path_query, is_explicit_path_query_windows, plan_query,
    recent_files_usable, recent_result_is_current, restored_filter_index, result_is_current,
    results_heading, should_debounce, stored_key_for_index,
};

// --- when a query is a path ---------------------------------------------

#[test]
fn an_absolute_path_is_a_path() {
    assert!(is_explicit_path_query("/etc/hosts"));
    assert!(is_explicit_path_query("/"));
}

#[test]
fn the_home_shorthand_counts_alone_and_with_a_separator() {
    assert!(is_explicit_path_query("~"));
    assert!(is_explicit_path_query("~/Documents"));
}

#[test]
fn the_home_shorthand_needs_its_separator_to_take_a_suffix() {
    // `~notes` stays a search rather than becoming a home-relative path that
    // does not exist.
    assert!(!is_explicit_path_query("~notes"));
}

#[test]
fn the_relative_shorthands_count_alone_and_with_a_separator() {
    assert!(is_explicit_path_query("."));
    assert!(is_explicit_path_query(".."));
    assert!(is_explicit_path_query("./src"));
    assert!(is_explicit_path_query("../src"));
}

#[test]
fn a_dot_inside_a_name_is_not_a_path() {
    // A search for `..` is not a thing; a search for a file called
    // `notes..txt` is.
    assert!(!is_explicit_path_query("notes..txt"));
    assert!(!is_explicit_path_query("...."));
}

#[test]
fn an_ordinary_word_is_not_a_path() {
    assert!(!is_explicit_path_query("report"));
    assert!(!is_explicit_path_query("src/main.rs"));
}

#[test]
fn nothing_is_not_a_path() {
    assert!(!is_explicit_path_query(""));
}

#[test]
fn a_backslash_anywhere_makes_it_a_path_on_windows() {
    // Not only at the front, unlike the forward-slash rule.
    assert!(is_explicit_path_query_windows("src\\main.rs"));
    assert!(!is_explicit_path_query("src\\main.rs"));
}

#[test]
fn a_drive_letter_counts_with_either_separator() {
    assert!(is_explicit_path_query_windows("C:/Users"));
    assert!(is_explicit_path_query_windows("C:\\Users"));
}

#[test]
fn a_bare_drive_letter_is_not_enough() {
    assert!(!is_explicit_path_query_windows("C:"));
}

// --- what a query does --------------------------------------------------

fn plan(text: &str, expanded: &str, exists: bool) -> QueryPlan {
    plan_query(
        text,
        expanded,
        exists,
        is_explicit_path_query(text),
        None,
        None,
    )
}

#[test]
fn an_empty_query_is_its_own_case() {
    assert_eq!(plan("", "", false), QueryPlan::EmptyQuery);
}

#[test]
fn whitespace_alone_is_an_empty_query() {
    assert_eq!(plan("   ", "", false), QueryPlan::EmptyQuery);
}

#[test]
fn a_path_that_exists_is_shown_directly() {
    assert_eq!(
        plan("/etc/hosts", "/etc/hosts", true),
        QueryPlan::DirectPath("/etc/hosts".to_owned())
    );
}

#[test]
fn a_path_that_does_not_exist_is_searched_for_instead() {
    assert_eq!(plan("/etc/nope", "/etc/nope", false), QueryPlan::Search);
}

#[test]
fn a_single_slash_is_searched_for_rather_than_opened() {
    // The root exists on every machine. Matching it would turn one slash into
    // a one-item list instead of a search for names containing a slash.
    assert_eq!(plan("/", "/", true), QueryPlan::Search);
}

#[test]
fn a_word_is_searched_for_even_when_a_file_of_that_name_exists() {
    // The path test comes first and it is about the *shape* of the text, not
    // about what happens to be on disk.
    assert_eq!(plan("report", "report", true), QueryPlan::Search);
}

#[test]
fn a_named_file_of_the_wrong_kind_gives_an_empty_direct_section() {
    // The user asked for that file. Answering with a list of other files
    // would be a different question, so the section is empty and still
    // headed "Direct file path".
    let plan = plan_query(
        "/etc/hosts",
        "/etc/hosts",
        true,
        true,
        Some(FileCategory::Document),
        Some(FileCategory::Image),
    );
    assert_eq!(plan, QueryPlan::DirectPathFiltered);
}

#[test]
fn a_named_file_of_the_right_kind_passes_the_filter() {
    let plan = plan_query(
        "/p.png",
        "/p.png",
        true,
        true,
        Some(FileCategory::Image),
        Some(FileCategory::Image),
    );
    assert_eq!(plan, QueryPlan::DirectPath("/p.png".to_owned()));
}

#[test]
fn with_no_filter_any_kind_of_file_passes() {
    let plan = plan_query(
        "/p.png",
        "/p.png",
        true,
        true,
        Some(FileCategory::Image),
        None,
    );
    assert_eq!(plan, QueryPlan::DirectPath("/p.png".to_owned()));
}

// --- the empty query ----------------------------------------------------

#[test]
fn the_empty_query_shows_recent_files_where_they_are_tracked() {
    assert_eq!(empty_query_mode(true), ResultMode::Recent);
}

#[test]
fn a_machine_with_no_recent_files_record_searches_the_index_instead() {
    // Rather than being shown an empty list.
    assert_eq!(empty_query_mode(false), ResultMode::IndexedSearch);
}

#[test]
fn an_empty_batch_of_recent_files_falls_through_to_the_index() {
    // A system that tracks recent files but has none yet still shows
    // something.
    assert!(!recent_files_usable(0));
    assert!(recent_files_usable(1));
}

#[test]
fn the_empty_query_asks_for_fifty_recent_files() {
    assert_eq!(RECENT_FILES_LIMIT, 50);
}

// --- discarding a late answer -------------------------------------------

#[test]
fn a_result_for_the_current_query_is_shown() {
    assert!(result_is_current(
        false,
        ResultMode::IndexedSearch,
        ResultMode::IndexedSearch,
        "rep",
        "rep"
    ));
}

#[test]
fn a_cancelled_result_is_dropped() {
    assert!(!result_is_current(
        true,
        ResultMode::IndexedSearch,
        ResultMode::IndexedSearch,
        "rep",
        "rep"
    ));
}

#[test]
fn a_result_arriving_after_the_view_changed_mode_is_dropped() {
    // A direct path typed while a search was still in flight.
    assert!(!result_is_current(
        false,
        ResultMode::DirectPath,
        ResultMode::IndexedSearch,
        "rep",
        "rep"
    ));
}

#[test]
fn a_result_for_a_query_that_has_moved_on_is_dropped() {
    // This is the one that matters most: without it, answers arriving out of
    // order leave the list showing a question the user has finished asking.
    assert!(!result_is_current(
        false,
        ResultMode::IndexedSearch,
        ResultMode::IndexedSearch,
        "rep",
        "report"
    ));
}

#[test]
fn recent_files_are_checked_against_an_empty_box_not_a_remembered_query() {
    // They are only ever requested for the empty query, so the test is that
    // the box is *still* empty.
    assert!(recent_result_is_current(false, ResultMode::Recent, ""));
    assert!(!recent_result_is_current(false, ResultMode::Recent, "rep"));
    assert!(!recent_result_is_current(true, ResultMode::Recent, ""));
    assert!(!recent_result_is_current(
        false,
        ResultMode::IndexedSearch,
        ""
    ));
}

// --- headings -----------------------------------------------------------

#[test]
fn a_search_with_a_query_is_headed_results() {
    assert_eq!(results_heading("rep"), "Results");
}

#[test]
fn a_search_with_no_query_is_headed_by_what_it_actually_shows() {
    // An empty query reaches the index sorted by modification time, so the
    // heading says that rather than "Results".
    assert_eq!(results_heading(""), "Recently Modified");
}

#[test]
fn the_other_two_headings_say_where_their_rows_came_from() {
    assert_eq!(RECENT_HEADING, "Recently Accessed");
    assert_eq!(DIRECT_PATH_HEADING, "Direct file path");
}

// --- the category filter ------------------------------------------------

#[test]
fn the_first_option_is_not_a_category() {
    assert_eq!(CATEGORY_FILTER_KEYS[0], "All");
    assert_eq!(category_for_index(0), None);
}

#[test]
fn each_remaining_option_selects_its_category() {
    assert_eq!(category_for_index(1), Some(FileCategory::Other));
    assert_eq!(category_for_index(2), Some(FileCategory::Directory));
    assert_eq!(category_for_index(3), Some(FileCategory::Image));
    assert_eq!(category_for_index(4), Some(FileCategory::Video));
    assert_eq!(category_for_index(5), Some(FileCategory::Audio));
    assert_eq!(category_for_index(6), Some(FileCategory::Document));
    assert_eq!(category_for_index(7), Some(FileCategory::Archive));
    assert_eq!(category_for_index(8), Some(FileCategory::Application));
}

#[test]
fn an_index_past_the_end_shows_everything_rather_than_nothing() {
    // A stored index from a future version's longer list.
    assert_eq!(category_for_index(9), None);
    assert_eq!(category_for_index(99), None);
}

#[test]
fn a_filter_change_within_range_is_accepted() {
    assert!(accepts_filter_change(3, 0));
}

#[test]
fn reselecting_the_current_filter_does_nothing() {
    // It would otherwise re-run the search for no reason.
    assert!(!accepts_filter_change(3, 3));
}

#[test]
fn a_filter_index_outside_the_list_is_refused() {
    assert!(!accepts_filter_change(-1, 0));
    assert!(!accepts_filter_change(9, 0));
}

#[test]
fn the_untranslated_key_is_what_gets_stored() {
    // Storing the visible label would make a filter chosen in one language
    // unreadable in another.
    assert_eq!(stored_key_for_index(3), "Images");
    assert_eq!(CATEGORY_STORAGE_KEY, "fileCategory");
}

#[test]
fn a_stored_filter_is_restored() {
    assert_eq!(restored_filter_index(Some("Images")), Some(3));
}

#[test]
fn nothing_stored_restores_nothing() {
    assert_eq!(restored_filter_index(None), None);
}

#[test]
fn a_stored_all_restores_nothing() {
    // `All` is already the default, so writing it back would be a no-op with a
    // change signal attached to it.
    assert_eq!(restored_filter_index(Some("All")), None);
}

#[test]
fn an_unrecognised_stored_value_restores_nothing() {
    // The C++ folds this into the same `index <= 0` branch as `All`, because
    // not-found is -1.
    assert_eq!(restored_filter_index(Some("Sandwiches")), None);
}

// --- the debounce and the command list ----------------------------------

#[test]
fn a_zero_debounce_runs_the_search_at_once() {
    // Rather than starting a timer that fires on the next tick, which is what
    // makes a fast local index feel like typing into a list.
    assert!(!should_debounce(0));
    assert!(should_debounce(1));
}

#[test]
fn searching_files_answers_queries_no_command_claimed() {
    const { assert!(SEARCH_IS_FALLBACK) };
}

#[test]
fn rebuilding_the_index_is_written_but_not_offered() {
    // The indexer sweeps on a timer now, and the C++ leaves the command in
    // place with a comment saying that deleting the cache directory has the
    // same effect.
    assert_eq!(compass_core::file_search::registered_commands(), ["search"]);
}
