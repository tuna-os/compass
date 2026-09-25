//! Choosing a window-manager backend, and remembering what had focus.
//!
//! Ported from `src/server/src/services/window-manager/window-manager.cpp`.

use compass_core::window_manager::{
    DUMMY_PROVIDER_ID, FocusMemoryUpdate, FocusedWindow, LINUX_PROVIDER_ORDER, Window,
    choose_provider, find_app_windows, find_window_by_id, focus_memory_update,
    focused_foreign_window, focused_window, is_capable, is_on_active_workspace, is_own_window,
    remembered_window_still_open, remembered_window_survives, workspaces_need_fetch,
};

const OWN_PID: u32 = 4242;
const APP_ID: &str = "compass";

fn window(id: &str) -> Window {
    Window {
        id: id.to_owned(),
        title: "Inbox".to_owned(),
        wm_class: "org.gnome.Geary".to_owned(),
        pid: Some(999),
        workspace: Some("2".to_owned()),
    }
}

fn own_window() -> Window {
    Window {
        pid: Some(OWN_PID),
        wm_class: APP_ID.to_owned(),
        ..window("own")
    }
}

// --- choosing a provider ------------------------------------------------

#[test]
fn the_first_activatable_provider_wins() {
    // Two activatable candidates, because with one the first and the last are
    // the same and the test would pass against either rule. On a GNOME
    // session the generic Wayland provider is activatable too, so this is the
    // ordinary case rather than a contrived one.
    assert_eq!(
        choose_provider(&[("hyprland", false), ("gnome", true), ("wayland", true)]),
        "gnome"
    );
}

#[test]
fn nothing_activatable_falls_back_to_the_dummy() {
    // A real provider rather than an absence, so every caller has something
    // to call and none of them checks for null.
    assert_eq!(choose_provider(&[("gnome", false)]), DUMMY_PROVIDER_ID);
    assert_eq!(choose_provider(&[]), DUMMY_PROVIDER_ID);
}

#[test]
fn the_generic_wayland_provider_is_asked_last() {
    // It is good enough for most standalone compositors and would claim
    // Hyprland and Niri too if asked first — and then neither would get its
    // own workspace support.
    assert_eq!(LINUX_PROVIDER_ORDER.last(), Some(&"wayland"));
}

#[test]
fn the_specific_compositors_come_before_it() {
    let wayland = LINUX_PROVIDER_ORDER
        .iter()
        .position(|p| *p == "wayland")
        .expect("wayland is in the list");
    for specific in ["hyprland", "gnome", "kde", "niri"] {
        let at = LINUX_PROVIDER_ORDER
            .iter()
            .position(|p| *p == specific)
            .unwrap_or_else(|| panic!("{specific} is in the list"));
        assert!(at < wayland, "{specific} must be offered before wayland");
    }
}

#[test]
fn the_dummy_provider_means_window_management_does_nothing() {
    assert!(!is_capable(DUMMY_PROVIDER_ID));
    assert!(is_capable("gnome"));
}

// --- recognising our own window -----------------------------------------

#[test]
fn a_window_with_our_pid_is_ours() {
    assert!(is_own_window(&own_window(), OWN_PID, APP_ID));
}

#[test]
fn the_pid_is_conclusive_even_when_the_class_matches() {
    // A window with our class but somebody else's pid is somebody else's.
    let impostor = Window {
        pid: Some(1),
        wm_class: APP_ID.to_owned(),
        ..window("x")
    };
    assert!(!is_own_window(&impostor, OWN_PID, APP_ID));
}

#[test]
fn the_class_is_only_a_fallback() {
    let no_pid = Window {
        pid: None,
        wm_class: APP_ID.to_owned(),
        ..window("x")
    };
    assert!(is_own_window(&no_pid, OWN_PID, APP_ID));
}

#[test]
fn the_class_fallback_ignores_case() {
    // The class a toolkit sets does not always match the application id in
    // case.
    let no_pid = Window {
        pid: None,
        wm_class: "Compass".to_owned(),
        ..window("x")
    };
    assert!(is_own_window(&no_pid, OWN_PID, APP_ID));
}

