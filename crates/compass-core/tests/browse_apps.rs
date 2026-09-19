//! The "Search Applications" builtin, read against
//! `src/server/src/builtins/system/browse-apps-model.{hpp,cpp}`.

use compass_core::browse_apps::{
    ActionKind, BrowseApp, COPY_ID_TITLE, COPY_LOCATION_TITLE, DesktopAction, HIDDEN_ACCESSORY,
    NUMBERED_ACTION_LIMIT, OPEN_KEYBIND, OPEN_KEYBIND_DEFAULT, OPEN_TITLE, SECTION_TITLE, Section,
    Shortcut, accessories, action_panel, score,
};
use compass_search::Query;

fn app(name: &str) -> BrowseApp {
    BrowseApp {
        id: format!("{}.desktop", name.to_lowercase()),
        display_name: name.to_owned(),
        description: String::new(),
        keywords: Vec::new(),
        path: format!("/usr/share/applications/{}.desktop", name.to_lowercase()),
        displayable: true,
        actions: Vec::new(),
    }
}

fn kinds(panel: &compass_core::browse_apps::ActionPanel) -> Vec<&ActionKind> {
    panel.actions.iter().map(|action| &action.kind).collect()
}

#[test]
fn the_title_matches_on_the_name_the_description_and_the_keywords() {
    // `fields.push_back({name, 1.0}); {desc, 0.5}; {kw, 0.3}`.
    let mut editor = app("Text Editor");
    editor.description = "Edit plain text files".to_owned();
    editor.keywords = vec!["gedit".to_owned()];

    for query in ["text editor", "plain text", "gedit"] {
        assert!(
            score(&editor, &Query::new(query)).accepted(),
            "{query} should match"
        );
    }
    assert!(!score(&editor, &Query::new("spreadsheet")).accepted());
}

#[test]
fn a_keyword_weighs_less_here_than_a_name_and_less_than_in_the_root_list() {
    // 0.3 here against `root_items`' 0.6 -- this list only competes with other
    // applications, so a keyword hit should not outrank a name hit.
    let mut by_name = app("Processes");
    by_name.description = "System monitor".to_owned();

    let mut by_keyword = app("System Monitor");
    by_keyword.keywords = vec!["processes".to_owned()];

    let query = Query::new("processes");
    assert!(
        score(&by_name, &query).weighted > score(&by_keyword, &query).weighted,
        "name {:?} should outweigh keyword {:?}",
        score(&by_name, &query),
        score(&by_keyword, &query)
    );

    // And the same pair, scored the way the root list scores it, is closer:
    // the weights are a real difference, not an accident of these strings.
    let root_keyword_weight = {
        use compass_search::{WeightedField, score_weighted};
        score_weighted(
            &[
                WeightedField::new(&by_keyword.display_name, 1.0),
                WeightedField::new("", 0.5),
                WeightedField::new(&by_keyword.keywords[0], 0.6),
            ],
            &query,
        )
    };
    assert!(
        root_keyword_weight.weighted > score(&by_keyword, &query).weighted,
        "0.6 must score the same keyword higher than 0.3"
    );
}

#[test]
fn a_hidden_application_says_so_and_a_visible_one_says_nothing() {
    // `if (!app->displayable()) return {{.text = tr("Hidden")}};`
    let mut hidden = app("Daemon");
    hidden.displayable = false;

    assert_eq!(accessories(&hidden), [HIDDEN_ACCESSORY]);
    assert_eq!(HIDDEN_ACCESSORY, "Hidden");
    assert!(accessories(&app("Firefox")).is_empty());
}

#[test]
fn the_section_heading_leaves_the_count_to_the_translator() {
    // `tr("Applications ({count})")` -- the placeholder is part of the string
    // a translator sees, so it is not formatted away here.
    assert_eq!(SECTION_TITLE, "Applications ({count})");
}

#[test]
fn the_panel_is_titled_after_the_application() {
    let panel = action_panel(&app("Firefox"), &[], false);
    assert_eq!(panel.title, "Firefox");
}

#[test]
fn opening_comes_first_and_clears_the_search() {
    // `open->setClearSearch(true)` -- launching something ends the search.
    let panel = action_panel(&app("Firefox"), &[], false);

    assert_eq!(panel.actions[0].kind, ActionKind::OpenApp);
    assert_eq!(panel.actions[0].title, OPEN_TITLE);
    assert!(panel.actions[0].clear_search);
    assert_eq!(panel.actions[0].section, Section::Main);
    assert!(
        panel.actions[1..].iter().all(|action| !action.clear_search),
        "and nothing else does"
    );
}

