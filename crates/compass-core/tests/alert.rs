//! What the confirmation dialog defaults to, and what answer each exit gives.
//!
//! Read off `AlertWidget` and `AlertModel` (`src/server/src/ui/alert/`) and
//! `ExtUIService::confirmAlert` (`src/server/src/extension/api/ui-service.hpp`).

use compass_core::alert::{
    Alert, AlertIcon, AlertModel, DEFAULT_CANCEL_TEXT, DEFAULT_CONFIRM_TEXT, DEFAULT_MESSAGE,
    DEFAULT_TITLE, Outcome, SemanticColor,
};

#[test]
fn the_defaults_are_the_cpp_members_verbatim() {
    // Read off AlertWidget's member initialisers. These are what a caller who
    // sets nothing shows a person before something irreversible happens.
    let alert = Alert::new();
    assert_eq!(alert.title, "Are you sure?");
    assert_eq!(alert.message, "This action cannot be undone");
    assert_eq!(alert.confirm_text, "Confirm");
    assert_eq!(alert.cancel_text, "Cancel");
    assert_eq!(DEFAULT_TITLE, "Are you sure?");
    assert_eq!(DEFAULT_MESSAGE, "This action cannot be undone");
    assert_eq!(DEFAULT_CONFIRM_TEXT, "Confirm");
    assert_eq!(DEFAULT_CANCEL_TEXT, "Cancel");
}

#[test]
fn the_confirm_button_is_red_and_the_cancel_button_is_not() {
    // The whole visual point of the dialog: the dangerous button looks
    // dangerous. A confirm button in the foreground colour would make
    // "delete everything" look like "OK".
    let alert = Alert::new();
    assert_eq!(alert.confirm_color, SemanticColor::RED);
    assert_eq!(alert.cancel_color, SemanticColor::FOREGROUND);
    assert_ne!(alert.confirm_color, alert.cancel_color);
}

#[test]
fn the_default_icon_is_a_red_warning() {
    let icon = Alert::new().icon.expect("there is a default icon");
    assert_eq!(icon.source, "warning");
    assert!(icon.builtin);
    assert_eq!(icon.fill, Some(SemanticColor::RED));
}

#[test]
fn a_builtin_icon_from_a_payload_is_tinted_red() {
    let icon = AlertIcon::from_payload("trash", true);
    assert_eq!(icon.fill, Some(SemanticColor::RED));
}

#[test]
fn an_icon_that_is_not_builtin_is_left_alone() {
    // confirmAlert only calls setFill when isBuiltin(). Tinting a photograph
    // red is not a highlight.
    let icon = AlertIcon::from_payload("file:///home/ada/photo.png", false);
    assert_eq!(icon.fill, None);
}

#[test]
fn an_alert_can_have_no_icon_at_all() {
    let alert = Alert::new().with_icon(None);
    assert_eq!(alert.icon, None);
}

#[test]
fn nothing_is_showing_to_begin_with() {
    let model = AlertModel::new();
    assert!(!model.is_visible());
    assert_eq!(model.current(), None);
}

#[test]
fn showing_an_alert_makes_it_visible() {
    let mut model = AlertModel::new();
    let replaced = model.show(Alert::new().with_title("Delete?"));
    assert_eq!(
        replaced, None,
        "nothing was showing, so nothing was replaced"
    );
    assert!(model.is_visible());
    assert_eq!(model.current().expect("showing").title, "Delete?");
}

#[test]
fn confirming_is_the_only_outcome_that_answers_true() {
    assert!(Outcome::Confirmed.confirmed());
    assert!(!Outcome::Cancelled.confirmed());
    assert!(!Outcome::Dismissed.confirmed());
    assert!(!Outcome::Replaced.confirmed());
}

#[test]
fn confirming_resolves_and_hides() {
    let mut model = AlertModel::new();
    model.show(Alert::new().with_title("Delete?"));

    let resolution = model.confirm().expect("resolves");
    assert_eq!(resolution.outcome, Outcome::Confirmed);
    assert_eq!(resolution.alert.title, "Delete?");
    assert!(!model.is_visible());
}

