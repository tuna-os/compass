//! The quicklink form, read against
//! `src/server/src/builtins/shortcut/shortcut-form-view-host.cpp`.

use compass_core::shortcut_form::{
    CREATE_FAILED, CREATED, DEFAULT_APP, DEFAULT_ICON, DefaultIcon, Errors, Existing,
    LINK_COMPLETIONS, Mode, REQUIRED, SUBMIT_TITLE, Submission, UPDATE_FAILED, UPDATED,
    VALIDATION_FAILED, default_icon, initial, submit, validate,
};

fn existing() -> Existing {
    Existing {
        id: "sct-abc".to_owned(),
        name: "Search".to_owned(),
        url: "https://example.org/?q={query}".to_owned(),
        app: "firefox.desktop".to_owned(),
        icon: "globe".to_owned(),
    }
}

#[test]
fn creating_starts_empty_with_both_defaults_selected() {
    let form = initial(Mode::Create, None, false, false);

    assert_eq!(form.name, "");
    assert_eq!(form.link, "");
    assert_eq!(form.app, DEFAULT_APP);
    assert_eq!(form.icon, DEFAULT_ICON);
    assert_eq!(form.navigation_title, "", "no title when creating");
}

#[test]
fn duplicating_prefixes_the_name_and_editing_does_not() {
    // `tr("Copy of %1")` in the Duplicate arm only.
    assert_eq!(
        initial(Mode::Duplicate, Some(&existing()), true, true).name,
        "Copy of Search"
    );
    assert_eq!(
        initial(Mode::Edit, Some(&existing()), true, true).name,
        "Search"
    );
}

#[test]
fn the_navigation_title_quotes_the_name() {
    assert_eq!(
        initial(Mode::Edit, Some(&existing()), true, true).navigation_title,
        "Edit \"Search\""
    );
    assert_eq!(
        initial(Mode::Duplicate, Some(&existing()), true, true).navigation_title,
        "Duplicate \"Search\""
    );
}

#[test]
fn an_uninstalled_application_reverts_to_the_default() {
    // `!isDefaultApp() && appDb->findById(appId)` -- both conditions, so a
    // quicklink pointing at an app that is gone opens the form on "default"
    // rather than showing a stale name.
    assert_eq!(
        initial(Mode::Edit, Some(&existing()), true, true).app,
        "firefox.desktop"
    );
    assert_eq!(
        initial(Mode::Edit, Some(&existing()), false, true).app,
        DEFAULT_APP,
        "the app is not installed any more"
    );

    let already_default = Existing {
        app: DEFAULT_APP.to_owned(),
        ..existing()
    };
    assert_eq!(
        initial(Mode::Edit, Some(&already_default), true, true).app,
        DEFAULT_APP
    );
}

#[test]
fn an_unknown_icon_reverts_to_the_default() {
    // `if (auto item = itemDataById(icon); !item.isEmpty())` -- an icon id
    // that is not in the list leaves the default selected.
    assert_eq!(
        initial(Mode::Edit, Some(&existing()), true, true).icon,
        "globe"
    );
    assert_eq!(
        initial(Mode::Edit, Some(&existing()), true, false).icon,
        DEFAULT_ICON
    );
}

#[test]
fn the_link_the_app_and_the_icon_are_required_and_the_name_is_not() {
    // Three checks, and no check on `m_name`: a quicklink with no name is
    // allowed, and the list shows an empty title for it. Reproduced rather
    // than tidied, because demanding a name would reject quicklinks that
    // already exist.
    assert_eq!(
        validate("", "", ""),
        Errors {
            link: Some(REQUIRED),
            app: Some(REQUIRED),
            icon: Some(REQUIRED),
        }
    );
    assert!(!validate("https://e.org", DEFAULT_APP, DEFAULT_ICON).any());
    assert_eq!(REQUIRED, "Required");

    let submission = submit(
        Mode::Create,
        None,
        "",
        "https://e.org",
        DEFAULT_APP,
        DEFAULT_ICON,
        "builtin:link",
    );
    assert!(
        matches!(submission, Submission::Create { ref name, .. } if name.is_empty()),
        "an unnamed quicklink is saved: {submission:?}"
    );
}

#[test]
fn a_rejected_form_says_validation_failed() {
    let submission = submit(Mode::Create, None, "x", "", DEFAULT_APP, DEFAULT_ICON, "i");

    match submission {
        Submission::Rejected { errors, toast } => {
            assert_eq!(toast, VALIDATION_FAILED);
            assert_eq!(VALIDATION_FAILED, "Validation failed");
            assert_eq!(errors.link, Some(REQUIRED));
            assert_eq!(errors.app, None);
        }
        other => panic!("expected a rejection: {other:?}"),
    }
}

