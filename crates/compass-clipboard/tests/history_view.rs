//! The clipboard history command's own decisions.
//!
//! Ported from `src/server/src/builtins/clipboard/`.

use compass_clipboard::history_view::{
    DefaultAction, EntryIcon, FILTER_STORAGE_KEY, FILTER_STORED_VALUES, FlightAction,
    KIND_FILTER_OPTIONS, QueryFlight, delivery_is_incremental, entry_icon, file_open_actions,
    filter_index_for_kind, is_copyable, kind_for_stored_value, link_open_actions, main_actions,
    pin_action_pins, restored_filter, stored_value_for_index, trailing_sections,
};
use compass_clipboard::kind::{EncryptionType, OfferKind};

// --- which action the return key runs -----------------------------------

#[test]
fn only_the_exact_word_paste_selects_pasting() {
    assert_eq!(DefaultAction::parse(Some("paste")), DefaultAction::Paste);
}

#[test]
fn anything_else_falls_back_to_copying() {
    // The safer of the two to get wrong: copying into the wrong window does
    // nothing, pasting into it does not.
    assert_eq!(DefaultAction::parse(None), DefaultAction::Copy);
    assert_eq!(DefaultAction::parse(Some("")), DefaultAction::Copy);
    assert_eq!(DefaultAction::parse(Some("copy")), DefaultAction::Copy);
    assert_eq!(DefaultAction::parse(Some("Paste")), DefaultAction::Copy);
}

// --- the icon on a row --------------------------------------------------

#[test]
fn each_kind_has_its_own_glyph() {
    assert_eq!(
        entry_icon(OfferKind::Image, None),
        EntryIcon::Builtin("image")
    );
    assert_eq!(
        entry_icon(OfferKind::Text, None),
        EntryIcon::Builtin("text")
    );
    assert_eq!(
        entry_icon(OfferKind::File, None),
        EntryIcon::Builtin("folder")
    );
}

#[test]
fn a_link_with_a_known_host_shows_the_sites_own_icon() {
    // A list of links is then scannable by site rather than being a column of
    // identical chain glyphs.
    assert_eq!(
        entry_icon(OfferKind::Link, Some("example.com")),
        EntryIcon::Favicon {
            host: "example.com".to_owned(),
            fallback: "link"
        }
    );
}

#[test]
fn a_favicon_keeps_the_generic_glyph_behind_it() {
    // The site may be unreachable, or may have no icon.
    let EntryIcon::Favicon { fallback, .. } = entry_icon(OfferKind::Link, Some("example.com"))
    else {
        panic!("expected a favicon");
    };
    assert_eq!(fallback, "link");
}

#[test]
fn a_link_with_no_host_falls_back_to_the_glyph_directly() {
    assert_eq!(
        entry_icon(OfferKind::Link, None),
        EntryIcon::Builtin("link")
    );
}

#[test]
fn an_unclassified_row_shows_a_question_mark_rather_than_nothing() {
    // A row written by a newer build still occupies a visible line.
    assert_eq!(
        entry_icon(OfferKind::Unknown, None),
        EntryIcon::Builtin("question-mark-circle")
    );
    assert_eq!(
        entry_icon(OfferKind::Count, None),
        EntryIcon::Builtin("question-mark-circle")
    );
}

// --- what can be copied -------------------------------------------------

#[test]
fn an_unencrypted_entry_is_always_copyable() {
    assert!(is_copyable(EncryptionType::None, false));
}

#[test]
fn an_encrypted_entry_needs_the_key_to_be_available() {
    assert!(!is_copyable(EncryptionType::Local, false));
    assert!(is_copyable(EncryptionType::Local, true));
}

#[test]
fn an_entry_that_cannot_be_copied_offers_a_way_into_the_settings() {
    // Rather than a copy action that would fail.
    assert_eq!(
        main_actions(false, true, DefaultAction::Copy),
        ["open-settings"]
    );
}

#[test]
fn neither_copy_nor_paste_is_offered_for_an_unreadable_entry() {
    let actions = main_actions(false, true, DefaultAction::Paste);
    assert!(!actions.contains(&"copy"));
    assert!(!actions.contains(&"paste"));
}

