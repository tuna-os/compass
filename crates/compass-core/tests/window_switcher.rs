//! The window and workspace switchers.
//!
//! Ported from `src/server/src/builtins/wm/`.

use compass_core::window_switcher::{
    CLOSE_WINDOW_SHORTCUT, Capabilities, SWITCH_WORKSPACES_KEYWORDS, WindowEntry, WorkspaceEntry,
    registered_commands, switch_to_workspace_label, switch_workspaces_name, window_accessory,
    window_action_panel, window_icon, window_search_fields, window_subtitle, window_title,
    workspace_accessories, workspace_apps, workspace_name_worth_showing, workspace_search_fields,
    workspace_search_placeholder, workspace_subtitle,
};

fn everything() -> Capabilities {
    Capabilities {
        workspaces: true,
        fullscreen: true,
        toggle_floating: true,
        toggle_overview: true,
        set_sticky: true,
        move_to_workspace: true,
    }
}

fn window() -> WindowEntry {
    WindowEntry {
        title: "Inbox".to_owned(),
        wm_class: "org.gnome.Geary".to_owned(),
        ..WindowEntry::default()
    }
}

fn with_app(mut entry: WindowEntry) -> WindowEntry {
    entry.app_name = Some("Mail".to_owned());
    entry.app_icon = Some("geary.png".to_owned());
    entry
}

// --- which commands exist -----------------------------------------------

#[test]
fn switching_windows_is_always_offered() {
    // Every compositor the launcher talks to can list and focus windows.
    assert_eq!(
        registered_commands(Capabilities::default()),
        ["switch-windows"]
    );
}

#[test]
fn a_compositor_with_everything_gets_every_command() {
    assert_eq!(
        registered_commands(everything()),
        [
            "switch-windows",
            "switch-workspaces",
            "toggle-fullscreen",
            "toggle-floating",
            "toggle-overview"
        ]
    );
}

#[test]
fn each_command_is_gated_on_its_own_capability() {
    // A command certain to fail is worse than a command that is absent.
    let caps = Capabilities {
        fullscreen: true,
        ..Capabilities::default()
    };
    assert_eq!(
        registered_commands(caps),
        ["switch-windows", "toggle-fullscreen"]
    );
}

#[test]
fn workspace_switching_needs_workspaces() {
    let caps = Capabilities {
        workspaces: false,
        toggle_overview: true,
        ..Capabilities::default()
    };
    assert!(!registered_commands(caps).contains(&"switch-workspaces"));
}

#[test]
fn sticky_support_registers_no_command_of_its_own() {
    // The C++ tests the capability and registers nothing in the branch. The
    // capability is used — pinning appears in a window's action panel — so the
    // empty branch is a note about an intended command, not dead weight.
    let caps = Capabilities {
        set_sticky: true,
        ..Capabilities::default()
    };
    assert_eq!(registered_commands(caps), ["switch-windows"]);
}

#[test]
fn windows_calls_them_desktops() {
    assert_eq!(switch_workspaces_name(true), "Switch Desktops");
    assert_eq!(switch_workspaces_name(false), "Switch Workspaces");
}

#[test]
fn both_words_are_searchable_whichever_one_is_shown() {
    // Someone typing the word their own desktop uses finds the command.
    assert!(SWITCH_WORKSPACES_KEYWORDS.contains(&"workspaces"));
    assert!(SWITCH_WORKSPACES_KEYWORDS.contains(&"desktops"));
    assert!(SWITCH_WORKSPACES_KEYWORDS.contains(&"virtual desktops"));
    assert!(SWITCH_WORKSPACES_KEYWORDS.contains(&"spaces"));
}

// --- a window's row -----------------------------------------------------

#[test]
fn the_window_title_is_the_row_title() {
    assert_eq!(window_title(&window()), "Inbox");
}

#[test]
fn a_recognised_window_is_subtitled_with_the_application_name() {
    assert_eq!(window_subtitle(&with_app(window())), "Mail");
}

#[test]
fn an_unrecognised_window_falls_back_to_its_wm_class() {
    // Not pretty, but it is the only thing that tells two unrecognised
    // windows apart.
    assert_eq!(window_subtitle(&window()), "org.gnome.Geary");
}

#[test]
fn a_recognised_window_uses_the_applications_icon() {
    assert_eq!(window_icon(&with_app(window())), "geary.png");
}