#[test]
fn someone_elses_window_is_not_ours() {
    assert!(!is_own_window(&window("x"), OWN_PID, APP_ID));
}

// --- which workspace a window is on -------------------------------------

#[test]
fn a_window_on_the_active_workspace_is_on_it() {
    assert!(is_on_active_workspace(&window("x"), true, Some("2")));
}

#[test]
fn a_window_on_another_workspace_is_not() {
    assert!(!is_on_active_workspace(&window("x"), true, Some("3")));
}

#[test]
fn a_compositor_with_no_workspaces_answers_yes() {
    assert!(is_on_active_workspace(&window("x"), false, None));
}

#[test]
fn a_window_with_no_workspace_answers_yes() {
    let w = Window {
        workspace: None,
        ..window("x")
    };
    assert!(is_on_active_workspace(&w, true, Some("3")));
}

#[test]
fn an_empty_workspace_id_answers_yes_too() {
    // Some compositors report an empty string rather than omitting the field.
    let w = Window {
        workspace: Some(String::new()),
        ..window("x")
    };
    assert!(is_on_active_workspace(&w, true, Some("3")));
}

#[test]
fn no_active_workspace_answers_yes() {
    assert!(is_on_active_workspace(&window("x"), true, None));
}

#[test]
fn every_unknown_answers_yes_on_purpose() {
    // Callers use this to decide whether to act on a window. Saying yes risks
    // acting on a window the user cannot see; saying no guarantees doing
    // nothing at all on a compositor that reports partial data.
    for (has_workspaces, active) in [(false, None), (true, None)] {
        assert!(is_on_active_workspace(&window("x"), has_workspaces, active));
    }
}

// --- which window an action acts on -------------------------------------

#[test]
fn a_compositor_that_knows_the_frontmost_window_is_trusted_outright() {
    // It knows what is in front even while the launcher has keyboard focus,
    // so none of the remembering is needed.
    assert_eq!(
        focused_window(true, None, OWN_PID, APP_ID, true),
        FocusedWindow::FromProvider
    );
}

#[test]
fn a_foreign_focused_window_is_used_as_given() {
    assert_eq!(
        focused_window(false, Some(&window("x")), OWN_PID, APP_ID, true),
        FocusedWindow::FromProvider
    );
}

#[test]
fn our_own_focused_window_falls_back_to_the_memory() {
    // Which it will be, because the launcher is open. This is the case the
    // memory exists for.
    assert_eq!(
        focused_window(false, Some(&own_window()), OWN_PID, APP_ID, true),
        FocusedWindow::Remembered
    );
}

#[test]
fn nothing_focused_uses_the_memory_only_while_the_launcher_has_focus() {
    assert_eq!(
        focused_window(false, None, OWN_PID, APP_ID, true),
        FocusedWindow::Remembered
    );
}

#[test]
fn nothing_focused_anywhere_answers_nothing() {
    // The memory may be stale.
    assert_eq!(
        focused_window(false, None, OWN_PID, APP_ID, false),
        FocusedWindow::None
    );
}

#[test]
fn the_foreign_focused_window_never_falls_back_to_the_memory() {
    // It answers what is focused *now*, and our own window is not an answer.
    assert_eq!(focused_foreign_window(None, OWN_PID, APP_ID), None);
    assert_eq!(
        focused_foreign_window(Some(&own_window()), OWN_PID, APP_ID),
        None
    );
    assert_eq!(
        focused_foreign_window(Some(&window("x")), OWN_PID, APP_ID),
        Some(window("x"))
    );
}

// --- keeping the memory -------------------------------------------------

#[test]
fn a_frontmost_capable_provider_records_nothing() {
    assert_eq!(
        focus_memory_update(true, Some(&window("x")), OWN_PID, APP_ID, false),
        FocusMemoryUpdate::Keep
    );
}