#[test]
fn both_are_offered_where_pasting_works() {
    assert_eq!(
        main_actions(true, true, DefaultAction::Copy),
        ["copy", "paste"]
    );
}

#[test]
fn the_preference_decides_which_of_them_the_return_key_runs() {
    // Both are present either way; only the order changes.
    assert_eq!(
        main_actions(true, true, DefaultAction::Paste),
        ["paste", "copy"]
    );
}

#[test]
fn a_platform_that_cannot_paste_is_not_offered_a_paste_that_does_nothing() {
    assert_eq!(main_actions(true, false, DefaultAction::Paste), ["copy"]);
    assert_eq!(main_actions(true, false, DefaultAction::Copy), ["copy"]);
}

// --- opening a file or a link -------------------------------------------

/// Accepts only the exact paths the fixtures name.
///
/// A stub that accepted anything would let a mangled path through: splitting
/// the URI list on the wrong separator leaves a stray carriage return on the
/// end, and only an existence check that looks at the text can notice.
fn always(path: &str) -> bool {
    [
        "file:///tmp/a.txt",
        "file:///tmp/b.txt",
        "file:///tmp/a.bin",
    ]
    .contains(&path)
}

fn never(_: &str) -> bool {
    false
}

#[test]
fn one_existing_file_can_be_opened() {
    assert_eq!(
        file_open_actions("file:///tmp/a.txt", always, true),
        ["open", "open-with"]
    );
}

#[test]
fn a_copy_of_several_files_has_no_single_thing_to_open() {
    let two = "file:///tmp/a.txt\r\nfile:///tmp/b.txt";
    assert!(file_open_actions(two, always, true).is_empty());
}

#[test]
fn a_trailing_separator_does_not_make_it_two_files() {
    // The split skips empty parts, so a payload ending in CRLF is still one
    // file rather than a file and nothing.
    assert_eq!(
        file_open_actions("file:///tmp/a.txt\r\n", always, true),
        ["open", "open-with"]
    );
}

#[test]
fn a_file_that_has_since_been_deleted_offers_nothing() {
    // Offering to open it would offer to open nothing.
    assert!(file_open_actions("file:///tmp/a.txt", never, true).is_empty());
}

#[test]
fn a_file_type_nothing_claims_still_gets_a_chooser() {
    assert_eq!(
        file_open_actions("file:///tmp/a.bin", always, false),
        ["open-with"]
    );
}

#[test]
fn a_link_needs_no_existence_check() {
    // The payload *is* the target.
    assert_eq!(link_open_actions(true), ["open", "open-with"]);
    assert_eq!(link_open_actions(false), ["open-with"]);
}

// --- the rest of the panel ----------------------------------------------

#[test]
fn reversible_and_irreversible_actions_are_separated() {
    // Pinning and renaming are changes to an entry; removing is not.
    assert_eq!(
        trailing_sections(),
        [vec!["pin", "edit-keywords"], vec!["remove", "remove-all"]]
    );
}

#[test]
fn an_unpinned_entry_offers_to_pin_and_a_pinned_one_to_unpin() {
    assert!(pin_action_pins(None));
    assert!(!pin_action_pins(Some(1_700_000_000)));
}

// --- the kind filter ----------------------------------------------------

#[test]
fn the_filter_has_five_options_the_first_of_which_is_not_a_kind() {
    assert_eq!(
        KIND_FILTER_OPTIONS,
        ["All", "Text", "Images", "Links", "Files"]
    );
    assert_eq!(filter_index_for_kind(None), 0);
}

#[test]
fn the_stored_vocabulary_is_the_enums_and_not_the_interfaces() {
    // The third option reads `Images` and stores `image`. Keeping them apart
    // is what lets either change without the other.
    assert_eq!(
        FILTER_STORED_VALUES,
        ["all", "text", "image", "link", "file"]
    );
    assert_eq!(KIND_FILTER_OPTIONS[2], "Images");
    assert_eq!(stored_value_for_index(2), "image");
    assert_eq!(FILTER_STORAGE_KEY, "filter");
}

