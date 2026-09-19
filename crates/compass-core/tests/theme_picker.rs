//! The theme picker, read against `theme-list-model.cpp` and
//! `theme-view-host.cpp` (`src/server/src/builtins/theme/`).

use compass_core::theme_picker::{
    AVAILABLE_SECTION, COPY_ID_TITLE, COPY_NAME_KEYBIND, COPY_PATH_KEYBIND, COPY_PATH_TITLE,
    CURRENT_SECTION, DEFAULT_DESCRIPTION, OPEN_FILE_TITLE, OPEN_KEYBIND, PALETTE_ORDER,
    PLACEHOLDER, SET_THEME_TITLE, Theme, ThemeAction, action_panel, leaving, selection, split,
};

fn theme(id: &str, name: &str) -> Theme {
    Theme {
        id: id.to_owned(),
        name: name.to_owned(),
        description: String::new(),
        icon: None,
        path: Some(format!("/themes/{id}.json")),
    }
}

fn ids(themes: &[Theme]) -> Vec<&str> {
    themes.iter().map(|theme| theme.id.as_str()).collect()
}

#[test]
fn the_configured_theme_sits_in_its_own_section() {
    let themes = [
        theme("dracula", "Dracula"),
        theme("nord", "Nord"),
        theme("solarized", "Solarized"),
    ];
    let list = split(&themes, "", "nord");

    assert_eq!(ids(&list.current), ["nord"]);
    assert_eq!(ids(&list.available), ["dracula", "solarized"]);
    assert_eq!(CURRENT_SECTION, "Current Theme");
    assert_eq!(AVAILABLE_SECTION, "Available Themes");
}

#[test]
fn an_empty_query_keeps_the_services_own_order() {
    // `if (!query.empty()) std::ranges::stable_sort(...)` -- with no query
    // nothing is sorted, so whatever order the theme service returned shows
    // through.
    let themes = [
        theme("zebra", "Zebra"),
        theme("apple", "Apple"),
        theme("mango", "Mango"),
    ];
    let list = split(&themes, "", "none-of-them");

    assert_eq!(ids(&list.available), ["zebra", "apple", "mango"]);
    assert!(list.current.is_empty());
}

#[test]
fn a_query_filters_and_orders_by_score() {
    // The name weighs 1.0 and the description 0.5, so a theme that only
    // matches in its description ranks below one that matches in its name --
    // and a theme that matches neither is dropped.
    let mut by_description = theme("aurora", "Aurora");
    by_description.description = "nord inspired colours".to_owned();

    let themes = [
        theme("dracula", "Dracula"),
        by_description,
        theme("nord", "Nord"),
    ];
    let list = split(&themes, "nord", "none");

    assert_eq!(
        ids(&list.available),
        ["nord", "aurora"],
        "the name match first, the description match second, Dracula gone"
    );
}

#[test]
fn the_current_theme_disappears_when_it_does_not_match() {
    // The `continue` is before the current/available split, so the "Current
    // Theme" section empties rather than pinning the configured theme.
    let themes = [theme("dracula", "Dracula"), theme("nord", "Nord")];
    let list = split(&themes, "nord", "dracula");

    assert!(
        list.current.is_empty(),
        "the configured theme is filtered out like any other"
    );
    assert_eq!(ids(&list.available), ["nord"]);
}

#[test]
fn the_description_is_searchable_and_the_id_is_not() {
    // `scoreWeighted({{name, 1.0}, {desc, 0.5}}, ...)` -- the id is not a
    // field, so typing it finds nothing unless the name contains it too.
    let mut described = theme("t1", "Midnight");
    described.description = "A dark theme for late nights".to_owned();
    let themes = [described];

    assert_eq!(
        ids(&split(&themes, "late nights", "none").available),
        ["t1"]
    );
    assert!(
        split(&themes, "t1", "none").available.is_empty(),
        "the id is not searchable"
    );
}

#[test]
fn a_theme_without_a_description_shows_the_fallback_string() {
    // `desc.isEmpty() ? tr("Default theme description") : desc` -- the string
    // itself, not a placeholder for one.
    assert_eq!(theme("t", "T").subtitle(), DEFAULT_DESCRIPTION);
    assert_eq!(DEFAULT_DESCRIPTION, "Default theme description");

    let mut described = theme("t", "T");
    described.description = "Mine".to_owned();
    assert_eq!(described.subtitle(), "Mine");
}

#[test]
fn the_palette_swatches_are_in_the_rows_own_order() {
    // `customData` maps PaletteColor0..7 to these, which is what the row draws
    // left to right -- not the palette's own order, and not alphabetical.
    assert_eq!(
        PALETTE_ORDER,
        [
            "Red",
            "Blue",
            "Cyan",
            "Green",
            "Magenta",
            "Orange",
            "Foreground",
            "TextMuted"
        ]
    );
}

#[test]
fn every_theme_can_be_set_and_have_its_id_copied() {
    let actions = action_panel(&Theme::default(), false);

    assert_eq!(
        actions,
        [
            ThemeAction {
                title: SET_THEME_TITLE,
                keybind: None,
            },
            ThemeAction {
                title: COPY_ID_TITLE,
                keybind: Some(COPY_NAME_KEYBIND),
            },
        ]
    );
    assert_eq!(SET_THEME_TITLE, "Set theme");
}

#[test]
fn opening_the_file_needs_both_a_file_and_an_editor() {
    let from_file = theme("nord", "Nord");
    let built_in = Theme {
        path: None,
        ..theme("default", "Default")
    };

    let titles = |theme: &Theme, editor: bool| -> Vec<&'static str> {
        action_panel(theme, editor)
            .into_iter()
            .map(|action| action.title)
            .collect()
    };

    assert_eq!(
        titles(&from_file, true),
        [
            SET_THEME_TITLE,
            OPEN_FILE_TITLE,
            COPY_ID_TITLE,
            COPY_PATH_TITLE
        ]
    );
    assert_eq!(
        titles(&from_file, false),
        [SET_THEME_TITLE, COPY_ID_TITLE, COPY_PATH_TITLE],
        "no editor, no open action -- but the path can still be copied"
    );
    assert_eq!(
        titles(&built_in, true),
        [SET_THEME_TITLE, COPY_ID_TITLE],
        "a theme with no file has neither"
    );
}

#[test]
fn the_three_actions_carry_their_keybinds() {
    let actions = action_panel(&theme("nord", "Nord"), true);
    let keybind = |title: &str| {
        actions
            .iter()
            .find(|action| action.title == title)
            .and_then(|action| action.keybind)
    };

    assert_eq!(keybind(OPEN_FILE_TITLE), Some(OPEN_KEYBIND));
    assert_eq!(keybind(COPY_ID_TITLE), Some(COPY_NAME_KEYBIND));
    assert_eq!(keybind(COPY_PATH_TITLE), Some(COPY_PATH_KEYBIND));
    assert_eq!(OPEN_KEYBIND, "action.open");
    assert_eq!(COPY_NAME_KEYBIND, "action.copy-name");
    assert_eq!(COPY_PATH_KEYBIND, "action.copy-path");
}

#[test]
fn selecting_previews_and_leaving_undoes_it() {
    // `setOnThemeSelected(... setTheme(theme->id()))` applies immediately, and
    // `beforePop` puts the configured theme back. Browsing the list is a live
    // preview; escaping is the undo.
    let previewed = theme("dracula", "Dracula");

    assert_eq!(selection(&previewed), "dracula");
    assert_eq!(leaving("nord"), "nord");
    assert_eq!(PLACEHOLDER, "Search for a theme...");
}