#[test]
fn cancelling_resolves_false_and_hides() {
    let mut model = AlertModel::new();
    model.show(Alert::new());

    let resolution = model.cancel().expect("resolves");
    assert!(!resolution.outcome.confirmed());
    assert!(!model.is_visible());
}

#[test]
fn navigating_away_cancels_the_alert() {
    // AlertModel's constructor connects currentViewChanged to dismiss(), which
    // calls triggerCancel. Leaving the view is an answer, and the answer is no.
    let mut model = AlertModel::new();
    model.show(Alert::new());

    let resolution = model.view_changed().expect("resolves");
    assert_eq!(resolution.outcome, Outcome::Dismissed);
    assert!(!resolution.outcome.confirmed());
    assert!(!model.is_visible());
}

#[test]
fn a_second_alert_cancels_the_first_and_reports_it() {
    // The TypeScript API documents this: "Calling this function when another
    // alert is currently pending will result in the pending alert to be
    // automatically canceled". The caller must get that resolution, or the
    // first extension's promise never settles.
    let mut model = AlertModel::new();
    model.show(Alert::new().with_title("First"));

    let replaced = model
        .show(Alert::new().with_title("Second"))
        .expect("reports the first");
    assert_eq!(replaced.alert.title, "First");
    assert_eq!(replaced.outcome, Outcome::Replaced);
    assert!(!replaced.outcome.confirmed());

    assert_eq!(model.current().expect("showing").title, "Second");
}

#[test]
fn confirming_twice_answers_once() {
    // The C++ nulls m_widget before triggering, so the second press finds
    // nothing. Answering twice would resolve an already-settled promise and,
    // worse, could run the destructive action twice.
    let mut model = AlertModel::new();
    model.show(Alert::new());

    assert!(model.confirm().is_some());
    assert_eq!(model.confirm(), None, "the second press must do nothing");
}

#[test]
fn cancelling_after_confirming_does_not_answer_again() {
    let mut model = AlertModel::new();
    model.show(Alert::new());

    assert!(model.confirm().is_some());
    assert_eq!(model.cancel(), None);
    assert_eq!(model.view_changed(), None);
}

#[test]
fn every_exit_on_an_empty_model_is_a_no_op() {
    let mut model = AlertModel::new();
    assert_eq!(model.confirm(), None);
    assert_eq!(model.cancel(), None);
    assert_eq!(model.view_changed(), None);
    assert!(!model.is_visible());
}

#[test]
fn the_resolution_carries_the_alert_that_ended_not_the_one_showing() {
    // With two alerts in play the caller has to know which promise to settle.
    let mut model = AlertModel::new();
    model.show(Alert::new().with_title("First"));
    model.show(Alert::new().with_title("Second"));

    let resolution = model.confirm().expect("resolves");
    assert_eq!(resolution.alert.title, "Second");
}

#[test]
fn the_extension_payload_sets_both_button_texts_and_their_colours() {
    // confirmAlert passes Red for the primary action and Foreground for the
    // dismiss action regardless of the style the extension asked for.
    let alert = Alert::new()
        .with_title("Revoke token?")
        .with_message("You will have to sign in again")
        .with_confirm("Revoke", SemanticColor::RED)
        .with_cancel("Keep it", SemanticColor::FOREGROUND);

    assert_eq!(alert.title, "Revoke token?");
    assert_eq!(alert.message, "You will have to sign in again");
    assert_eq!(alert.confirm_text, "Revoke");
    assert_eq!(alert.confirm_color, SemanticColor::RED);
    assert_eq!(alert.cancel_text, "Keep it");
    assert_eq!(alert.cancel_color, SemanticColor::FOREGROUND);
}

#[test]
fn a_semantic_colour_keeps_the_name_the_theme_will_resolve() {
    assert_eq!(SemanticColor::RED.name(), "Red");
    assert_eq!(SemanticColor::FOREGROUND.name(), "Foreground");
}