#[test]
fn each_stored_value_selects_its_kind() {
    assert_eq!(kind_for_stored_value("text"), Some(OfferKind::Text));
    assert_eq!(kind_for_stored_value("image"), Some(OfferKind::Image));
    assert_eq!(kind_for_stored_value("link"), Some(OfferKind::Link));
    assert_eq!(kind_for_stored_value("file"), Some(OfferKind::File));
}

#[test]
fn all_is_not_a_kind() {
    assert_eq!(kind_for_stored_value("all"), None);
}

#[test]
fn each_kind_sits_at_its_own_index() {
    assert_eq!(filter_index_for_kind(Some(OfferKind::Text)), 1);
    assert_eq!(filter_index_for_kind(Some(OfferKind::Image)), 2);
    assert_eq!(filter_index_for_kind(Some(OfferKind::Link)), 3);
    assert_eq!(filter_index_for_kind(Some(OfferKind::File)), 4);
}

#[test]
fn the_kinds_with_no_option_answer_zero_rather_than_out_of_range() {
    // Answering with an index past the end would select nothing at all.
    assert_eq!(filter_index_for_kind(Some(OfferKind::Unknown)), 0);
    assert_eq!(filter_index_for_kind(Some(OfferKind::Count)), 0);
}

#[test]
fn an_index_past_the_end_stores_all() {
    assert_eq!(stored_value_for_index(9), "all");
}

#[test]
fn nothing_stored_restores_everything() {
    assert_eq!(restored_filter(None), (0, None));
}

#[test]
fn an_unrecognised_stored_value_restores_everything_too() {
    // Neither is a reason to show an empty list.
    assert_eq!(restored_filter(Some("sandwiches")), (0, None));
}

#[test]
fn a_stored_filter_restores_its_index_and_its_kind_together() {
    assert_eq!(restored_filter(Some("link")), (3, Some(OfferKind::Link)));
}

// --- the query controller -----------------------------------------------

#[test]
fn the_first_request_runs_at_once() {
    let (state, action) = QueryFlight::default().request();
    assert_eq!(action, FlightAction::Run);
    assert!(state.running);
    assert!(!state.pending);
}

#[test]
fn a_request_while_one_runs_waits() {
    let (state, _) = QueryFlight::default().request();
    let (state, action) = state.request();
    assert_eq!(action, FlightAction::Queue);
    assert!(state.pending);
}

#[test]
fn a_third_request_collapses_into_the_same_waiting_slot() {
    // Only the newest matters, so they do not queue up.
    let (state, _) = QueryFlight::default().request();
    let (state, _) = state.request();
    let (state, action) = state.request();
    assert_eq!(action, FlightAction::Queue);
    assert_eq!(
        state,
        QueryFlight {
            running: true,
            pending: true
        }
    );
}

#[test]
fn a_finished_query_with_nothing_waiting_is_delivered() {
    let (state, _) = QueryFlight::default().request();
    let (state, action) = state.finish();
    assert_eq!(action, FlightAction::Deliver);
    assert_eq!(state, QueryFlight::default());
}

#[test]
fn a_finished_query_with_something_waiting_is_dropped_unseen() {
    // Each delivery consumes the view's one-shot "select the first row" flag,
    // so delivering a result that is already out of date would move the
    // selection out from under the user.
    let (state, _) = QueryFlight::default().request();
    let (state, _) = state.request();
    let (state, action) = state.finish();
    assert_eq!(action, FlightAction::Run);
    assert_eq!(
        state,
        QueryFlight {
            running: true,
            pending: false
        }
    );
}

#[test]
fn the_replacement_query_is_itself_delivered_when_nothing_follows_it() {
    let (state, _) = QueryFlight::default().request();
    let (state, _) = state.request();
    let (state, _) = state.finish();
    let (_, action) = state.finish();
    assert_eq!(action, FlightAction::Deliver);
}

#[test]
fn the_first_delivery_after_a_reset_moves_the_selection_to_the_top() {
    assert!(!delivery_is_incremental(true));
}

#[test]
fn every_later_delivery_leaves_the_selection_where_it_is() {
    // A pin, a rename, or a new copy arriving while the user reads.
    assert!(delivery_is_incremental(false));
}
