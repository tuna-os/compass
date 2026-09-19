//! The Create Extension form, read against
//! `src/server/src/builtins/developer/create-extension-view-host.cpp`.

use compass_core::create_extension::{
    DESCRIPTION_TOO_SHORT, Errors, FORM_HAS_ERRORS, Form, GENERATION_FAILED, LOCATION_MISSING,
    MIN_DESCRIPTION, MIN_SHORT, SUBMIT_TITLE, SUCCESS_EMOJI, SUCCESS_TITLE, Submission, TOO_SHORT,
    expand_path, submit, success_navigation, validate,
};

fn valid() -> Form {
    Form {
        author: "ana".to_owned(),
        title: "My Extension".to_owned(),
        description: "Does something useful with things".to_owned(),
        location: "~/extensions".to_owned(),
        command_title: "Search".to_owned(),
        command_description: "Searches".to_owned(),
        template_id: "list".to_owned(),
    }
}

#[test]
fn a_filled_in_form_with_a_real_directory_generates() {
    let submission = submit(&valid(), "/home/ana", true);
    assert_eq!(
        submission,
        Submission::Generate {
            target: "/home/ana/extensions".to_owned(),
        }
    );
}

#[test]
fn every_field_is_checked_so_every_mistake_shows_at_once() {
    // `submit()` clears all six errors, runs all six checks with no early
    // return, and reports together. A form that stopped at the first problem
    // would make the user submit six times.
    let empty = Form::default();
    let errors = validate(&empty, false);

    assert_eq!(
        errors,
        Errors {
            author: Some(TOO_SHORT),
            title: Some(TOO_SHORT),
            description: Some(DESCRIPTION_TOO_SHORT),
            location: Some(LOCATION_MISSING),
            command_title: Some(TOO_SHORT),
            command_description: Some(TOO_SHORT),
        }
    );
    assert!(errors.any());
}

#[test]
fn the_description_is_held_to_a_longer_minimum_than_the_rest() {
    // 16 against 3: the description is what a reader sees in a list.
    assert_eq!(MIN_SHORT, 3);
    assert_eq!(MIN_DESCRIPTION, 16);

    let mut form = valid();
    form.description = "Short".to_owned();
    let errors = validate(&form, true);

    assert_eq!(errors.description, Some(DESCRIPTION_TOO_SHORT));
    assert_eq!(errors.author, None, "three characters is enough there");
    assert_eq!(DESCRIPTION_TOO_SHORT, "Min. 16 chars");
    assert_eq!(TOO_SHORT, "Min. 3 chars");
}

#[test]
fn exactly_the_minimum_is_accepted() {
    // `if (m_author.size() < 3)` -- three passes, two does not.
    let mut form = valid();
    form.author = "ana".to_owned();
    form.description = "x".repeat(MIN_DESCRIPTION);
    assert!(!validate(&form, true).any());

    form.author = "an".to_owned();
    assert_eq!(validate(&form, true).author, Some(TOO_SHORT));

    form.author = "ana".to_owned();
    form.description = "x".repeat(MIN_DESCRIPTION - 1);
    assert_eq!(
        validate(&form, true).description,
        Some(DESCRIPTION_TOO_SHORT)
    );
}

#[test]
fn a_location_that_is_not_a_directory_is_the_one_error_about_the_world() {
    // `fs::is_directory(expandPath(location))` -- everything else is about the
    // text typed; this one asks the filesystem.
    let errors = validate(&valid(), false);

    assert_eq!(errors.location, Some(LOCATION_MISSING));
    assert_eq!(LOCATION_MISSING, "Must exist");
    assert!(
        errors.author.is_none() && errors.title.is_none(),
        "and nothing else is wrong"
    );
}

#[test]
fn a_rejected_form_carries_one_toast_and_all_the_errors() {
    let submission = submit(&Form::default(), "/home/ana", false);

    match submission {
        Submission::Rejected { errors, toast } => {
            assert_eq!(toast, FORM_HAS_ERRORS);
            assert_eq!(FORM_HAS_ERRORS, "Form has errors");
            assert!(errors.any());
        }
        other => panic!("expected a rejection: {other:?}"),
    }
}

#[test]
fn a_tilde_is_expanded_and_only_at_the_front() {
    // `expandPath`: `~` alone, or `~/...`. Anything else is literal.
    assert_eq!(expand_path("~", "/home/ana"), "/home/ana");
    assert_eq!(expand_path("~/code", "/home/ana"), "/home/ana/code");
    assert_eq!(expand_path("/tmp/code", "/home/ana"), "/tmp/code");
    assert_eq!(
        expand_path("~root/code", "/home/ana"),
        "~root/code",
        "another user's home is not expanded, so the check will fail on it"
    );
    assert_eq!(
        expand_path("/tmp/~/code", "/home/ana"),
        "/tmp/~/code",
        "and a tilde in the middle is just a character"
    );
}

#[test]
fn the_success_view_replaces_the_form_rather_than_stacking_on_it() {
    // `popSelf()` then `pushView(successView)` -- escaping from the success
    // view goes back to where the form was opened from, not to a filled-in
    // form.
    let navigation = success_navigation();

    assert!(navigation.pop_self);
    assert_eq!(navigation.title, SUCCESS_TITLE);
    assert_eq!(navigation.emoji, SUCCESS_EMOJI);
    assert_eq!(SUCCESS_TITLE, "Extension created!");
    assert_eq!(SUCCESS_EMOJI, "🥳");
}

#[test]
fn the_visible_strings_are_the_ones_the_cpp_uses() {
    assert_eq!(SUBMIT_TITLE, "Create extension");
    assert_eq!(GENERATION_FAILED, "Failed to create extension");
}

#[test]
fn a_form_can_fail_the_filesystem_check_alone_and_be_otherwise_perfect() {
    // The common case: everything typed correctly, the directory misspelt.
    let form = valid();
    let errors = validate(&form, false);

    assert_eq!(
        errors,
        Errors {
            location: Some(LOCATION_MISSING),
            ..Errors::default()
        }
    );
}
