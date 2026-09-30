//! The two "pick a default" views, read against
//! `set-default-browser-view-host.hpp` and `set-default-terminal-view-host.hpp`
//! (`src/server/src/builtins/system/`).

use compass_core::default_app::{
    BROWSER_ACTION, BROWSER_FAILURE, BROWSER_PLACEHOLDER, BROWSER_PROBE_URL, BROWSER_SECTION,
    BROWSER_SUCCESS, PickerApp, TERMINAL_ACTION, TERMINAL_FAILURE, TERMINAL_PLACEHOLDER,
    TERMINAL_SECTION, TERMINAL_SUCCESS, browser_picker, terminal_picker,
};

fn app(id: &str, name: &str) -> PickerApp {
    PickerApp {
        id: id.to_owned(),
        display_name: name.to_owned(),
        description: String::new(),
        displayable: true,
        terminal_emulator: false,
    }
}

fn terminal(id: &str, name: &str) -> PickerApp {
    PickerApp {
        terminal_emulator: true,
        ..app(id, name)
    }
}

fn ids(items: &[compass_core::default_app::PickerItem]) -> Vec<&str> {
    items.iter().map(|item| item.app.id.as_str()).collect()
}

#[test]
fn the_browser_list_is_whatever_opens_an_https_url() {
    // `appDb->findOpeners(PROBE_URL)`; the C++ probes `https://vicinae.com`.
    // The list is not "things that look like browsers": an application
    // registered for https shows up whether or not it is one.
    assert_eq!(BROWSER_PROBE_URL, "https://tunaos.org/compass");

    let openers = [
        app("firefox.desktop", "Firefox"),
        app("org.gnome.Epiphany.desktop", "Web"),
    ];
    let picker = browser_picker(&openers, None);

    assert_eq!(
        ids(&picker.items),
        ["firefox.desktop", "org.gnome.Epiphany.desktop"]
    );
}

#[test]
fn a_hidden_application_is_not_offered_as_a_browser() {
    // `views::filter([](auto &&app) { return app->displayable(); })`.
    let openers = [
        PickerApp {
            displayable: false,
            ..app("hidden-handler.desktop", "Hidden Handler")
        },
        app("firefox.desktop", "Firefox"),
    ];

    assert_eq!(
        ids(&browser_picker(&openers, None).items),
        ["firefox.desktop"]
    );
}

#[test]
fn the_current_default_browser_comes_first_and_is_marked() {
    // `stable_sort(browsers, [&](a, b) { return isDefault(a) > isDefault(b); })`
    // plus the green check accessory on the matching id.
    let openers = [
        app("a.desktop", "A"),
        app("b.desktop", "B"),
        app("c.desktop", "C"),
    ];
    let picker = browser_picker(&openers, Some("c.desktop"));

    assert_eq!(ids(&picker.items), ["c.desktop", "a.desktop", "b.desktop"]);
    assert!(picker.items[0].is_default);
    assert!(
        picker.items[1..].iter().all(|item| !item.is_default),
        "only one row is marked"
    );
}

#[test]
fn the_rest_keep_the_order_the_database_gave_them() {
    // The comparator says nothing about two non-defaults, so only the sort's
    // stability keeps them in order. The browser view is stable in the C++;
    // the terminal view is not, and this port makes both so — see PARITY.md.
    let openers: Vec<PickerApp> = (0..8)
        .map(|i| app(&format!("app{i}.desktop"), &format!("App {i}")))
        .collect();

    let picker = browser_picker(&openers, Some("app5.desktop"));
    let expected: Vec<String> = std::iter::once("app5.desktop".to_owned())
        .chain(
            (0..8)
                .filter(|i| *i != 5)
                .map(|i| format!("app{i}.desktop")),
        )
        .collect();

    assert_eq!(ids(&picker.items), expected);
}

#[test]
fn a_default_that_is_not_in_the_list_marks_nothing() {
    // `webBrowser()` can name an application that no longer handles https, or
    // is no longer installed.
    let openers = [app("firefox.desktop", "Firefox")];
    let picker = browser_picker(&openers, Some("chromium.desktop"));

    assert_eq!(ids(&picker.items), ["firefox.desktop"]);
    assert!(!picker.items[0].is_default);
}

#[test]
fn the_terminal_list_is_filtered_by_the_entrys_own_flag() {
    // `filter([](auto &&app) { return app->displayable() && app->isTerminalEmulator(); })`
    // over the *whole* application list -- no MIME lookup here.
    let apps = [
        app("firefox.desktop", "Firefox"),
        terminal("org.gnome.Console.desktop", "Console"),
        PickerApp {
            displayable: false,
            ..terminal("hidden-term.desktop", "Hidden Terminal")
        },
        terminal("alacritty.desktop", "Alacritty"),
    ];

    let picker = terminal_picker(&apps, None);
    assert_eq!(
        ids(&picker.items),
        ["org.gnome.Console.desktop", "alacritty.desktop"]
    );
}

#[test]
fn the_current_default_terminal_comes_first_too() {
    let apps = [
        terminal("a.desktop", "A"),
        terminal("b.desktop", "B"),
        terminal("c.desktop", "C"),
    ];
    let picker = terminal_picker(&apps, Some("b.desktop"));

    assert_eq!(ids(&picker.items), ["b.desktop", "a.desktop", "c.desktop"]);
    assert!(picker.items[0].is_default);
}

#[test]
fn each_picker_carries_its_own_words() {
    // The two views are near-copies, and every visible string differs. A
    // shared implementation that reused one set would be wrong in a way only a
    // screenshot would catch.
    let browser = browser_picker(&[], None);
    assert_eq!(browser.placeholder, BROWSER_PLACEHOLDER);
    assert_eq!(browser.section, BROWSER_SECTION);
    assert_eq!(browser.action, BROWSER_ACTION);
    assert_eq!(browser.success, BROWSER_SUCCESS);
    assert_eq!(browser.failure, BROWSER_FAILURE);

    let term = terminal_picker(&[], None);
    assert_eq!(term.placeholder, TERMINAL_PLACEHOLDER);
    assert_eq!(term.section, TERMINAL_SECTION);
    assert_eq!(term.action, TERMINAL_ACTION);
    assert_eq!(term.success, TERMINAL_SUCCESS);
    assert_eq!(term.failure, TERMINAL_FAILURE);

    assert_eq!(BROWSER_PLACEHOLDER, "Select a web browser...");
    assert_eq!(TERMINAL_PLACEHOLDER, "Select a terminal emulator...");
    assert_eq!(BROWSER_SUCCESS, "Default browser changed");
    assert_eq!(TERMINAL_SUCCESS, "Default terminal changed");
    assert_eq!(BROWSER_FAILURE, "Failed to set default browser");
    assert_eq!(TERMINAL_FAILURE, "Failed to set default terminal");
}

#[test]
fn an_empty_list_is_a_picker_with_no_rows_rather_than_an_error() {
    // A machine with no terminal emulator installed still opens the view.
    assert!(
        terminal_picker(&[app("firefox.desktop", "Firefox")], None)
            .items
            .is_empty()
    );
    assert!(
        browser_picker(&[], Some("firefox.desktop"))
            .items
            .is_empty()
    );
}
