//! The snippet form, read against
//! `src/server/src/builtins/snippet/snippet-form-view-host.cpp` and
//! `snippet::Expansion::validateKeyword`.

use compass_core::snippet_form::{
    CONTENT_EMPTY, CREATED, Errors, Expansion, KEYWORD_EMPTY, KEYWORD_NOT_PRINTABLE,
    KEYWORD_TOO_LONG, MAX_KEYWORD, MIN_NAME, NAME_TOO_SHORT, Submission, TOO_MANY_CURSORS, UPDATED,
    VALIDATION_FAILED, submit, validate, validate_keyword,
};

#[test]
fn a_name_needs_two_characters() {
    assert_eq!(validate("a", "text", 0, "").name, Some(NAME_TOO_SHORT));
    assert_eq!(validate("ab", "text", 0, "").name, None);
    assert_eq!(MIN_NAME, 2);
    assert_eq!(NAME_TOO_SHORT, "2 chars min.");
}

#[test]
fn the_content_must_not_be_empty() {
    assert_eq!(validate("name", "", 0, "").content, Some(CONTENT_EMPTY));
    assert_eq!(CONTENT_EMPTY, "Content should not be empty");
}

#[test]
fn one_cursor_placeholder_is_allowed_and_two_are_not() {
    // `count_if(placeholders, id == "cursor") > 1` -- one is the point of the
    // feature, two would be ambiguous.
    assert_eq!(validate("name", "a{cursor}b", 1, "").content, None);
    assert_eq!(
        validate("name", "a{cursor}b{cursor}", 2, "").content,
        Some(TOO_MANY_CURSORS)
    );
    assert_eq!(TOO_MANY_CURSORS, "Only one {cursor} placeholder is allowed");
}

#[test]
fn the_cursor_count_is_only_consulted_when_there_is_content() {
    // The check is in the `else` of the emptiness test, so empty content
    // reports emptiness and not a cursor problem.
    assert_eq!(validate("name", "", 5, "").content, Some(CONTENT_EMPTY));
}

#[test]
fn an_empty_keyword_means_no_expansion_rather_than_an_error() {
    // `if (!m_keyword.isEmpty())` guards the whole check: a snippet without a
    // keyword is a snippet you paste rather than type.
    assert_eq!(validate("name", "text", 0, "").keyword, None);

    let submission = submit(None, "name", "text", 0, "", false, &[]);
    assert!(
        matches!(
            submission,
            Submission::Save {
                expansion: None,
                ..
            }
        ),
        "{submission:?}"
    );
}

#[test]
fn a_keyword_must_be_printable_ascii_without_spaces() {
    // `uc <= 127 && isprint(uc) && !isspace(uc)` -- `isprint` includes the
    // space and `isspace` takes it back out, so the rule is ASCII graphic.
    assert_eq!(validate_keyword("addr"), None);
    assert_eq!(validate_keyword("addr-1_!"), None);
    assert_eq!(
        validate_keyword("my addr"),
        Some(KEYWORD_NOT_PRINTABLE),
        "a space is rejected"
    );
    assert_eq!(validate_keyword("addr\t"), Some(KEYWORD_NOT_PRINTABLE));
    assert_eq!(
        validate_keyword("café"),
        Some(KEYWORD_NOT_PRINTABLE),
        "and so is anything outside ASCII"
    );
    assert_eq!(
        KEYWORD_NOT_PRINTABLE,
        "Keyword must only contain printable ASCII characters (no spaces)"
    );
}

#[test]
fn a_keyword_is_capped_at_thirty_two() {
    assert_eq!(validate_keyword(&"a".repeat(MAX_KEYWORD)), None);
    assert_eq!(
        validate_keyword(&"a".repeat(MAX_KEYWORD + 1)),
        Some(KEYWORD_TOO_LONG)
    );
    assert_eq!(MAX_KEYWORD, 32);
    assert_eq!(KEYWORD_TOO_LONG, "Keyword exceeds maximum length of 32");
}

#[test]
fn validate_keyword_rejects_an_empty_one_even_though_the_form_never_asks() {
    // The function guards itself; the form guards the call. Both are pinned,
    // because the function is public and another caller may not guard.
    assert_eq!(validate_keyword(""), Some(KEYWORD_EMPTY));
    assert_eq!(KEYWORD_EMPTY, "Keyword cannot be empty");
}

#[test]
fn every_field_is_checked_so_every_mistake_shows_at_once() {
    assert_eq!(
        validate("a", "", 0, "bad keyword"),
        Errors {
            name: Some(NAME_TOO_SHORT),
            content: Some(CONTENT_EMPTY),
            keyword: Some(KEYWORD_NOT_PRINTABLE),
        }
    );
}

#[test]
fn a_rejected_form_says_validation_failed() {
    let submission = submit(None, "a", "text", 0, "", false, &[]);

    match submission {
        Submission::Rejected { errors, toast } => {
            assert_eq!(toast, VALIDATION_FAILED);
            assert_eq!(VALIDATION_FAILED, "Validation failed");
            assert_eq!(errors.name, Some(NAME_TOO_SHORT));
        }
        other => panic!("expected a rejection: {other:?}"),
    }
}

#[test]
fn a_keyword_brings_the_expansion_with_its_word_flag_and_its_apps() {
    let apps = vec!["firefox".to_owned(), "code".to_owned()];
    let submission = submit(None, "Address", "1 Main St", 0, "addr", true, &apps);

    assert!(
        matches!(
            submission,
            Submission::Save {
                expansion: Some(Expansion {
                    ref keyword,
                    word: true,
                    ref apps,
                }),
                ..
            } if keyword == "addr" && apps.len() == 2
        ),
        "{submission:?}"
    );
}

#[test]
fn the_two_success_messages_are_not_symmetrical() {
    // "Snippet updated" against "Snippet successfully created" -- copied as
    // they are, because they are translated strings.
    let Submission::Save { success, id, .. } =
        submit(Some("snp-1"), "name", "text", 0, "", false, &[])
    else {
        panic!("expected a save");
    };
    assert_eq!(success, UPDATED);
    assert_eq!(id.as_deref(), Some("snp-1"));

    let Submission::Save { success, id, .. } = submit(None, "name", "text", 0, "", false, &[])
    else {
        panic!("expected a save");
    };
    assert_eq!(success, CREATED);
    assert_eq!(id, None);

    assert_eq!(UPDATED, "Snippet updated");
    assert_eq!(CREATED, "Snippet successfully created");
}