#[test]
fn a_foreign_window_is_remembered() {
    assert_eq!(
        focus_memory_update(false, Some(&window("x")), OWN_PID, APP_ID, false),
        FocusMemoryUpdate::Remember
    );
}

#[test]
fn our_own_window_is_not_remembered_and_does_not_clear_the_memory() {
    // The whole point is that opening the launcher should not lose what was
    // underneath it.
    assert_eq!(
        focus_memory_update(false, Some(&own_window()), OWN_PID, APP_ID, true),
        FocusMemoryUpdate::Keep
    );
}

#[test]
fn nothing_focused_while_the_launcher_has_focus_keeps_the_memory() {
    assert_eq!(
        focus_memory_update(false, None, OWN_PID, APP_ID, true),
        FocusMemoryUpdate::Keep
    );
}

#[test]
fn nothing_focused_anywhere_forgets() {
    assert_eq!(
        focus_memory_update(false, None, OWN_PID, APP_ID, false),
        FocusMemoryUpdate::Forget
    );
}

#[test]
fn a_remembered_window_on_another_workspace_is_dropped() {
    // Focusing it would take the user somewhere they did not ask to go, which
    // is worse than doing nothing.
    assert!(!remembered_window_survives(&window("x"), true, Some("3")));
    assert!(remembered_window_survives(&window("x"), true, Some("2")));
}

#[test]
fn a_remembered_window_that_has_closed_is_dropped() {
    let open = vec!["a".to_owned(), "b".to_owned()];
    assert!(remembered_window_still_open("a", &open));
    assert!(!remembered_window_still_open("gone", &open));
}

#[test]
fn the_check_is_by_id_not_by_identity() {
    // Which is what catches a window the compositor destroyed and replaced.
    assert!(!remembered_window_still_open("a", &[]));
}

// --- finding windows ----------------------------------------------------

#[test]
fn an_applications_windows_are_found_by_class() {
    let windows = vec![window("a"), window("b")];
    let found = find_app_windows(&windows, "Something Else", |c| c == "org.gnome.Geary");
    assert_eq!(found.len(), 2);
}

#[test]
fn a_window_whose_title_is_the_application_name_is_found_too() {
    // A fallback for applications whose windows carry no usable class.
    let windows = vec![Window {
        wm_class: String::new(),
        title: "Mail".to_owned(),
        ..window("a")
    }];
    let found = find_app_windows(&windows, "Mail", |_| false);
    assert_eq!(found.len(), 1);
}

#[test]
fn the_title_fallback_ignores_case() {
    let windows = vec![Window {
        wm_class: String::new(),
        title: "mail".to_owned(),
        ..window("a")
    }];
    assert_eq!(find_app_windows(&windows, "Mail", |_| false).len(), 1);
}

#[test]
fn the_title_fallback_is_an_equality_and_not_a_substring() {
    // It is loose enough already: a document window titled after its file
    // will not match, which is the trade the C++ makes.
    let windows = vec![Window {
        wm_class: String::new(),
        title: "Mail - Inbox".to_owned(),
        ..window("a")
    }];
    assert!(find_app_windows(&windows, "Mail", |_| false).is_empty());
}

#[test]
fn a_window_matching_neither_is_not_found() {
    let windows = vec![window("a")];
    assert!(find_app_windows(&windows, "Mail", |_| false).is_empty());
}

#[test]
fn a_window_is_found_by_the_compositors_handle() {
    let windows = vec![window("a"), window("b")];
    assert_eq!(
        find_window_by_id(&windows, "b").map(|w| w.id.as_str()),
        Some("b")
    );
    assert_eq!(find_window_by_id(&windows, "c"), None);
}

// --- the workspace cache ------------------------------------------------

#[test]
fn the_workspace_list_is_fetched_once_and_cached() {
    assert!(workspaces_need_fetch(false));
    assert!(!workspaces_need_fetch(true));
}

#[test]
fn a_provider_with_no_workspaces_caches_the_absence() {
    // An empty list is cached rather than nothing, so the provider is not
    // asked again on every lookup.
    assert!(!workspaces_need_fetch(true));
}