#[test]
fn editing_updates_and_duplicating_creates() {
    // The C++ branches on `Mode::Edit` alone, so Duplicate takes the create
    // path -- which is the whole point of duplicating.
    let edit = submit(
        Mode::Edit,
        Some(&existing()),
        "Search",
        "https://e.org",
        DEFAULT_APP,
        "globe",
        "builtin:link",
    );
    assert!(
        matches!(edit, Submission::Update { ref id, .. } if id == "sct-abc"),
        "{edit:?}"
    );

    let duplicate = submit(
        Mode::Duplicate,
        Some(&existing()),
        "Copy of Search",
        "https://e.org",
        DEFAULT_APP,
        "globe",
        "builtin:link",
    );
    assert!(
        matches!(duplicate, Submission::Create { .. }),
        "{duplicate:?}"
    );
}

#[test]
fn the_toasts_differ_between_creating_and_updating() {
    let Submission::Update {
        success, failure, ..
    } = submit(
        Mode::Edit,
        Some(&existing()),
        "n",
        "l",
        "a",
        "i",
        "builtin:link",
    )
    else {
        panic!("expected an update");
    };
    assert_eq!((success, failure), (UPDATED, UPDATE_FAILED));

    let Submission::Create {
        success, failure, ..
    } = submit(Mode::Create, None, "n", "l", "a", "i", "builtin:link")
    else {
        panic!("expected a create");
    };
    assert_eq!((success, failure), (CREATED, CREATE_FAILED));

    assert_eq!(UPDATED, "Shortcut updated");
    assert_eq!(CREATED, "Shortcut created");
    assert_eq!(UPDATE_FAILED, "Failed to update shortcut");
    assert_eq!(CREATE_FAILED, "Failed to create shortcut");
    assert_eq!(SUBMIT_TITLE, "Submit");
}

#[test]
fn the_default_icon_is_stored_as_what_it_resolved_to() {
    // `if (iconId == "default") iconId = m_resolvedDefaultIcon;` -- the stored
    // icon is the favicon or app icon the user was shown, not the word
    // "default", so it does not change under them later.
    let Submission::Create { icon, .. } = submit(
        Mode::Create,
        None,
        "n",
        "https://example.org",
        DEFAULT_APP,
        DEFAULT_ICON,
        "favicon:example.org",
    ) else {
        panic!("expected a create");
    };
    assert_eq!(icon, "favicon:example.org");

    let Submission::Create { icon, .. } = submit(
        Mode::Create,
        None,
        "n",
        "https://example.org",
        DEFAULT_APP,
        "globe",
        "favicon:example.org",
    ) else {
        panic!("expected a create");
    };
    assert_eq!(icon, "globe", "a chosen icon is stored as chosen");
}

#[test]
fn a_web_link_prefers_its_favicon_over_the_openers_icon() {
    // The favicon request is fired after the opener lookup and overwrites the
    // default icon when it lands, so for an http(s) link the favicon wins.
    assert_eq!(
        default_icon(Some("firefox-icon"), "https", "example.org"),
        DefaultIcon::Favicon {
            host: "example.org".to_owned()
        }
    );
    assert_eq!(
        default_icon(Some("gimp-icon"), "file", ""),
        DefaultIcon::Opener {
            icon: "gimp-icon".to_owned()
        }
    );
    assert_eq!(default_icon(None, "file", ""), DefaultIcon::BuiltinLink);
}

#[test]
fn the_scheme_test_is_a_prefix_and_takes_in_more_than_http_and_https() {
    // `url.scheme().startsWith("http")` -- so `httpx://` would also trigger a
    // favicon lookup. Harmless, and reproduced rather than tightened.
    assert!(matches!(
        default_icon(None, "http", "example.org"),
        DefaultIcon::Favicon { .. }
    ));
    assert!(matches!(
        default_icon(None, "httpx", "example.org"),
        DefaultIcon::Favicon { .. }
    ));
    assert!(!matches!(
        default_icon(None, "ftp", "example.org"),
        DefaultIcon::Favicon { .. }
    ));
}

#[test]
fn the_link_field_offers_three_completions_and_one_of_them_positions_the_cursor() {
    // `{argument name=""}` with `cursorOffset` 16 -- between the quotes, where
    // the user types the argument's name.
    let titles: Vec<&str> = LINK_COMPLETIONS
        .iter()
        .map(|completion| completion.title)
        .collect();
    assert_eq!(titles, ["Selected Text", "Clipboard Text", "Argument"]);

    let argument = LINK_COMPLETIONS[2];
    assert_eq!(argument.template, Some("{argument name=\"\"}"));
    assert_eq!(argument.cursor_offset, Some(16));
    assert_eq!(
        &argument.template.unwrap()[..argument.cursor_offset.unwrap()],
        "{argument name=\"",
        "the cursor lands inside the quotes"
    );
    assert!(LINK_COMPLETIONS[0].template.is_none());
}