#[test]
fn an_unrecognised_window_gets_the_generic_window_glyph() {
    assert_eq!(window_icon(&window()), "app-window");
}

#[test]
fn a_named_workspace_shows_its_name() {
    let entry = WindowEntry {
        workspace_name: "Mail".to_owned(),
        workspace_id: Some("3".to_owned()),
        ..window()
    };
    assert_eq!(window_accessory(&entry).as_deref(), Some("Mail"));
}

#[test]
fn a_numbered_workspace_shows_its_number_with_a_label() {
    let entry = WindowEntry {
        workspace_id: Some("3".to_owned()),
        ..window()
    };
    assert_eq!(window_accessory(&entry).as_deref(), Some("WS 3"));
}

#[test]
fn a_window_on_no_workspace_shows_nothing() {
    // An absent accessory is not the same as "WS " with nothing after it.
    assert_eq!(window_accessory(&window()), None);
}

#[test]
fn a_workspace_named_after_its_own_number_is_treated_as_unnamed() {
    // A compositor with no names reports the id as the name. Showing that
    // would put a bare `3` where a name belongs and lose the `WS` that says
    // what the number means.
    assert!(!workspace_name_worth_showing("3", "3"));
    assert!(workspace_name_worth_showing("Mail", "3"));
}

// --- searching windows --------------------------------------------------

#[test]
fn the_title_carries_the_most_weight() {
    // It is what changes between two windows of the same application.
    let entry = with_app(window());
    let fields = window_search_fields(&entry);
    assert_eq!(fields[0], ("Inbox", 1.0));
}

#[test]
fn the_wm_class_stays_searchable_even_when_an_application_was_recognised() {
    // Someone who knows a window as `org.gnome.Geary` should still find it
    // when the desktop entry calls it Mail.
    let entry = with_app(window());
    let fields = window_search_fields(&entry);
    assert_eq!(
        fields,
        [("Inbox", 1.0), ("Mail", 0.5), ("org.gnome.Geary", 0.3)]
    );
}

#[test]
fn an_unrecognised_window_is_searched_on_two_fields() {
    let entry = window();
    let fields = window_search_fields(&entry);
    assert_eq!(fields, [("Inbox", 1.0), ("org.gnome.Geary", 0.3)]);
}

// --- a window's actions -------------------------------------------------

#[test]
fn focusing_comes_first_and_closing_last() {
    let panel = window_action_panel(&window(), Capabilities::default());
    assert_eq!(panel[0], ["focus", "close"]);
}

#[test]
fn pinning_appears_only_where_the_compositor_can_pin() {
    let caps = Capabilities {
        set_sticky: true,
        ..Capabilities::default()
    };
    assert_eq!(
        window_action_panel(&window(), caps)[0],
        ["focus", "pin", "close"]
    );
}

#[test]
fn moving_to_a_workspace_appears_only_where_it_is_supported() {
    let caps = Capabilities {
        move_to_workspace: true,
        ..Capabilities::default()
    };
    assert_eq!(
        window_action_panel(&window(), caps)[0],
        ["focus", "bring-to-workspace", "close"]
    );
}

#[test]
fn quitting_is_offered_only_for_a_recognised_window() {
    // Quitting belongs to the application. There is nothing to quit when all
    // that is known is a `WM_CLASS`.
    assert_eq!(window_action_panel(&window(), everything()).len(), 1);
    let panel = window_action_panel(&with_app(window()), everything());
    assert_eq!(panel.len(), 2);
    assert_eq!(panel[1], ["quit-app", "force-quit-app"]);
}

#[test]
fn closing_a_window_has_a_shortcut() {
    assert_eq!(CLOSE_WINDOW_SHORTCUT, "ctrl+q");
}

// --- a workspace's row --------------------------------------------------

fn workspace(count: usize) -> WorkspaceEntry {
    WorkspaceEntry {
        name: "Mail".to_owned(),
        window_count: count,
        ..WorkspaceEntry::default()
    }
}

#[test]
fn an_empty_workspace_says_empty_rather_than_counting_to_zero() {
    // `empty` reads as a state; `0 windows` reads as a measurement.
    assert_eq!(workspace_subtitle(&workspace(0)), "empty");
}