#[test]
fn an_open_window_puts_focus_above_open_and_only_the_first_one() {
    // `if (!activeWindows.empty()) { addAction(new FocusWindowAction(activeWindows.front())); }`
    let panel = action_panel(
        &app("Firefox"),
        &["win-1".to_owned(), "win-2".to_owned()],
        false,
    );

    assert_eq!(
        panel.actions[0].kind,
        ActionKind::FocusWindow {
            window: "win-1".to_owned()
        }
    );
    assert_eq!(panel.actions[1].kind, ActionKind::OpenApp);
    assert!(
        !kinds(&panel).iter().any(|kind| matches!(
            kind,
            ActionKind::FocusWindow { window } if window == "win-2"
        )),
        "the second window gets no action"
    );
}

#[test]
fn the_first_nine_desktop_actions_get_numbered_shortcuts_and_the_tenth_does_not() {
    // `if (i < 9) action->setShortcut(QString("control+shift+%1").arg(i + 1));`
    let mut firefox = app("Firefox");
    firefox.actions = (0..11)
        .map(|i| DesktopAction {
            id: format!("action-{i}"),
            display_name: format!("Action {i}"),
        })
        .collect();

    let panel = action_panel(&firefox, &[], false);
    let desktop: Vec<_> = panel
        .actions
        .iter()
        .filter(|action| matches!(action.kind, ActionKind::OpenDesktopAction { .. }))
        .collect();

    assert_eq!(desktop.len(), 11);
    assert_eq!(
        desktop[0].shortcut,
        Some(Shortcut::Literal("control+shift+1".to_owned())),
        "one-based"
    );
    assert_eq!(
        desktop[8].shortcut,
        Some(Shortcut::Literal("control+shift+9".to_owned()))
    );
    assert_eq!(desktop[9].shortcut, None, "the tenth gets nothing");
    assert_eq!(desktop[10].shortcut, None);
    assert_eq!(NUMBERED_ACTION_LIMIT, 9);
}

#[test]
fn desktop_actions_keep_their_entry_order_and_their_own_names() {
    let mut firefox = app("Firefox");
    firefox.actions = vec![
        DesktopAction {
            id: "new-window".to_owned(),
            display_name: "New Window".to_owned(),
        },
        DesktopAction {
            id: "new-private-window".to_owned(),
            display_name: "New Private Window".to_owned(),
        },
    ];

    let panel = action_panel(&firefox, &[], false);
    let titles: Vec<&str> = panel
        .actions
        .iter()
        .filter(|action| matches!(action.kind, ActionKind::OpenDesktopAction { .. }))
        .map(|action| action.title.as_str())
        .collect();

    assert_eq!(titles, ["New Window", "New Private Window"]);
}

#[test]
fn open_location_appears_only_when_something_can_open_it() {
    // `if (auto opener = appDb->provider()->locationOpener(*app))`.
    let without = action_panel(&app("Firefox"), &[], false);
    assert!(!kinds(&without).contains(&&ActionKind::OpenLocation));

    let with = action_panel(&app("Firefox"), &[], true);
    let location = with
        .actions
        .iter()
        .find(|action| action.kind == ActionKind::OpenLocation)
        .expect("the action is there");

    assert_eq!(location.section, Section::Utils);
    assert_eq!(
        location.shortcut,
        Some(Shortcut::Keybind(OPEN_KEYBIND)),
        "the named keybind, not a literal chord: the user can rebind it"
    );
    assert_eq!(OPEN_KEYBIND, "action.open");
    assert_eq!(OPEN_KEYBIND_DEFAULT, "control+o");
}

#[test]
fn the_two_copy_actions_are_last_and_in_the_utils_section() {
    let panel = action_panel(&app("Firefox"), &[], true);
    let tail: Vec<&ActionKind> = kinds(&panel).into_iter().rev().take(2).rev().collect();

    assert_eq!(tail, [&ActionKind::CopyAppId, &ActionKind::CopyAppLocation]);
    for action in panel.actions.iter().rev().take(2) {
        assert_eq!(action.section, Section::Utils);
        assert_eq!(action.shortcut, None);
    }
    assert_eq!(COPY_ID_TITLE, "Copy App ID");
    assert_eq!(COPY_LOCATION_TITLE, "Copy App Location");
}

#[test]
fn the_main_section_comes_before_the_utils_section() {
    let mut firefox = app("Firefox");
    firefox.actions = vec![DesktopAction {
        id: "new-window".to_owned(),
        display_name: "New Window".to_owned(),
    }];

    let panel = action_panel(&firefox, &["win-1".to_owned()], true);
    let sections: Vec<Section> = panel.actions.iter().map(|action| action.section).collect();
    let first_util = sections
        .iter()
        .position(|section| *section == Section::Utils)
        .expect("there are utils");

    assert!(
        sections[..first_util]
            .iter()
            .all(|section| *section == Section::Main),
        "{sections:?}"
    );
    assert!(
        sections[first_util..]
            .iter()
            .all(|section| *section == Section::Utils),
        "{sections:?}"
    );
}
