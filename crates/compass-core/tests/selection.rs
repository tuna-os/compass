//! Where the selected text comes from, and what is said when there is none.
//!
//! Read off `LinuxSelectionService` and `DummySelectionService`
//! (`src/server/src/services/selection/`).

use compass_core::selection::{
    DummySelectionService, LinuxSelectionService, NO_SELECTION_ERROR, UNSUPPORTED_ERROR,
};

/// A fallback that must not be consulted.
fn unused_fallback() -> String {
    panic!("the fallback must not be asked when the accurate source has text")
}

#[test]
fn the_pushed_primary_selection_is_preferred() {
    // Reading Qt first would give a stale answer whenever the accurate source
    // works, which on a compositor that grants data-control is always.
    let mut service = LinuxSelectionService::new();
    service.set_primary_text("from data-control");

    assert_eq!(
        service.selected_text(unused_fallback),
        Ok("from data-control".to_owned())
    );
}

#[test]
fn qt_is_asked_only_when_the_accurate_source_is_empty() {
    let service = LinuxSelectionService::new();
    assert_eq!(
        service.selected_text(|| "from qt".to_owned()),
        Ok("from qt".to_owned())
    );
}

#[test]
fn an_empty_selection_from_both_sources_is_an_error_not_an_empty_string() {
    // An extension handed "" would render an empty detail view and look
    // broken; an error it can branch on is the usable answer.
    let service = LinuxSelectionService::new();
    assert_eq!(
        service.selected_text(String::new),
        Err(NO_SELECTION_ERROR.to_owned())
    );
}

#[test]
fn a_cleared_selection_does_not_leave_the_old_text_behind() {
    // primarySelectionChanged fires with "" when a selection is dropped.
    // Keeping the previous text would hand an extension something the person
    // deselected.
    let mut service = LinuxSelectionService::new();
    service.set_primary_text("first");
    service.set_primary_text("");

    assert_eq!(
        service.selected_text(String::new),
        Err(NO_SELECTION_ERROR.to_owned())
    );
}

#[test]
fn a_cleared_selection_still_falls_back_to_qt() {
    let mut service = LinuxSelectionService::new();
    service.set_primary_text("first");
    service.set_primary_text("");

    assert_eq!(
        service.selected_text(|| "from qt".to_owned()),
        Ok("from qt".to_owned())
    );
}

#[test]
fn the_latest_pushed_selection_wins() {
    let mut service = LinuxSelectionService::new();
    service.set_primary_text("first");
    service.set_primary_text("second");
    assert_eq!(
        service.selected_text(unused_fallback),
        Ok("second".to_owned())
    );
}

#[test]
fn whitespace_is_a_selection() {
    // Only emptiness triggers the fallback. A person who selected a blank line
    // selected something.
    let mut service = LinuxSelectionService::new();
    service.set_primary_text("   ");
    assert_eq!(service.selected_text(unused_fallback), Ok("   ".to_owned()));
}

#[test]
fn the_unsupported_message_is_not_the_no_selection_one() {
    // An extension author reading "unable to get selected text" looks for a
    // selection; reading the other knows to stop.
    assert_eq!(
        DummySelectionService.selected_text(),
        Err(UNSUPPORTED_ERROR.to_owned())
    );
    assert_ne!(UNSUPPORTED_ERROR, NO_SELECTION_ERROR);
    assert_eq!(NO_SELECTION_ERROR, "Unable to get selected text");
    assert_eq!(
        UNSUPPORTED_ERROR,
        "Selected text is not supported on this platform"
    );
}