#[test]
fn one_window_is_singular_and_several_are_plural() {
    // A declared divergence. The C++ writes `tr("%n window(s)", "", n)`, and
    // Qt only applies plural forms where a translation supplies them. There is
    // no English entry, so the shipped English reads `3 window(s)` — the
    // translator's placeholder, in the interface. This is what every
    // translated locale already does.
    assert_eq!(workspace_subtitle(&workspace(1)), "1 window");
    assert_eq!(workspace_subtitle(&workspace(3)), "3 windows");
}

#[test]
fn the_screen_is_appended_when_one_matched() {
    let entry = WorkspaceEntry {
        screen_name: Some("DP-1".to_owned()),
        ..workspace(2)
    };
    assert_eq!(workspace_subtitle(&entry), "2 windows - DP-1");
}

#[test]
fn an_empty_workspace_on_a_known_screen_still_names_the_screen() {
    let entry = WorkspaceEntry {
        screen_name: Some("DP-1".to_owned()),
        ..workspace(0)
    };
    assert_eq!(workspace_subtitle(&entry), "empty - DP-1");
}

#[test]
fn a_workspace_on_no_matched_screen_says_nothing_about_screens() {
    assert_eq!(workspace_subtitle(&workspace(2)), "2 windows");
}

// --- the applications on a workspace ------------------------------------

#[test]
fn each_application_appears_once_however_many_windows_it_has() {
    // Three terminals show one icon, and the count still says three.
    let apps = workspace_apps(&[
        Some("Terminal".to_owned()),
        Some("Terminal".to_owned()),
        Some("Terminal".to_owned()),
    ]);
    assert_eq!(apps, ["Terminal"]);
}

#[test]
fn applications_keep_the_order_their_windows_were_listed_in() {
    // The names are deliberately out of alphabetical order: with `Files`
    // before `Mail` a sort and an insertion order look identical, and the test
    // would pass against either.
    let apps = workspace_apps(&[
        Some("Mail".to_owned()),
        Some("Files".to_owned()),
        Some("Mail".to_owned()),
    ]);
    assert_eq!(apps, ["Mail", "Files"]);
}

#[test]
fn an_unrecognised_window_contributes_no_icon() {
    let apps = workspace_apps(&[None, Some("Mail".to_owned()), None]);
    assert_eq!(apps, ["Mail"]);
}

#[test]
fn the_accessories_are_the_deduplicated_applications() {
    let entry = WorkspaceEntry {
        app_names: vec!["Mail".to_owned(), "Files".to_owned()],
        ..workspace(5)
    };
    assert_eq!(workspace_accessories(&entry), ["Mail", "Files"]);
}

// --- searching workspaces -----------------------------------------------

#[test]
fn a_workspace_is_searched_by_name_screen_and_applications() {
    // Searching the applications is what lets someone find "the workspace with
    // the browser on it" without knowing what it is called.
    let entry = WorkspaceEntry {
        screen_name: Some("DP-1".to_owned()),
        app_names: vec!["Firefox".to_owned()],
        ..workspace(1)
    };
    assert_eq!(
        workspace_search_fields(&entry),
        [("Mail", 1.0), ("DP-1", 0.8), ("Firefox", 0.3)]
    );
}

#[test]
fn a_workspace_with_no_screen_is_searched_on_its_name_alone() {
    let entry = workspace(0);
    assert_eq!(workspace_search_fields(&entry), [("Mail", 1.0)]);
}

#[test]
fn every_application_on_a_workspace_is_searchable() {
    let entry = WorkspaceEntry {
        app_names: vec!["Firefox".to_owned(), "Mail".to_owned()],
        ..workspace(2)
    };
    let fields = workspace_search_fields(&entry);
    assert_eq!(fields.len(), 3);
    assert_eq!(fields[2], ("Mail", 0.3));
}

// --- the workspace row's action and placeholder -------------------------

#[test]
fn the_switch_action_follows_the_platforms_word() {
    assert_eq!(switch_to_workspace_label(true), "Switch to desktop");
    assert_eq!(switch_to_workspace_label(false), "Switch to workspace");
}

#[test]
fn the_placeholder_follows_the_platforms_word_too() {
    assert_eq!(workspace_search_placeholder(true), "Search desktops...");
    assert_eq!(workspace_search_placeholder(false), "Search workspaces...");
}
